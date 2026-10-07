//! Route behavior end to end: built-in Switchyard algorithms over a mock provider.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex, OnceLock};

use axum::Router;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use serde_json::{Value, json};
use switchyard_conductor::config::Config;
use switchyard_conductor::server;

/// Records every upstream model called. The judge model answers with `judge_reply`; every other
/// model answers with its own name so tests can see who served. Models listed in `broken` fail.
#[derive(Default)]
struct Mock {
    calls: Mutex<Vec<String>>,
    judge_reply: Mutex<String>,
    broken: Mutex<Vec<String>>,
}

impl Mock {
    fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
    fn answers(&self) -> Vec<String> {
        self.calls()
            .into_iter()
            .filter(|m| m != "judge-m")
            .collect()
    }
}

async fn completions(
    State(mock): State<Arc<Mock>>,
    axum::Json(body): axum::Json<Value>,
) -> Response {
    let model = body["model"].as_str().unwrap().to_string();
    mock.calls.lock().unwrap().push(model.clone());
    if mock.broken.lock().unwrap().contains(&model) {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    }
    let content = if model == "judge-m" {
        mock.judge_reply.lock().unwrap().clone()
    } else {
        model.clone()
    };
    axum::Json(json!({"id": "c", "object": "chat.completion", "created": 1, "model": model,
        "choices": [{"index": 0, "message": {"role": "assistant", "content": content}, "finish_reason": "stop"}]}))
    .into_response()
}

async fn serve(app: Router) -> SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    addr
}

struct Harness {
    url: String,
    mock: Arc<Mock>,
}

async fn harness(routes: &str) -> Harness {
    logs();
    let mock = Arc::new(Mock::default());
    let upstream = serve(
        Router::new()
            .route("/chat/completions", post(completions))
            .with_state(mock.clone()),
    )
    .await;
    let toml = format!(
        r#"listen = "127.0.0.1:0"
[providers.mock]
base_url = "http://{upstream}"
api_key_env = "K"
max_retries = 0
[targets.fast]
endpoints = [{{ provider = "mock", model = "fast-m" }}]
[targets.smart]
endpoints = [{{ provider = "mock", model = "smart-m" }}]
[targets.judge]
endpoints = [{{ provider = "mock", model = "judge-m" }}]
[targets.a]
endpoints = [{{ provider = "mock", model = "a-m" }}]
[targets.b]
endpoints = [{{ provider = "mock", model = "b-m" }}]
{routes}"#
    );
    let config = Config::from_toml(&toml, |_| Some("key".into())).unwrap();
    let gateway = serve(server::router(config).unwrap()).await;
    Harness {
        url: format!("http://{gateway}"),
        mock,
    }
}

fn plain(model: &str) -> Value {
    json!({"model": model, "messages": [{"role": "user", "content": "hello"}]})
}

/// An agent turn: a task, a Bash tool call, and its result (failed or fine).
fn tool_turn(model: &str, failed: bool) -> Value {
    let result = if failed {
        "fatal runtime error: out of memory"
    } else {
        "ok"
    };
    json!({"model": model, "messages": [
        {"role": "user", "content": "fix the build"},
        {"role": "assistant", "content": null, "tool_calls": [{"id": "call_1", "type": "function",
            "function": {"name": "Bash", "arguments": "{\"command\":\"cargo test\"}"}}]},
        {"role": "tool", "tool_call_id": "call_1", "content": result}]})
}

struct Reply {
    status: u16,
    target: String,
    body: Value,
}

async fn send(h: &Harness, body: Value, session: Option<&str>) -> Reply {
    let mut req = reqwest::Client::new()
        .post(format!("{}/v1/chat/completions", h.url))
        .json(&body);
    if let Some(s) = session {
        req = req.header("x-switchyard-session-id", s);
    }
    let resp = req.send().await.unwrap();
    let status = resp.status().as_u16();
    let target = resp
        .headers()
        .get("x-conductor-target")
        .map(|v| v.to_str().unwrap().to_string())
        .unwrap_or_default();
    Reply {
        status,
        target,
        body: resp.json().await.unwrap_or(Value::Null),
    }
}

const STAGE: &str = r#"
[routes.auto]
type = "stage_router"
efficient = ["fast"]
capable = ["smart"]
"#;

#[tokio::test]
async fn passthrough_and_direct_targets_reach_their_target() {
    let h = harness("[routes.p]\ntype = \"passthrough\"\ntargets = [\"fast\"]\n").await;
    assert_eq!(send(&h, plain("p"), None).await.target, "fast");
    assert_eq!(send(&h, plain("smart"), None).await.target, "smart");
}

#[tokio::test]
async fn random_route_honors_weights() {
    let h = harness(
        "[routes.r]\ntype = \"random\"\ntargets = [\"fast\", \"smart\"]\nweights = [0.0, 1.0]\nseed = 1\n",
    )
    .await;
    for _ in 0..5 {
        assert_eq!(send(&h, plain("r"), None).await.target, "smart");
    }
}

#[tokio::test]
async fn stage_router_escalates_on_failing_tool_results() {
    let h = harness(STAGE).await;
    let failing = send(&h, tool_turn("auto", true), Some("s-fail")).await;
    assert_eq!((failing.status, failing.target.as_str()), (200, "smart"));
    assert_eq!(failing.body["choices"][0]["message"]["content"], "smart-m");
    let clean = send(&h, tool_turn("auto", false), Some("s-clean")).await;
    assert_eq!(clean.target, "fast");
}

#[tokio::test]
async fn escalation_state_is_per_session() {
    let h = harness(STAGE).await;
    assert_eq!(
        send(&h, tool_turn("auto", true), Some("s1")).await.target,
        "smart"
    );
    // Same session, clean turn: the capable hold persists.
    assert_eq!(
        send(&h, tool_turn("auto", false), Some("s1")).await.target,
        "smart"
    );
    // Different session is unaffected.
    assert_eq!(
        send(&h, tool_turn("auto", false), Some("s2")).await.target,
        "fast"
    );
}

const CLASSIFIER: &str = r#"
[routes.smartish]
type = "llm_classifier"
mode = "capability"
efficient = ["fast"]
capable = ["smart"]
judge = ["judge"]
"#;

fn verdict(boundary: &str, p_solve: f64) -> String {
    json!({"crux": "bounded task", "primary_rule": "SUP-1", "capability_boundary": boundary, "p_solve": p_solve}).to_string()
}

#[tokio::test]
async fn classifier_judge_calls_go_through_the_pool_and_stay_invisible() {
    let h = harness(CLASSIFIER).await;
    *h.mock.judge_reply.lock().unwrap() = verdict("supported", 0.95);
    let easy = send(&h, plain("smartish"), None).await;
    assert_eq!(easy.target, "fast");
    assert_eq!(
        easy.body["choices"][0]["message"]["content"], "fast-m",
        "the verdict must not reach the client"
    );
    assert!(h.mock.calls().contains(&"judge-m".to_string()));

    *h.mock.judge_reply.lock().unwrap() = verdict("unsupported", 0.05);
    let hard = send(&h, plain("smartish"), None).await;
    assert_eq!(hard.target, "smart");
}

#[tokio::test]
async fn whole_target_failure_falls_back_to_the_next_selected_target() {
    let h = harness("[routes.chain]\ntype = \"passthrough\"\ntargets = [\"a\", \"b\"]\n").await;
    h.mock.broken.lock().unwrap().push("a-m".into());
    let reply = send(&h, plain("chain"), None).await;
    assert_eq!((reply.status, reply.target.as_str()), (200, "b"));
    assert_eq!(h.mock.answers(), ["a-m", "b-m"]);
}

#[tokio::test]
async fn streaming_responses_carry_attribution_headers() {
    let h = harness("").await;
    let resp = reqwest::Client::new()
        .post(format!("{}/v1/chat/completions", h.url))
        .json(&json!({"model": "fast", "stream": true, "messages": [{"role": "user", "content": "hi"}]}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.headers()["x-conductor-target"], "fast");
    assert_eq!(resp.headers()["x-conductor-provider"], "mock");
}

#[derive(Clone, Default)]
struct LogBuf(Arc<Mutex<Vec<u8>>>);
impl std::io::Write for LogBuf {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for LogBuf {
    type Writer = LogBuf;
    fn make_writer(&'a self) -> LogBuf {
        self.clone()
    }
}

/// One process-wide subscriber: per-test subscribers race on tracing's callsite cache.
fn logs() -> &'static LogBuf {
    static LOGS: OnceLock<LogBuf> = OnceLock::new();
    LOGS.get_or_init(|| {
        let buf = LogBuf::default();
        tracing_subscriber::fmt()
            .with_max_level(tracing::Level::INFO)
            .with_ansi(false)
            .with_writer(buf.clone())
            .init();
        buf
    })
}

#[tokio::test]
async fn request_log_names_target_and_provider() {
    let h = harness("").await;
    send(&h, plain("fast"), None).await;
    let captured = String::from_utf8(logs().0.lock().unwrap().clone()).unwrap();
    assert!(
        captured.contains("target=\"fast\"") || captured.contains("target=fast"),
        "{captured}"
    );
    assert!(
        captured.contains("provider=\"mock\"") || captured.contains("provider=mock"),
        "{captured}"
    );
}

#[tokio::test]
async fn escalation_mode_route_starts_on_the_efficient_target() {
    let h = harness(
        "[routes.agent]\ntype = \"llm_classifier\"\nmode = \"escalation\"\nefficient = [\"fast\"]\ncapable = [\"smart\"]\njudge = [\"judge\"]\n",
    )
    .await;
    let reply = send(&h, plain("agent"), Some("esc-1")).await;
    assert_eq!((reply.status, reply.target.as_str()), (200, "fast"));
}

/// A Claude-Code-style Anthropic client: the agent loop starts on the efficient model, then a
/// failing tool result escalates the same session to the capable one, all through /v1/messages.
#[tokio::test]
async fn anthropic_agent_escalates_after_a_failing_tool_result() {
    let h = harness(STAGE).await;
    let turn = |is_error: bool, result: &str| {
        json!({"model": "auto", "max_tokens": 100, "messages": [
            {"role": "user", "content": "fix the build"},
            {"role": "assistant", "content": [{"type": "tool_use", "id": "toolu_1", "name": "Bash", "input": {"command": "cargo test"}}]},
            {"role": "user", "content": [{"type": "tool_result", "tool_use_id": "toolu_1", "is_error": is_error, "content": result}]}]})
    };
    let call = |body: Value| {
        let url = format!("{}/v1/messages", h.url);
        async move {
            let resp = reqwest::Client::new()
                .post(url)
                .header("x-switchyard-session-id", "agent-1")
                .json(&body)
                .send()
                .await
                .unwrap();
            let target = resp.headers()["x-conductor-target"]
                .to_str()
                .unwrap()
                .to_string();
            let body: Value = resp.json().await.unwrap();
            (target, body)
        }
    };
    let (target, body) = call(turn(false, "ok")).await;
    assert_eq!(target, "fast");
    assert_eq!(body["type"], "message");
    let (target, body) = call(turn(true, "fatal runtime error: out of memory")).await;
    assert_eq!(target, "smart");
    assert_eq!(body["content"][0]["text"], "smart-m");
}
