//! An endpoint's `request_defaults` (ADR 0038): fields added to a request sent to it when the request does not carry
//! them, never replacing one it does, whichever protocol the client spoke, and only on that endpoint's attempt.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test scaffolding

mod common;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::sse::{Event, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use common::{chat_completion, chunk, serve, usage};
use humpyard::config::Config;
use humpyard::server::{self, Reload};
use serde_json::{Value, json};
use tokio::sync::mpsc;

/// What an upstream saw, and whether it fails.
struct Upstream {
    seen: Mutex<Vec<Value>>,
    fail: bool,
}

async fn completions(
    State(up): State<Arc<Upstream>>,
    axum::Json(body): axum::Json<Value>,
) -> Response {
    let stream = body["stream"] == true;
    up.seen.lock().unwrap().push(body);
    if up.fail {
        return (StatusCode::INTERNAL_SERVER_ERROR, "boom").into_response();
    }
    if stream {
        let events = async_stream::stream! {
            yield Ok::<_, std::io::Error>(Event::default().data(
                chunk(json!({"role": "assistant", "content": "hi"}), None).to_string()));
            yield Ok(Event::default().data(chunk(json!({}), Some("stop")).to_string()));
            yield Ok(Event::default().data(
                json!({"id": "c", "object": "chat.completion.chunk", "created": 1, "model": "m",
                       "choices": [], "usage": usage(10, 5)}).to_string()));
            yield Ok(Event::default().data("[DONE]"));
        };
        return Sse::new(events).into_response();
    }
    axum::Json(chat_completion("m", "ok", Some(usage(10, 5)))).into_response()
}

async fn upstream(fail: bool) -> (String, Arc<Upstream>) {
    let up = Arc::new(Upstream {
        seen: Mutex::new(vec![]),
        fail,
    });
    let app = Router::new()
        .route("/chat/completions", post(completions))
        .with_state(up.clone());
    (format!("http://{}", serve(app).await), up)
}

fn last(up: &Upstream) -> Value {
    up.seen.lock().unwrap().last().unwrap().clone()
}

/// Provider keys come from `PATH`, which every process has, so a reload (which reads the real environment) works.
fn config_text(url: &str, defaults: &str) -> String {
    format!(
        r#"listen = "127.0.0.1:0"
[providers.p]
base_url = "{url}"
api_key_env = "PATH"
max_retries = 0
[targets.dflt]
endpoints = [{{ provider = "p", model = "m"{defaults} }}]
"#
    )
}

const DEFAULTS: &str = r#", request_defaults = { reasoning_effort = "none", temperature = 0.7, top_p = 0.8, top_k = 20, presence_penalty = 1.5 }"#;

async fn gateway(text: &str) -> String {
    let config = Config::from_toml(text, |n| std::env::var(n).ok()).unwrap();
    format!(
        "http://{}",
        serve(server::router(config).await.unwrap()).await
    )
}

async fn chat(url: &str, mut body: Value) -> Value {
    body["model"] = json!("dflt");
    body["messages"] = json!([{"role": "user", "content": "hi"}]);
    let resp = reqwest::Client::new()
        .post(format!("{url}/v1/chat/completions"))
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    resp.json().await.unwrap_or(Value::Null)
}

async fn messages(url: &str, mut body: Value) {
    body["model"] = json!("dflt");
    body["max_tokens"] = json!(64);
    body["messages"] = json!([{"role": "user", "content": "hi"}]);
    let resp = reqwest::Client::new()
        .post(format!("{url}/v1/messages"))
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200, "{:?}", resp.text().await);
}

const FIELDS: [&str; 7] = [
    "reasoning_effort",
    "temperature",
    "top_p",
    "top_k",
    "min_p",
    "presence_penalty",
    "frequency_penalty",
];

#[tokio::test]
async fn a_default_reaches_the_endpoint_when_the_request_lacks_the_field() {
    let (url, up) = upstream(false).await;
    let g = gateway(&config_text(&url, DEFAULTS)).await;
    chat(&g, json!({})).await;
    let sent = last(&up);
    assert_eq!(sent["reasoning_effort"], "none", "{sent}");
    assert_eq!(sent["temperature"], 0.7, "{sent}");
    assert_eq!(sent["top_p"], 0.8, "{sent}");
    assert_eq!(sent["top_k"], 20, "{sent}");
    assert_eq!(sent["presence_penalty"], 1.5, "{sent}");
    assert_eq!(sent["model"], "m", "the rest of the request is untouched");
    assert_eq!(sent["messages"][0]["content"], "hi");
}

#[tokio::test]
async fn the_clients_own_value_wins_and_only_the_missing_fields_are_filled() {
    let (url, up) = upstream(false).await;
    let g = gateway(&config_text(&url, DEFAULTS)).await;
    chat(&g, json!({"reasoning_effort": "high", "temperature": 0.2})).await;
    let sent = last(&up);
    assert_eq!(sent["reasoning_effort"], "high", "{sent}");
    assert_eq!(sent["temperature"], 0.2, "{sent}");
    assert_eq!(
        sent["top_p"], 0.8,
        "a field the client left out is still filled"
    );
    assert_eq!(sent["top_k"], 20, "{sent}");
}

#[tokio::test]
async fn a_client_value_equal_to_zero_is_still_the_clients() {
    let (url, up) = upstream(false).await;
    let g = gateway(&config_text(&url, DEFAULTS)).await;
    chat(&g, json!({"temperature": 0, "presence_penalty": 0})).await;
    let sent = last(&up);
    assert_eq!(sent["temperature"], 0, "{sent}");
    assert_eq!(sent["presence_penalty"], 0, "{sent}");
}

#[tokio::test]
async fn an_anthropic_format_request_gets_the_defaults_and_keeps_its_own_values() {
    let (url, up) = upstream(false).await;
    let g = gateway(&config_text(&url, DEFAULTS)).await;

    messages(&g, json!({})).await;
    let sent = last(&up);
    assert_eq!(sent["temperature"], 0.7, "{sent}");
    assert_eq!(sent["top_k"], 20, "{sent}");
    assert_eq!(sent["reasoning_effort"], "none", "{sent}");

    // The client's own temperature and its effort (`output_config.effort` becomes `reasoning_effort`) win.
    messages(
        &g,
        json!({"temperature": 0.1, "output_config": {"effort": "high"}}),
    )
    .await;
    let sent = last(&up);
    assert_eq!(sent["temperature"], 0.1, "{sent}");
    assert_eq!(sent["reasoning_effort"], "high", "{sent}");
    assert_eq!(sent["top_p"], 0.8, "{sent}");
}

#[tokio::test]
async fn a_streaming_request_gets_the_defaults() {
    let (url, up) = upstream(false).await;
    let g = gateway(&config_text(&url, DEFAULTS)).await;
    let resp = reqwest::Client::new()
        .post(format!("{g}/v1/chat/completions"))
        .json(&json!({"model": "dflt", "stream": true, "messages": [{"role": "user", "content": "hi"}]}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let _ = resp.text().await.unwrap();
    let sent = last(&up);
    assert_eq!(sent["stream"], true, "{sent}");
    assert_eq!(sent["temperature"], 0.7, "{sent}");
    assert_eq!(sent["reasoning_effort"], "none", "{sent}");
}

#[tokio::test]
async fn an_endpoint_without_defaults_is_sent_none_of_the_fields() {
    let (url, up) = upstream(false).await;
    let g = gateway(&config_text(&url, "")).await;
    chat(&g, json!({})).await;
    let sent = last(&up);
    for field in FIELDS {
        assert!(sent.get(field).is_none(), "{field} in {sent}");
    }
}

#[tokio::test]
async fn a_failover_sends_each_endpoint_its_own_defaults() {
    let (first_url, first) = upstream(true).await;
    let (second_url, second) = upstream(false).await;
    let text = format!(
        r#"listen = "127.0.0.1:0"
[providers.a]
base_url = "{first_url}"
api_key_env = "PATH"
max_retries = 0
[providers.b]
base_url = "{second_url}"
api_key_env = "PATH"
max_retries = 0
[targets.dflt]
endpoints = [{{ provider = "a", model = "m", request_defaults = {{ temperature = 0.7 }} }}, {{ provider = "b", model = "m" }}]
"#
    );
    let g = gateway(&text).await;
    chat(&g, json!({})).await;
    assert_eq!(last(&first)["temperature"], 0.7);
    assert!(
        last(&second).get("temperature").is_none(),
        "the second endpoint declares none: {}",
        last(&second)
    );
}

#[tokio::test]
async fn defaults_are_not_shown_to_a_client() {
    let (url, _up) = upstream(false).await;
    let g = gateway(&config_text(&url, DEFAULTS)).await;
    let reply = chat(&g, json!({})).await;
    assert!(!reply.to_string().contains("presence_penalty"), "{reply}");
    let models: Value = reqwest::get(format!("{g}/v1/models"))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(!models.to_string().contains("request_defaults"), "{models}");
    let info = reqwest::get(format!("{g}/v1/key/info")).await.unwrap();
    let info = info.text().await.unwrap();
    assert!(
        !info.contains("request_defaults") && !info.contains("presence_penalty"),
        "{info}"
    );
}

/// The gateway's own headers on a reply, without the ones that differ on every request.
fn decision_headers(resp: &reqwest::Response) -> Vec<(String, String)> {
    let mut headers: Vec<_> = resp
        .headers()
        .iter()
        .filter(|(name, _)| {
            name.as_str().starts_with("x-humpyard-") && *name != "x-humpyard-request-id"
        })
        .map(|(name, value)| (name.to_string(), value.to_str().unwrap().to_string()))
        .collect();
    headers.sort();
    headers
}

#[tokio::test]
async fn defaults_do_not_change_which_endpoint_is_chosen_or_how_the_request_is_judged() {
    let (url, _up) = upstream(false).await;
    let with = gateway(&config_text(&url, DEFAULTS)).await;
    let without = gateway(&config_text(&url, "")).await;
    let ask = |g: String| async move {
        reqwest::Client::new()
            .post(format!("{g}/v1/chat/completions"))
            .json(&json!({"model": "dflt", "messages": [{"role": "user", "content": "hi"}]}))
            .send()
            .await
            .unwrap()
    };
    let (a, b) = (ask(with).await, ask(without).await);
    assert_eq!(a.status(), b.status());
    let (headers_a, headers_b) = (decision_headers(&a), decision_headers(&b));
    assert!(!headers_a.is_empty(), "the gateway says what it did");
    assert_eq!(headers_a, headers_b);
}

/// A gateway driven as the binary drives it, with a trigger channel standing in for SIGHUP.
struct Running {
    url: String,
    path: std::path::PathBuf,
    reload: mpsc::Sender<()>,
    stop: mpsc::Sender<()>,
    task: tokio::task::JoinHandle<()>,
    _dir: tempfile::TempDir,
}

async fn start(text: &str) -> Running {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("humpyard.toml");
    std::fs::write(&path, text).unwrap();
    let config = Config::from_toml(text, |n| std::env::var(n).ok()).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let (reload, triggers) = mpsc::channel(4);
    let (stop, signals) = mpsc::channel(4);
    let reload_path = path.clone();
    let task = tokio::spawn(async move {
        let _ = server::run(
            config,
            listener,
            signals,
            Some(Reload {
                path: reload_path,
                triggers,
            }),
        )
        .await;
    });
    Running {
        url,
        path,
        reload,
        stop,
        task,
        _dir: dir,
    }
}

impl Running {
    async fn health(&self) -> Value {
        reqwest::get(format!("{}/v1/health", self.url))
            .await
            .unwrap()
            .json()
            .await
            .unwrap()
    }

    /// Writes the file, asks for a reload and waits until it has been attempted.
    async fn reload_with(&self, text: &str) -> Value {
        let before = self.health().await["reload"]["attempts"]
            .as_u64()
            .unwrap_or(0);
        std::fs::write(&self.path, text).unwrap();
        self.reload.send(()).await.unwrap();
        for _ in 0..100 {
            let health = self.health().await;
            if health["reload"]["attempts"].as_u64().unwrap_or(0) > before {
                return health;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("the reload did not finish");
    }
}

#[tokio::test]
async fn a_reload_adds_changes_and_removes_defaults_and_a_bad_table_keeps_the_old_file_serving() {
    let (url, up) = upstream(false).await;
    let g = start(&config_text(&url, "")).await;

    chat(&g.url, json!({})).await;
    assert!(last(&up).get("temperature").is_none());

    let health = g
        .reload_with(&config_text(
            &url,
            ", request_defaults = { temperature = 0.7 }",
        ))
        .await;
    assert_eq!(health["reload"]["ok"], true, "{health}");
    chat(&g.url, json!({})).await;
    assert_eq!(last(&up)["temperature"], 0.7, "added");

    let health = g
        .reload_with(&config_text(
            &url,
            ", request_defaults = { temperature = 0.5 }",
        ))
        .await;
    assert_eq!(health["reload"]["ok"], true, "{health}");
    chat(&g.url, json!({})).await;
    assert_eq!(last(&up)["temperature"], 0.5, "changed");

    // A field outside the list never takes effect, and the previous table keeps serving.
    let health = g
        .reload_with(&config_text(
            &url,
            ", request_defaults = { max_tokens = 5 }",
        ))
        .await;
    assert_eq!(health["reload"]["ok"], false, "{health}");
    assert!(
        health["reload"]["message"]
            .as_str()
            .unwrap()
            .contains("request_defaults.max_tokens"),
        "{health}"
    );
    chat(&g.url, json!({})).await;
    assert_eq!(last(&up)["temperature"], 0.5, "the old table still applies");

    let health = g.reload_with(&config_text(&url, "")).await;
    assert_eq!(health["reload"]["ok"], true, "{health}");
    chat(&g.url, json!({})).await;
    assert!(last(&up).get("temperature").is_none(), "removed");

    g.stop.send(()).await.unwrap();
    let _ = tokio::time::timeout(Duration::from_secs(10), g.task).await;
}
