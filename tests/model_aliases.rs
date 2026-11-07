//! Model aliases (ADR 0033): another name for a route or target. A request for an alias is served
//! as the real name, authorization and the decision log see only the real name, `/v1/models`
//! lists the aliases a key may use with their labels, and `model_aliases = false` turns it all off.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test scaffolding

mod common;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::extract::State;
use axum::response::IntoResponse;
use axum::routing::post;
use common::{chat_completion, chat_request, serve, usage};
use humpyard::config::Config;
use humpyard::server;
use serde_json::{Value, json};

/// Records the upstream model of every call, so a test sees what the provider was asked for.
type Models = Arc<Mutex<Vec<String>>>;

async fn completions(
    State(models): State<Models>,
    axum::Json(body): axum::Json<Value>,
) -> axum::response::Response {
    models
        .lock()
        .unwrap()
        .push(body["model"].as_str().unwrap().to_string());
    axum::Json(chat_completion("m", "ok", Some(usage(10, 5)))).into_response()
}

struct Rig {
    url: String,
    keys: HashMap<&'static str, String>,
    models: Models,
    _dir: tempfile::TempDir,
}

/// Targets `a` and `b`; routes `plain` (a) and `premium` (b); keys `alice` (everything) and `bob`
/// (only `plain`). `extra` adds aliases, selector rules or `model_aliases`.
async fn rig(top: &str, extra: &str) -> Rig {
    let models = Models::default();
    let upstream = serve(
        Router::new()
            .route("/chat/completions", post(completions))
            .with_state(models.clone()),
    )
    .await;
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("ledger.db");
    let mut keys = HashMap::new();
    let mut blocks = Vec::new();
    for (id, allowed) in [("alice", ""), ("bob", "allowed_routes = [\"plain\"]")] {
        let (key, hash) = humpyard::auth::generate_key().unwrap();
        blocks.push(format!("[keys.{id}]\nsha256 = \"{hash}\"\n{allowed}"));
        keys.insert(id, key);
    }
    let toml = format!(
        r#"listen = "127.0.0.1:0"
{top}
[ledger]
path = "{db}"
{keys}
[providers.p]
base_url = "http://{upstream}"
api_key_env = "K"
max_retries = 0
[targets.a]
endpoints = [{{ provider = "p", model = "ma" }}]
[targets.b]
endpoints = [{{ provider = "p", model = "mb" }}]
[routes.plain]
type = "passthrough"
targets = ["a"]
[routes.premium]
type = "passthrough"
targets = ["b"]
{extra}
"#,
        db = db.display(),
        keys = blocks.join("\n")
    );
    let config = Config::from_toml(&toml, |_| Some("k".into())).unwrap();
    Rig {
        url: format!(
            "http://{}",
            serve(server::router(config).await.unwrap()).await
        ),
        keys,
        models,
        _dir: dir,
    }
}

const ALIASES: &str = r#"
[aliases.claude-plain]
model = "plain"
display_name = "Plain"
description = "The plain pool"
[aliases.claude-premium]
model = "premium"
[aliases.claude-a]
model = "a"
"#;

impl Rig {
    async fn ask(&self, key: &str, path: &str, body: Value) -> reqwest::Response {
        reqwest::Client::new()
            .post(format!("{}{path}", self.url))
            .bearer_auth(&self.keys[key])
            .json(&body)
            .send()
            .await
            .unwrap()
    }

    async fn chat(&self, key: &str, model: &str) -> reqwest::Response {
        self.ask(key, "/v1/chat/completions", chat_request(model, false))
            .await
    }

    async fn listed(&self, key: &str) -> Vec<Value> {
        let body: Value = reqwest::Client::new()
            .get(format!("{}/v1/models", self.url))
            .bearer_auth(&self.keys[key])
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        body["data"].as_array().unwrap().clone()
    }

    async fn decisions(&self, key: &str) -> Vec<Value> {
        let body: Value = reqwest::Client::new()
            .get(format!("{}/v1/decisions", self.url))
            .bearer_auth(&self.keys[key])
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        body["data"].as_array().unwrap().clone()
    }
}

fn ids(entries: &[Value]) -> Vec<&str> {
    entries.iter().map(|e| e["id"].as_str().unwrap()).collect()
}

#[tokio::test]
async fn an_alias_is_served_as_its_route_and_recorded_under_the_real_name() {
    let r = rig("", ALIASES).await;
    let resp = r.chat("alice", "claude-premium").await;
    assert_eq!(resp.status(), 200);
    assert_eq!(r.models.lock().unwrap().as_slice(), ["mb"]);
    let decisions = r.decisions("alice").await;
    assert_eq!(decisions.len(), 1, "{decisions:?}");
    assert_eq!(decisions[0]["requested_model"], "premium");
    assert_eq!(decisions[0]["route"], "premium");
    assert_eq!(decisions[0]["served"]["target"], "b");
}

#[tokio::test]
async fn an_alias_may_name_a_target_and_every_endpoint_shape_resolves_it() {
    let r = rig("", ALIASES).await;
    assert_eq!(r.chat("alice", "claude-a").await.status(), 200);
    let anthropic = json!({"model": "claude-a", "max_tokens": 5,
        "messages": [{"role": "user", "content": "hi"}]});
    assert_eq!(
        r.ask("alice", "/v1/messages", anthropic.clone())
            .await
            .status(),
        200
    );
    let counted = r.ask("alice", "/v1/messages/count_tokens", anthropic).await;
    assert_eq!(counted.status(), 200);
    assert_eq!(r.models.lock().unwrap().as_slice(), ["ma", "ma"]);
}

#[tokio::test]
async fn real_names_and_unknown_names_behave_as_before() {
    let r = rig("", ALIASES).await;
    assert_eq!(r.chat("alice", "premium").await.status(), 200);
    assert_eq!(r.models.lock().unwrap().as_slice(), ["mb"]);
    assert_eq!(r.chat("alice", "claude-ghost").await.status(), 404);
}

#[tokio::test]
async fn the_allowlist_is_checked_on_the_real_name() {
    let r = rig("", ALIASES).await;
    assert_eq!(r.chat("bob", "claude-plain").await.status(), 200);
    let refused = r.chat("bob", "claude-premium").await;
    assert_eq!(refused.status(), 403);
    let body: Value = refused.json().await.unwrap();
    let message = body["error"]["message"].as_str().unwrap();
    assert!(message.contains("`premium`"), "{message}");
    assert!(!message.contains("claude-premium"), "{message}");
    assert_eq!(r.models.lock().unwrap().as_slice(), ["ma"]);
}

#[tokio::test]
async fn models_lists_the_aliases_a_key_may_use_with_their_labels() {
    let r = rig("", ALIASES).await;
    let alice = r.listed("alice").await;
    assert_eq!(
        ids(&alice),
        [
            "plain",
            "premium",
            "a",
            "b",
            "claude-a",
            "claude-plain",
            "claude-premium"
        ]
    );
    let plain = alice.iter().find(|e| e["id"] == "claude-plain").unwrap();
    assert_eq!(plain["display_name"], "Plain");
    assert_eq!(plain["description"], "The plain pool");
    assert_eq!(plain["object"], "model");
    let bare = alice.iter().find(|e| e["id"] == "claude-a").unwrap();
    assert!(bare.get("display_name").is_none() && bare.get("description").is_none());
    let real = alice.iter().find(|e| e["id"] == "plain").unwrap();
    assert!(real.get("display_name").is_none());

    // Bob may use `plain` only: its alias is listed, the others are hidden like their real names.
    assert_eq!(ids(&r.listed("bob").await), ["plain", "claude-plain"]);
}

#[tokio::test]
async fn selector_rules_match_the_real_name_for_an_alias() {
    let extra = format!(
        "{ALIASES}\n[[select]]\nname = \"to-premium\"\nwhen = {{ model = \"plain\" }}\nroute = \"premium\"\n"
    );
    let r = rig("", &extra).await;
    assert_eq!(r.chat("alice", "claude-plain").await.status(), 200);
    assert_eq!(r.models.lock().unwrap().as_slice(), ["mb"]);
    let decisions = r.decisions("alice").await;
    assert_eq!(decisions[0]["rule"], "to-premium");
    assert_eq!(decisions[0]["requested_model"], "plain");
}

#[tokio::test]
async fn switched_off_aliases_are_neither_served_nor_listed() {
    let r = rig("model_aliases = false", ALIASES).await;
    assert_eq!(r.chat("alice", "claude-plain").await.status(), 404);
    assert!(r.models.lock().unwrap().is_empty());
    assert_eq!(
        ids(&r.listed("alice").await),
        ["plain", "premium", "a", "b"]
    );
    assert_eq!(r.chat("alice", "plain").await.status(), 200);
}
