#![cfg(unix)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test scaffolding: fail loudly
//! The real binary, a real SIGHUP: an edited config file is picked up without a restart, and a
//! broken one is rejected while the process keeps serving.

mod common;

use std::process::{Command, Stdio};
use std::time::Duration;

use axum::Router;
use axum::response::IntoResponse;
use axum::routing::post;
use common::{chat_completion, chat_request, serve, usage};
use serde_json::Value;

fn config(port: u16, upstream: std::net::SocketAddr, extra: &str) -> String {
    format!(
        r#"listen = "127.0.0.1:{port}"
[providers.mock]
base_url = "http://{upstream}"
api_key_env = "HUMPYARD_TEST_KEY"
max_retries = 0
[targets.fast]
endpoints = [{{ provider = "mock", model = "fast-m" }}]
[routes.plain]
type = "passthrough"
targets = ["fast"]
{extra}
"#
    )
}

async fn health(port: u16) -> Value {
    reqwest::get(format!("http://127.0.0.1:{port}/v1/health"))
        .await
        .unwrap()
        .json()
        .await
        .unwrap()
}

async fn wait_for_attempts(port: u16, attempts: u64) -> Value {
    for _ in 0..100 {
        let report = health(port).await;
        if report["reload"]["attempts"].as_u64().unwrap_or(0) >= attempts {
            return report;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("the reload never finished");
}

#[tokio::test]
async fn sighup_reloads_the_config_file_of_the_running_binary() {
    let upstream = serve(Router::new().route(
        "/chat/completions",
        post(|| async {
            axum::Json(chat_completion("m", "ok", Some(usage(10, 5)))).into_response()
        }),
    ))
    .await;
    let dir = tempfile::tempdir().unwrap();
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let path = dir.path().join("humpyard.toml");
    std::fs::write(&path, config(port, upstream, "")).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_humpyard"))
        .args(["serve", "--config", path.to_str().unwrap()])
        .env("HUMPYARD_TEST_KEY", "test-key")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    for _ in 0..200 {
        if reqwest::get(format!("http://127.0.0.1:{port}/healthz"))
            .await
            .is_ok()
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let before = health(port).await["config"]["fingerprint"].clone();
    let pid = child.id().to_string();
    let hup = || {
        let status = Command::new("kill").args(["-HUP", &pid]).status().unwrap();
        assert!(status.success());
    };

    // A new route appears after the file is edited and the process is signalled.
    let ask = |model: &'static str| async move {
        reqwest::Client::new()
            .post(format!("http://127.0.0.1:{port}/v1/chat/completions"))
            .json(&chat_request(model, false))
            .send()
            .await
            .unwrap()
            .status()
            .as_u16()
    };
    assert_eq!(ask("extra").await, 404);
    std::fs::write(
        &path,
        config(
            port,
            upstream,
            "[routes.extra]\ntype = \"passthrough\"\ntargets = [\"fast\"]",
        ),
    )
    .unwrap();
    hup();
    let report = wait_for_attempts(port, 1).await;
    assert_eq!(report["reload"]["ok"], true, "{report}");
    assert_ne!(report["config"]["fingerprint"], before);
    assert_eq!(ask("extra").await, 200);

    // A broken file is rejected and the process keeps serving what it has.
    std::fs::write(&path, "not [valid").unwrap();
    hup();
    let report = wait_for_attempts(port, 2).await;
    assert_eq!(report["reload"]["ok"], false, "{report}");
    assert_eq!(ask("extra").await, 200);
    assert!(child.try_wait().unwrap().is_none(), "still running");
    let _ = child.kill();
}
