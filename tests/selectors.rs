//! Route selectors end to end: rules pick the route from the key, headers, tags and agent
//! metadata; clients can narrow but never widen; the decision is visible in headers and the ledger.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test scaffolding

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use axum::Router;
use axum::extract::State;
use axum::response::IntoResponse;
use axum::routing::post;
use common::{chat_completion, chat_request, serve, usage};
use humpyard::config::Config;
use humpyard::ledger::Ledger;
use humpyard::server;
use serde_json::{Value, json};

async fn completions(State(calls): State<Arc<AtomicUsize>>) -> impl IntoResponse {
    calls.fetch_add(1, Ordering::SeqCst);
    axum::Json(chat_completion("m", "ok", Some(usage(10, 5))))
}

async fn upstream() -> (Arc<AtomicUsize>, String) {
    let calls = Arc::new(AtomicUsize::new(0));
    let app = Router::new()
        .route("/chat/completions", post(completions))
        .with_state(calls.clone());
    (calls, format!("http://{}", serve(app).await))
}

struct Rig {
    url: String,
    cheap_calls: Arc<AtomicUsize>,
    premium_calls: Arc<AtomicUsize>,
    keys: std::collections::HashMap<&'static str, String>,
    db: std::path::PathBuf,
    _dir: tempfile::TempDir,
}

/// Targets `cheap` and `premium`; routes `plain` (cheap), `cheap-first` (cheap then premium) and
/// `premium-only`; keys `full` (unrestricted), `limited` (no premium-only) and `ci-nightly`.
async fn rig(select: &str) -> Rig {
    let (cheap_calls, cheap_url) = upstream().await;
    let (premium_calls, premium_url) = upstream().await;
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("ledger.db");
    let mut keys = std::collections::HashMap::new();
    let mut key_blocks = Vec::new();
    for (id, extra) in [
        ("full", ""),
        ("limited", "allowed_routes = [\"plain\", \"cheap-first\"]\n"),
        ("ci-nightly", ""),
    ] {
        let (key, hash) = humpyard::auth::generate_key().unwrap();
        key_blocks.push(format!("[keys.{id}]\nsha256 = \"{hash}\"\n{extra}"));
        keys.insert(id, key);
    }
    let toml = format!(
        r#"listen = "127.0.0.1:0"
[ledger]
path = "{db}"
{key_blocks}
[providers.a]
base_url = "{cheap_url}"
api_key_env = "K"
[providers.b]
base_url = "{premium_url}"
api_key_env = "K"
[targets.cheap]
endpoints = [{{ provider = "a", model = "small" }}]
[targets.premium]
endpoints = [{{ provider = "b", model = "big" }}]
[routes.plain]
type = "passthrough"
targets = ["cheap"]
[routes.cheap-first]
type = "passthrough"
targets = ["cheap", "premium"]
[routes.premium-only]
type = "passthrough"
targets = ["premium"]
{select}
"#,
        db = db.display(),
        key_blocks = key_blocks.join("\n")
    );
    let config = Config::from_toml(&toml, |_| Some("secret".into())).unwrap();
    Rig {
        url: format!(
            "http://{}",
            serve(server::router(config).await.unwrap()).await
        ),
        cheap_calls,
        premium_calls,
        keys,
        db,
        _dir: dir,
    }
}

const RULES: &str = r#"
[[select]]
name = "subagents"
when = { subagent = true }
route = "cheap-first"
[[select]]
name = "deep"
when = { profile = "deep" }
route = "premium-only"
[[select]]
name = "infra"
when = { tag = { team = "infra" } }
route = "premium-only"
[[select]]
name = "ci"
when = { key = "ci-*" }
route = "premium-only"
[[select]]
name = "openclaw"
when = { header = { "X-Client" = "open*" } }
route = "cheap-first"
"#;

struct Answer {
    status: u16,
    route: Option<String>,
    rule: Option<String>,
    body: Value,
}

impl Rig {
    async fn ask(&self, key: &str, model: &str, headers: &[(&str, &str)]) -> Answer {
        self.ask_body(key, chat_request(model, false), headers)
            .await
    }

    async fn ask_body(&self, key: &str, body: Value, headers: &[(&str, &str)]) -> Answer {
        let mut req = reqwest::Client::new()
            .post(format!("{}/v1/chat/completions", self.url))
            .bearer_auth(&self.keys[key])
            .json(&body);
        for (k, v) in headers {
            req = req.header(*k, *v);
        }
        let resp = req.send().await.unwrap();
        let get = |name: &str| {
            resp.headers()
                .get(name)
                .map(|v| v.to_str().unwrap().to_string())
        };
        let (route, rule, status) = (
            get("x-humpyard-route"),
            get("x-humpyard-rule"),
            resp.status().as_u16(),
        );
        Answer {
            status,
            route,
            rule,
            body: resp.json().await.unwrap_or(Value::Null),
        }
    }

    fn calls(&self) -> (usize, usize) {
        (
            self.cheap_calls.load(Ordering::SeqCst),
            self.premium_calls.load(Ordering::SeqCst),
        )
    }
}

fn picked(a: &Answer) -> (u16, &str, &str) {
    (
        a.status,
        a.route.as_deref().unwrap_or(""),
        a.rule.as_deref().unwrap_or(""),
    )
}

#[tokio::test]
async fn without_a_matching_rule_the_requested_model_is_the_route() {
    let r = rig(RULES).await;
    assert_eq!(
        picked(&r.ask("full", "plain", &[]).await),
        (200, "plain", "default")
    );
    assert_eq!(r.calls(), (1, 0));
    // Unknown models are still 404 when no rule applies.
    assert_eq!(r.ask("full", "nope", &[]).await.status, 404);
}

#[tokio::test]
async fn headers_profiles_tags_keys_and_agent_metadata_choose_the_route() {
    let r = rig(RULES).await;
    let deep = r
        .ask("full", "plain", &[("x-humpyard-profile", "deep")])
        .await;
    assert_eq!(picked(&deep), (200, "premium-only", "deep"));
    let tagged = r
        .ask("full", "plain", &[("X-Humpyard-Tag-Team", "infra")])
        .await;
    assert_eq!(picked(&tagged), (200, "premium-only", "infra"));
    let ci = r.ask("ci-nightly", "plain", &[]).await;
    assert_eq!(picked(&ci), (200, "premium-only", "ci"));
    let named = r.ask("full", "plain", &[("x-client", "openclaw")]).await;
    assert_eq!(picked(&named), (200, "cheap-first", "openclaw"));
    let sub = r
        .ask("full", "plain", &[("x-switchyard-is-subagent", "true")])
        .await;
    assert_eq!(picked(&sub), (200, "cheap-first", "subagents"));
    assert_eq!(
        r.calls(),
        (2, 3),
        "cheap served the two cheap-first requests"
    );
}

#[tokio::test]
async fn the_first_matching_rule_wins() {
    let r = rig(RULES).await;
    // Both `deep` and `openclaw` match; `deep` is listed first.
    let a = r
        .ask(
            "full",
            "plain",
            &[("x-humpyard-profile", "deep"), ("x-client", "openclaw")],
        )
        .await;
    assert_eq!(picked(&a), (200, "premium-only", "deep"));
}

#[tokio::test]
async fn a_rule_can_rescue_a_model_name_the_gateway_does_not_know() {
    let r = rig(RULES).await;
    // Clients that hard-code a model name still get routed by what they say about themselves.
    let a = r.ask("full", "gpt-4o", &[("x-client", "openclaw")]).await;
    assert_eq!(picked(&a), (200, "cheap-first", "openclaw"));
    assert_eq!(r.ask("full", "gpt-4o", &[]).await.status, 404);
}

#[tokio::test]
async fn client_headers_can_narrow_but_never_widen_a_restricted_key() {
    let r = rig(RULES).await;
    // `limited` may not use premium-only. The profile rule would send it there, so it is skipped
    // and the request follows the requested model instead of failing or escalating.
    let a = r
        .ask("limited", "plain", &[("x-humpyard-profile", "deep")])
        .await;
    assert_eq!(picked(&a), (200, "plain", "default"));
    assert_eq!(r.calls(), (1, 0), "premium untouched");
    // The next applicable rule still applies.
    let b = r
        .ask(
            "limited",
            "plain",
            &[("x-humpyard-profile", "deep"), ("x-client", "openclaw")],
        )
        .await;
    assert_eq!(picked(&b), (200, "cheap-first", "openclaw"));
    // Asking for the forbidden route directly is still refused.
    let c = r.ask("limited", "premium-only", &[]).await;
    assert_eq!(c.status, 403, "{:?}", c.body);
    assert_eq!(r.calls().1, 0);
}

#[tokio::test]
async fn the_stream_flag_is_a_condition() {
    let r = rig(r#"
[[select]]
name = "streams"
when = { stream = true }
route = "premium-only"
"#)
    .await;
    let plain = r.ask("full", "plain", &[]).await;
    assert_eq!(picked(&plain), (200, "plain", "default"));
    let streamed = reqwest::Client::new()
        .post(format!("{}/v1/chat/completions", r.url))
        .bearer_auth(&r.keys["full"])
        .json(&chat_request("plain", true))
        .send()
        .await
        .unwrap();
    assert_eq!(streamed.headers()["x-humpyard-rule"], "streams");
    assert_eq!(streamed.headers()["x-humpyard-route"], "premium-only");
}

#[tokio::test]
async fn token_counting_follows_the_same_selection() {
    let r = rig(RULES).await;
    let resp = reqwest::Client::new()
        .post(format!("{}/v1/messages/count_tokens", r.url))
        .bearer_auth(&r.keys["limited"])
        .header("x-client", "openclaw")
        .json(&json!({"model": "gpt-4o", "max_tokens": 10, "messages": [{"role": "user", "content": "hi"}]}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200, "{:?}", resp.text().await);
}

#[tokio::test]
async fn the_ledger_records_the_route_that_served() {
    let r = rig(RULES).await;
    r.ask("full", "plain", &[("x-humpyard-profile", "deep")])
        .await;
    tokio::time::sleep(Duration::from_millis(400)).await;
    let rows = Ledger::open(&r.db)
        .await
        .unwrap()
        .entries_since(0)
        .await
        .unwrap();
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0].route, "premium-only");
    assert_eq!(rows[0].target, "premium");
}

// ---- config validation ----

fn load(select: &str) -> Result<Config, String> {
    let toml = format!(
        r#"listen = "127.0.0.1:0"
[providers.p]
base_url = "http://127.0.0.1:1"
api_key_env = "K"
[targets.t]
endpoints = [{{ provider = "p", model = "m" }}]
[routes.r]
type = "passthrough"
targets = ["t"]
{select}
"#
    );
    Config::from_toml(&toml, |_| Some("k".into())).map_err(|e| e.to_string())
}

#[test]
fn invalid_selector_config_is_rejected_with_a_clear_message() {
    let cases = [
        ("[[select]]\nroute = \"missing\"", "unknown route `missing`"),
        (
            "[[select]]\nwhen = { header = { authorization = \"*\" } }\nroute = \"r\"",
            "credential header",
        ),
        (
            "[[select]]\nwhen = { header = { \"X-Api-Key\" = \"*\" } }\nroute = \"r\"",
            "credential header",
        ),
        (
            "[[select]]\nwhen = { tag = { \"bad name\" = \"x\" } }\nroute = \"r\"",
            "invalid tag name",
        ),
        ("[[select]]\nwhen = { bogus = 1 }\nroute = \"r\"", "bogus"),
        (
            "[[select]]\nroute = \"r\"\n[[select]]\nwhen = { key = \"a\" }\nroute = \"r\"",
            "can never match",
        ),
        (
            "[[select]]\nname = \"x\"\nwhen = { key = \"a\" }\nroute = \"r\"\n[[select]]\nname = \"x\"\nwhen = { key = \"b\" }\nroute = \"r\"",
            "used twice",
        ),
    ];
    for (select, expected) in cases {
        let message = load(select)
            .err()
            .unwrap_or_else(|| panic!("accepted: {select}"));
        assert!(message.contains(expected), "{select}: {message}");
    }
    assert!(
        load("[[select]]\nwhen = { key = \"a*\" }\nroute = \"r\"\n[[select]]\nroute = \"t\"")
            .is_ok()
    );
}

// ---- conditions on what the request contains ----

const CONTENT_RULES: &str = r#"
[[select]]
name = "vision"
when = { images = true }
route = "premium-only"
[[select]]
name = "long"
when = { prompt_tokens = { min = 2000 } }
route = "premium-only"
[[select]]
name = "tools"
when = { tools = true }
route = "cheap-first"
"#;

fn long_prompt() -> String {
    "x".repeat(30_000)
}

fn with_tools(prompt: &str) -> Value {
    json!({"model": "plain", "messages": [{"role": "user", "content": prompt}],
           "tools": [{"type": "function", "function": {"name": "ls", "description": "list",
                      "parameters": {"type": "object", "properties": {}}}}]})
}

fn with_image() -> Value {
    json!({"model": "plain", "messages": [{"role": "user", "content": [
        {"type": "text", "text": "what is this?"},
        {"type": "image_url", "image_url": {"url": "data:image/png;base64,iVBORw0KGgo="}}
    ]}]})
}

#[tokio::test]
async fn prompt_size_tools_and_images_choose_the_route() {
    let r = rig(CONTENT_RULES).await;
    let small = r.ask("full", "plain", &[]).await;
    assert_eq!(picked(&small), (200, "plain", "default"));
    let long = r
        .ask_body("full", chat_request_with("plain", &long_prompt()), &[])
        .await;
    assert_eq!(picked(&long), (200, "premium-only", "long"));
    let tools = r.ask_body("full", with_tools("hi"), &[]).await;
    assert_eq!(picked(&tools), (200, "cheap-first", "tools"));
    let image = r.ask_body("full", with_image(), &[]).await;
    assert_eq!(picked(&image), (200, "premium-only", "vision"));
    // Order matters: a long prompt with tools follows the earlier `long` rule.
    let both = r.ask_body("full", with_tools(&long_prompt()), &[]).await;
    assert_eq!(picked(&both), (200, "premium-only", "long"));
}

fn chat_request_with(model: &str, prompt: &str) -> Value {
    json!({"model": model, "messages": [{"role": "user", "content": prompt}]})
}

#[tokio::test]
async fn explain_takes_the_features_and_agrees_with_the_gateways_own_estimate() {
    let r = rig(CONTENT_RULES).await;
    let prompt = long_prompt();
    // Ask the gateway how big it thinks the prompt is, then explain with that number.
    let counted: Value = reqwest::Client::new()
        .post(format!("{}/v1/messages/count_tokens", r.url))
        .bearer_auth(&r.keys["full"])
        .json(&json!({"model": "plain", "max_tokens": 1, "messages": [{"role": "user", "content": prompt}]}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let tokens = counted["input_tokens"].as_u64().unwrap();
    assert!(tokens > 2000, "{counted}");

    let explain = |body: Value| {
        let client = reqwest::Client::new();
        let (url, key) = (
            format!("{}/v1/route/explain", r.url),
            r.keys["full"].clone(),
        );
        async move {
            client
                .post(url)
                .bearer_auth(key)
                .json(&body)
                .send()
                .await
                .unwrap()
                .json::<Value>()
                .await
                .unwrap()
        }
    };
    let report = explain(json!({"model": "plain", "prompt_tokens": tokens})).await;
    assert_eq!(report["selected"]["rule"], "long", "{report}");
    let real = r
        .ask_body("full", chat_request_with("plain", &prompt), &[])
        .await;
    assert_eq!(real.rule.as_deref(), report["selected"]["rule"].as_str());

    let small = explain(json!({"model": "plain", "prompt_tokens": 10})).await;
    assert_eq!(small["selected"]["rule"], "default");
    assert_eq!(
        small["rules"][1]["mismatches"],
        json!(["prompt_tokens: wanted at least 2000, got 10"])
    );
    let unknown = explain(json!({"model": "plain"})).await;
    assert_eq!(
        unknown["rules"][1]["mismatches"],
        json!(["prompt_tokens: the request's size is not known"])
    );
    let flags = explain(json!({"model": "plain", "tools": true, "images": true})).await;
    assert_eq!(flags["selected"]["rule"], "vision");
}

#[test]
fn prompt_token_ranges_are_validated() {
    for (select, expected) in [
        (
            "[[select]]\nwhen = { prompt_tokens = {} }\nroute = \"r\"",
            "needs a `min` or a `max`",
        ),
        (
            "[[select]]\nwhen = { prompt_tokens = { min = 10, max = 5 } }\nroute = \"r\"",
            "above max",
        ),
        (
            "[[select]]\nwhen = { prompt_tokens = { least = 5 } }\nroute = \"r\"",
            "least",
        ),
    ] {
        let message = load(select)
            .err()
            .unwrap_or_else(|| panic!("accepted: {select}"));
        assert!(message.contains(expected), "{select}: {message}");
    }
    assert!(
        load("[[select]]\nwhen = { prompt_tokens = { min = 5, max = 5 } }\nroute = \"r\"").is_ok()
    );
}
