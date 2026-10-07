//! Routing-policy seam: eligibility narrows targets, tiers degrade, nothing eligible is a 503.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::format_push_string,
    clippy::assert_is_empty
)] // test scaffolding: fail loudly, favor readability

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::extract::State;
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use humpyard::config::Config;
use humpyard::policy::{PolicyContext, RoutingPolicy};
use humpyard::server;
use serde_json::{Value, json};

/// Records every upstream model called; the judge answers with a fixed, usable verdict.
#[derive(Default)]
struct Mock {
    calls: Mutex<Vec<String>>,
}

async fn completions(
    State(mock): State<Arc<Mock>>,
    axum::Json(body): axum::Json<Value>,
) -> Response {
    let model = body["model"].as_str().unwrap().to_string();
    mock.calls.lock().unwrap().push(model.clone());
    let content = if model == "judge-m" {
        json!({"crux": "x", "primary_rule": "SUP-1", "capability_boundary": "supported", "p_solve": 0.9}).to_string()
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

/// Excludes a fixed set of targets and records what it was asked about.
#[derive(Default)]
struct Deny {
    targets: Vec<&'static str>,
    seen: Mutex<Vec<(String, Option<String>)>>,
}

impl RoutingPolicy for Deny {
    fn is_eligible(&self, context: &PolicyContext<'_>, target: &str) -> bool {
        self.seen.lock().unwrap().push((
            context.route.to_string(),
            context.session_id.map(String::from),
        ));
        !self.targets.contains(&target)
    }
}

fn deny(targets: &[&'static str]) -> Arc<Deny> {
    Arc::new(Deny {
        targets: targets.to_vec(),
        ..Default::default()
    })
}

struct Harness {
    url: String,
    mock: Arc<Mock>,
}

impl Harness {
    fn called(&self, model: &str) -> bool {
        self.mock.calls.lock().unwrap().iter().any(|m| m == model)
    }
}

const ROUTES: &str = r#"
[routes.auto]
type = "stage_router"
efficient = ["fast"]
capable = ["smart"]
[routes.chain]
type = "passthrough"
targets = ["a", "b"]
[routes.split]
type = "random"
targets = ["fast", "smart"]
weights = [1.0, 0.0]
seed = 3
[routes.judged]
type = "llm_classifier"
mode = "capability"
efficient = ["fast"]
capable = ["smart"]
judge = ["judge"]
"#;

async fn harness(policy: Arc<dyn RoutingPolicy>) -> Harness {
    let mock = Arc::new(Mock::default());
    let upstream = serve(
        Router::new()
            .route("/chat/completions", post(completions))
            .with_state(mock.clone()),
    )
    .await;
    let mut toml = format!(
        "listen = \"127.0.0.1:0\"\n[providers.mock]\nbase_url = \"http://{upstream}\"\napi_key_env = \"K\"\nmax_retries = 0\n"
    );
    for (target, model) in [
        ("fast", "fast-m"),
        ("smart", "smart-m"),
        ("judge", "judge-m"),
        ("a", "a-m"),
        ("b", "b-m"),
    ] {
        toml += &format!(
            "[targets.{target}]\nendpoints = [{{ provider = \"mock\", model = \"{model}\" }}]\n"
        );
    }
    toml += ROUTES;
    let config = Config::from_toml(&toml, |_| Some("key".into())).unwrap();
    let gateway = serve(server::router_with_policy(config, policy).await.unwrap()).await;
    Harness {
        url: format!("http://{gateway}"),
        mock,
    }
}

fn plain(model: &str) -> Value {
    json!({"model": model, "messages": [{"role": "user", "content": "hello"}]})
}

fn failing_turn(model: &str) -> Value {
    json!({"model": model, "messages": [
        {"role": "user", "content": "fix the build"},
        {"role": "assistant", "content": null, "tool_calls": [{"id": "call_1", "type": "function",
            "function": {"name": "Bash", "arguments": "{\"command\":\"cargo test\"}"}}]},
        {"role": "tool", "tool_call_id": "call_1", "content": "fatal runtime error: out of memory"}]})
}

fn clean_turn(model: &str) -> Value {
    let mut body = failing_turn(model);
    body["messages"][2]["content"] = json!("ok");
    body
}

async fn post_json(
    h: &Harness,
    path: &str,
    body: Value,
    session: Option<&str>,
) -> reqwest::Response {
    let mut req = reqwest::Client::new()
        .post(format!("{}{path}", h.url))
        .json(&body);
    if let Some(s) = session {
        req = req.header("x-switchyard-session-id", s);
    }
    req.send().await.unwrap()
}

async fn served(h: &Harness, body: Value) -> (u16, String) {
    let resp = post_json(h, "/v1/chat/completions", body, None).await;
    let target = resp
        .headers()
        .get("x-humpyard-target")
        .map(|v| v.to_str().unwrap().to_string())
        .unwrap_or_default();
    (resp.status().as_u16(), target)
}

#[tokio::test]
async fn default_policy_allows_every_target() {
    let h = harness(Arc::new(humpyard::policy::AllowAll)).await;
    assert_eq!(
        served(&h, failing_turn("auto")).await,
        (200, "smart".to_string())
    );
}

#[tokio::test]
async fn excluded_capable_tier_degrades_to_efficient() {
    let h = harness(deny(&["smart"])).await;
    // This turn would escalate; with capable excluded it is served by the efficient target.
    assert_eq!(
        served(&h, failing_turn("auto")).await,
        (200, "fast".to_string())
    );
    assert!(!h.called("smart-m"));
}

#[tokio::test]
async fn excluded_efficient_tier_degrades_to_capable() {
    let h = harness(deny(&["fast"])).await;
    assert_eq!(
        served(&h, clean_turn("auto")).await,
        (200, "smart".to_string())
    );
    assert!(!h.called("fast-m"));
}

#[tokio::test]
async fn excluded_target_is_never_used_even_as_a_fallback() {
    let h = harness(deny(&["a"])).await;
    assert_eq!(served(&h, plain("chain")).await, (200, "b".to_string()));
    assert!(!h.called("a-m"));
    // Direct request for the excluded target itself has nothing eligible.
    assert_eq!(served(&h, plain("a")).await.0, 503);
}

#[tokio::test]
async fn random_weights_realign_when_a_target_is_removed() {
    // Weights [1, 0] favor `fast`; with `fast` excluded the only choice left is `smart`.
    let h = harness(deny(&["fast"])).await;
    assert_eq!(served(&h, plain("split")).await, (200, "smart".to_string()));
}

#[tokio::test]
async fn excluded_judge_is_replaced_not_called() {
    let h = harness(deny(&["judge"])).await;
    let (status, _) = served(&h, plain("judged")).await;
    assert_eq!(status, 200);
    assert!(!h.called("judge-m"));
}

#[tokio::test]
async fn nothing_eligible_is_503_in_every_protocol_shape() {
    let h = harness(deny(&["fast", "smart", "judge", "a", "b"])).await;

    let chat = post_json(&h, "/v1/chat/completions", plain("auto"), None).await;
    assert_eq!(chat.status(), 503);
    let body: Value = chat.json().await.unwrap();
    let message = body["error"]["message"].as_str().unwrap();
    assert!(
        message.contains("auto") && message.contains("routing policy"),
        "{message}"
    );

    let anthropic = post_json(
        &h,
        "/v1/messages",
        json!({"model": "auto", "max_tokens": 10, "messages": [{"role": "user", "content": "hi"}]}),
        None,
    )
    .await;
    assert_eq!(anthropic.status(), 503);
    let body: Value = anthropic.json().await.unwrap();
    assert_eq!(body["type"], "error");
    assert!(body["error"]["message"].as_str().unwrap().contains("auto"));

    let responses = post_json(
        &h,
        "/v1/responses",
        json!({"model": "auto", "input": "hi"}),
        None,
    )
    .await;
    assert_eq!(responses.status(), 503);
    let body: Value = responses.json().await.unwrap();
    assert!(body["error"]["message"].as_str().unwrap().contains("auto"));
}

#[tokio::test]
async fn policy_sees_route_and_session() {
    let policy = deny(&[]);
    let h = harness(policy.clone()).await;
    post_json(&h, "/v1/chat/completions", plain("chain"), Some("sess-9")).await;
    let seen = policy.seen.lock().unwrap().clone();
    assert!(!seen.is_empty());
    assert!(
        seen.iter()
            .all(|(route, session)| route == "chain" && session.as_deref() == Some("sess-9")),
        "{seen:?}"
    );
}

#[tokio::test]
async fn models_lists_routes_and_targets() {
    let h = harness(deny(&[])).await;
    let models: Value = reqwest::get(format!("{}/v1/models", h.url))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let ids: Vec<&str> = models["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["id"].as_str().unwrap())
        .collect();
    for expected in [
        "auto", "chain", "split", "judged", "fast", "smart", "judge", "a", "b",
    ] {
        assert!(ids.contains(&expected), "{expected} missing from {ids:?}");
    }
}
