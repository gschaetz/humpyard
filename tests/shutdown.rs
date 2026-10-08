#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::format_push_string,
    clippy::needless_pass_by_value
)] // test scaffolding: fail loudly, favor readability
//! Graceful shutdown: drain, grace period, second signal, and entries written before `run` returns.

mod common;

use std::convert::Infallible;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use axum::Router;
use axum::response::sse::{Event, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use common::{chat_completion, chat_request, chunk, serve, usage};
use humpyard::config::Config;
use humpyard::ledger::Ledger;
use humpyard::server;
use serde_json::Value;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

async fn completions(axum::Json(body): axum::Json<Value>) -> Response {
    let model = body["model"].as_str().unwrap().to_string();
    if body["stream"] == true {
        // Sends one chunk, then stalls far longer than any test waits.
        let events = async_stream::stream! {
            yield Ok::<_, Infallible>(Event::default().data(
                chunk(serde_json::json!({"role": "assistant", "content": "hi"}), None).to_string()));
            tokio::time::sleep(Duration::from_secs(60)).await;
        };
        return Sse::new(events).into_response();
    }
    if model == "delay-m" {
        tokio::time::sleep(Duration::from_millis(900)).await;
    }
    axum::Json(chat_completion(&model, "ok", Some(usage(10, 5)))).into_response()
}

struct Running {
    addr: SocketAddr,
    signals: mpsc::Sender<()>,
    task: JoinHandle<Result<(), String>>,
    db: PathBuf,
    _dir: tempfile::TempDir,
}

async fn start(grace_secs: u64) -> Running {
    let upstream = serve(Router::new().route("/chat/completions", post(completions))).await;
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("ledger.db");
    let toml = format!(
        r#"listen = "127.0.0.1:0"
shutdown_grace_secs = {grace_secs}
[ledger]
path = "{}"
[providers.mock]
base_url = "http://{upstream}"
api_key_env = "K"
max_retries = 0
timeout_secs = 120
[targets.fast]
endpoints = [{{ provider = "mock", model = "fast-m", price = {{ input = 1.0, output = 2.0 }} }}]
[targets.delay]
endpoints = [{{ provider = "mock", model = "delay-m", price = {{ input = 1.0, output = 2.0 }} }}]
[targets.hang]
endpoints = [{{ provider = "mock", model = "hang-m", price = {{ input = 1.0, output = 2.0 }} }}]
"#,
        db.display()
    );
    let config = Config::from_toml(&toml, |_| Some("key".into())).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (signals, rx) = mpsc::channel(4);
    let task = tokio::spawn(server::run(config, listener, rx));
    // Wait until it answers.
    for _ in 0..100 {
        if reqwest::get(format!("http://{addr}/healthz")).await.is_ok() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    Running {
        addr,
        signals,
        task,
        db,
        _dir: dir,
    }
}

impl Running {
    async fn post(&self, model: &str, stream: bool) -> reqwest::Response {
        reqwest::Client::new()
            .post(format!("http://{}/v1/chat/completions", self.addr))
            .json(&chat_request(model, stream))
            .send()
            .await
            .unwrap()
    }
}

#[tokio::test]
async fn an_in_flight_request_finishes_and_new_connections_are_refused() {
    let server = start(30).await;
    let url = format!("http://{}/v1/chat/completions", server.addr);
    let in_flight = tokio::spawn(async move {
        reqwest::Client::new()
            .post(url)
            .json(&chat_request("delay", false))
            .send()
            .await
            .unwrap()
            .status()
    });
    tokio::time::sleep(Duration::from_millis(300)).await;
    server.signals.send(()).await.unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;

    let late = reqwest::Client::new()
        .post(format!("http://{}/v1/chat/completions", server.addr))
        .timeout(Duration::from_secs(2))
        .json(&chat_request("fast", false))
        .send()
        .await;
    assert!(
        late.is_err(),
        "a connection made after the signal must be refused"
    );

    assert_eq!(
        in_flight.await.unwrap(),
        200,
        "the request already running completes"
    );
    let result = tokio::time::timeout(Duration::from_secs(5), server.task)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(result, Ok(()));
}

#[tokio::test]
async fn entries_are_in_the_ledger_the_moment_run_returns() {
    let server = start(30).await;
    for _ in 0..5 {
        assert_eq!(server.post("fast", false).await.status(), 200);
    }
    // No waiting for the background writer: the signal arrives right after the last response.
    server.signals.send(()).await.unwrap();
    let db = server.db.clone();
    let result = tokio::time::timeout(Duration::from_secs(5), server.task)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(result, Ok(()));
    let entries = Ledger::open(&db)
        .await
        .unwrap()
        .entries_since(0)
        .await
        .unwrap();
    assert_eq!(entries.len(), 5, "{entries:?}");
}

#[tokio::test]
async fn the_grace_period_cuts_a_long_stream_and_records_it_as_cancelled() {
    let server = start(1).await;
    let mut stream = server.post("hang", true).await;
    stream.chunk().await.unwrap().unwrap(); // the first chunk arrives; the rest never will
    let started = Instant::now();
    server.signals.send(()).await.unwrap();

    let mut text = String::new();
    while let Ok(Some(chunk)) = tokio::time::timeout(Duration::from_secs(8), stream.chunk())
        .await
        .unwrap_or(Ok(None))
    {
        text.push_str(&String::from_utf8_lossy(&chunk));
    }
    assert!(
        text.contains("shutting down"),
        "the stream should end with an error event: {text:?}"
    );
    assert!(
        started.elapsed() < Duration::from_secs(6),
        "cut at the grace period, not the 60s stall"
    );

    let result = tokio::time::timeout(Duration::from_secs(8), server.task)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(result, Ok(()));
    let entries = Ledger::open(&server.db)
        .await
        .unwrap()
        .entries_since(0)
        .await
        .unwrap();
    assert_eq!(entries.len(), 1, "{entries:?}");
    assert_eq!(entries[0].outcome, "cancelled");
}

#[tokio::test]
async fn a_second_signal_cancels_at_once_instead_of_waiting_out_the_grace_period() {
    let server = start(30).await; // a 30s grace period that the test must not wait for
    let mut stream = server.post("hang", true).await;
    stream.chunk().await.unwrap().unwrap();
    let started = Instant::now();
    server.signals.send(()).await.unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    server.signals.send(()).await.unwrap();

    let result = tokio::time::timeout(Duration::from_secs(8), server.task)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(result, Ok(()));
    assert!(
        started.elapsed() < Duration::from_secs(6),
        "took {:?}",
        started.elapsed()
    );
    let entries = Ledger::open(&server.db)
        .await
        .unwrap()
        .entries_since(0)
        .await
        .unwrap();
    assert_eq!(entries.len(), 1, "{entries:?}");
    assert_eq!(entries[0].outcome, "cancelled");
}

/// Holds SQLite's write lock so the ledger writer cannot make progress, then releases it.
/// Only a flush that really waits for the writer can make `run` outlast the lock.
#[tokio::test]
async fn run_waits_for_a_stalled_ledger_writer_before_returning() {
    use sqlx::{Connection, Executor};

    let server = start(30).await;
    let mut lock = sqlx::SqliteConnection::connect(&format!("sqlite://{}", server.db.display()))
        .await
        .unwrap();
    lock.execute("BEGIN IMMEDIATE").await.unwrap();

    for _ in 0..3 {
        assert_eq!(server.post("fast", false).await.status(), 200);
    }
    let started = Instant::now();
    server.signals.send(()).await.unwrap();
    let release = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(600)).await;
        lock.execute("COMMIT").await.unwrap();
    });

    let db = server.db.clone();
    let result = tokio::time::timeout(Duration::from_secs(8), server.task)
        .await
        .unwrap()
        .unwrap();
    let waited = started.elapsed();
    assert_eq!(result, Ok(()));
    assert!(
        waited >= Duration::from_millis(500),
        "run returned after only {waited:?}: it did not wait for the writer"
    );
    let entries = Ledger::open(&db)
        .await
        .unwrap()
        .entries_since(0)
        .await
        .unwrap();
    assert_eq!(entries.len(), 3, "{entries:?}");
    release.await.unwrap();
}
