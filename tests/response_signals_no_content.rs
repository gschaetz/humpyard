//! No answer content reaches storage (spec: `response-signals`, change `record-response-quality-signals`,
//! ADR 0030): a call whose answer text, tool name and tool-call arguments hold a recognizable string is recorded
//! and flagged, and the ledger file, both decision endpoints, both statistics endpoints, the metrics exposition and
//! the captured logs hold the flag and none of that string. Streamed and buffered answers are both checked.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test scaffolding

mod common;

use std::convert::Infallible;
use std::time::Duration;

use axum::Router;
use axum::extract::Json;
use axum::response::sse::{Event, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use common::{chunk, logs, serve, usage};
use humpyard::config::Config;
use humpyard::ledger::Ledger;
use humpyard::server;
use serde_json::{Value, json};

/// The recognizable string; it appears in the answer text, the tool name and the (cut-off) arguments.
const MARKER: &str = "zq91-answer-marker-5c2e";

/// A key name that appears nowhere else.
const ARG_KEY: &str = "zq91_argkey_8d1f";

fn tool_name() -> String {
    format!("tool_{MARKER}")
}

/// Arguments that end in the middle of a JSON object, so the call is flagged `bad_tool_call`.
fn cut_arguments() -> String {
    format!("{{\"{ARG_KEY}\":\"{MARKER}")
}

async fn completions(Json(body): Json<Value>) -> Response {
    if body["stream"] == true {
        let events = async_stream::stream! {
            let event = |v: Value| Event::default().data(v.to_string());
            yield Ok::<_, Infallible>(event(chunk(
                json!({"role": "assistant", "content": format!("text {MARKER}")}), None)));
            yield Ok(event(chunk(
                json!({"tool_calls": [{"index": 0, "id": "call_1", "type": "function",
                       "function": {"name": tool_name(), "arguments": cut_arguments()}}]}), None)));
            yield Ok(event(chunk(json!({}), Some("tool_calls"))));
            yield Ok(Event::default().data("[DONE]"));
        };
        return Sse::new(events).into_response();
    }
    Json(
        json!({"id": "c", "object": "chat.completion", "created": 1, "model": "m",
        "choices": [{"index": 0, "finish_reason": "tool_calls", "message": {
            "role": "assistant", "content": format!("text {MARKER}"),
            "tool_calls": [{"id": "call_1", "type": "function",
                "function": {"name": tool_name(), "arguments": cut_arguments()}}]}}],
        "usage": usage(10, 5)}),
    )
    .into_response()
}

struct Rig {
    url: String,
    alice: String,
    gary: String,
    metrics: String,
    db: std::path::PathBuf,
    _dir: tempfile::TempDir,
}

async fn rig() -> Rig {
    let upstream = format!(
        "http://{}",
        serve(Router::new().route("/chat/completions", post(completions))).await
    );
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("ledger.db");
    let (alice, alice_hash) = humpyard::auth::generate_key().unwrap();
    let (gary, gary_hash) = humpyard::auth::generate_key().unwrap();
    let (metrics, metrics_hash) = humpyard::auth::generate_key().unwrap();
    let toml = format!(
        "listen = \"127.0.0.1:0\"\n[ledger]\npath = \"{}\"\n\
         [keys.alice]\nsha256 = \"{alice_hash}\"\n\
         [admin.credentials.gary]\nsha256 = \"{gary_hash}\"\nscopes = [\"read\"]\n\
         [metrics]\nenabled = true\nbearer_sha256 = \"{metrics_hash}\"\n\
         [providers.p]\nbase_url = \"{upstream}\"\napi_key_env = \"K\"\nmax_retries = 0\ntimeout_secs = 5\n\
         [targets.t]\nendpoints = [{{ provider = \"p\", model = \"m\", price = {{ input = 1.0, output = 2.0 }} }}]\n\
         [routes.r]\ntype = \"passthrough\"\ntargets = [\"t\"]\n",
        db.display()
    );
    let config = Config::from_toml(&toml, |_| Some("k".into())).unwrap();
    let url = format!(
        "http://{}",
        serve(server::router(config).await.unwrap()).await
    );
    Rig {
        url,
        alice,
        gary,
        metrics,
        db,
        _dir: dir,
    }
}

impl Rig {
    async fn get(&self, path: &str, key: &str) -> String {
        let response = reqwest::Client::new()
            .get(format!("{}{path}", self.url))
            .bearer_auth(key)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200, "{path}");
        response.text().await.unwrap()
    }

    async fn ask(&self, stream: bool) -> String {
        let response = reqwest::Client::new()
            .post(format!("{}/v1/chat/completions", self.url))
            .bearer_auth(&self.alice)
            .json(&json!({"model": "r", "stream": stream,
                          "messages": [{"role": "user", "content": "hello"}]}))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        response.text().await.unwrap()
    }
}

#[tokio::test]
async fn a_flagged_answer_leaves_the_flag_and_none_of_its_content() {
    let log = logs();
    let rig = rig().await;
    let streamed = rig.ask(true).await;
    let buffered = rig.ask(false).await;
    // Sanity: the client does receive the content; it is only storage that must not.
    assert!(streamed.contains(MARKER) && buffered.contains(MARKER));

    // Wait for the writer, then read the facts back.
    let ledger = Ledger::open(&rig.db).await.unwrap();
    let mut rows = Vec::new();
    for _ in 0..200 {
        rows = ledger.entries_since(0).await.unwrap();
        if rows.len() >= 2 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(30)).await;
    }
    assert_eq!(rows.len(), 2, "one streamed and one buffered answer");
    for row in &rows {
        let signals = row.signals.expect("an answer is judged");
        assert!(signals.bad_tool_call, "the cut-off call is flagged");
    }
    ledger.flush().await;

    let mut files = String::new();
    for suffix in ["", "-wal"] {
        if let Ok(bytes) = std::fs::read(format!("{}{suffix}", rig.db.display())) {
            files.push_str(&String::from_utf8_lossy(&bytes));
        }
    }
    let surfaces = [
        ("ledger files", files),
        (
            "/v1/decisions",
            rig.get("/v1/decisions?limit=10", &rig.alice).await,
        ),
        (
            "/admin/v1/decisions",
            rig.get("/admin/v1/decisions?limit=10", &rig.gary).await,
        ),
        (
            "/v1/stats",
            rig.get("/v1/stats?group_by=target", &rig.alice).await,
        ),
        (
            "/admin/v1/stats",
            rig.get("/admin/v1/stats?group_by=target", &rig.gary).await,
        ),
        ("/metrics", rig.get("/metrics", &rig.metrics).await),
        ("logs", log.contents()),
    ];
    for (name, text) in &surfaces {
        assert!(
            !text.is_empty(),
            "{name} is empty, so the check proves nothing"
        );
        for needle in [MARKER, ARG_KEY] {
            assert!(!text.contains(needle), "{name} holds {needle:?}");
        }
    }
    // The surfaces do record the calls, so the absence above is not an empty-handed pass.
    assert!(
        surfaces[0].1.contains("alice"),
        "the files hold the metadata"
    );
    assert!(
        surfaces[1].1.contains("\"outcome\""),
        "decisions list the calls"
    );
}
