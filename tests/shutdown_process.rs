#![cfg(unix)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test scaffolding: fail loudly
//! The real binary, real signals: after SIGTERM or SIGINT it drains, writes the ledger and exits 0.

mod common;

use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use axum::Router;
use axum::response::IntoResponse;
use axum::routing::post;
use common::{chat_completion, chat_request, serve, usage};
use humpyard::ledger::Ledger;

struct Gateway {
    child: Child,
    port: u16,
    db: std::path::PathBuf,
    _dir: tempfile::TempDir,
}

impl Drop for Gateway {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

async fn start() -> Gateway {
    let upstream = serve(Router::new().route(
        "/chat/completions",
        post(|| async {
            axum::Json(chat_completion("fast-m", "ok", Some(usage(10, 5)))).into_response()
        }),
    ))
    .await;
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("ledger.db");
    // A free port for the child process to listen on.
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let config = format!(
        r#"listen = "127.0.0.1:{port}"
shutdown_grace_secs = 5
[ledger]
path = "{}"
[providers.mock]
base_url = "http://{upstream}"
api_key_env = "HUMPYARD_TEST_KEY"
max_retries = 0
[targets.fast]
endpoints = [{{ provider = "mock", model = "fast-m", price = {{ input = 1.0, output = 2.0 }} }}]
"#,
        db.display()
    );
    let path = dir.path().join("humpyard.toml");
    std::fs::write(&path, config).unwrap();
    let child = Command::new(env!("CARGO_BIN_EXE_humpyard"))
        .args(["serve", "--config", path.to_str().unwrap()])
        .env("HUMPYARD_TEST_KEY", "test-key")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let gateway = Gateway {
        child,
        port,
        db,
        _dir: dir,
    };
    for _ in 0..200 {
        if reqwest::get(format!("http://127.0.0.1:{port}/healthz"))
            .await
            .is_ok()
        {
            return gateway;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("the gateway never started listening");
}

async fn stop_with(signal: &str) {
    let mut gateway = start().await;
    let status = reqwest::Client::new()
        .post(format!(
            "http://127.0.0.1:{}/v1/chat/completions",
            gateway.port
        ))
        .json(&chat_request("fast", false))
        .send()
        .await
        .unwrap()
        .status();
    assert_eq!(status, 200);

    let sent = Command::new("kill")
        .args([signal, &gateway.child.id().to_string()])
        .status()
        .unwrap();
    assert!(sent.success());

    let started = Instant::now();
    let exit = loop {
        if let Some(status) = gateway.child.try_wait().unwrap() {
            break status;
        }
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "the gateway did not exit after {signal}"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    };
    assert!(exit.success(), "exit status after {signal}: {exit:?}");

    // Read straight away: a clean exit means the entry was already written.
    let entries = Ledger::open(&gateway.db)
        .await
        .unwrap()
        .entries_since(0)
        .await
        .unwrap();
    assert_eq!(entries.len(), 1, "{entries:?}");
}

#[tokio::test]
async fn sigterm_drains_flushes_the_ledger_and_exits_zero() {
    stop_with("-TERM").await;
}

#[tokio::test]
async fn sigint_drains_flushes_the_ledger_and_exits_zero() {
    stop_with("-INT").await;
}
