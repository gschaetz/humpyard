//! `POST /v1/route/explain`: a dry run that reports which rule applies and why, using the same
//! selection, budget and policy code as real requests (so the two always agree).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test scaffolding

mod common;

use std::collections::HashMap;
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

/// (key, requested model, request headers, sub-agent flag)
type Case<'a> = (&'a str, &'a str, Vec<(&'a str, &'a str)>, bool);

struct Rig {
    url: String,
    calls: Arc<AtomicUsize>,
    keys: HashMap<&'static str, String>,
    db: std::path::PathBuf,
    _dir: tempfile::TempDir,
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
name = "openclaw"
when = { header = { "x-client" = "open*" } }
route = "cheap-first"
"#;

async fn rig() -> Rig {
    let (calls, url_a) = upstream().await;
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("ledger.db");
    let mut keys = HashMap::new();
    let mut blocks = Vec::new();
    for (id, extra) in [
        ("full", ""),
        ("limited", "allowed_routes = [\"plain\", \"cheap-first\"]"),
        ("tight", "daily_tokens = 1"),
    ] {
        let (key, hash) = humpyard::auth::generate_key().unwrap();
        blocks.push(format!("[keys.{id}]\nsha256 = \"{hash}\"\n{extra}"));
        keys.insert(id, key);
    }
    let toml = format!(
        r#"listen = "127.0.0.1:0"
[ledger]
path = "{db}"
{keys}
[providers.a]
base_url = "{url_a}"
api_key_env = "K"
[targets.cheap]
endpoints = [{{ provider = "a", model = "small" }}]
[targets.premium]
endpoints = [{{ provider = "a", model = "big" }}]
[routes.plain]
type = "passthrough"
targets = ["cheap"]
[routes.cheap-first]
type = "passthrough"
targets = ["cheap", "premium"]
fallback_on = ["overflow", "rate_limit"]
[routes.premium-only]
type = "passthrough"
targets = ["premium"]
{RULES}
"#,
        db = db.display(),
        keys = blocks.join("\n")
    );
    let config = Config::from_toml(&toml, |_| Some("secret".into())).unwrap();
    Rig {
        url: format!(
            "http://{}",
            serve(server::router(config).await.unwrap()).await
        ),
        calls,
        keys,
        db,
        _dir: dir,
    }
}

impl Rig {
    async fn explain(&self, key: &str, body: Value) -> Value {
        let resp = reqwest::Client::new()
            .post(format!("{}/v1/route/explain", self.url))
            .bearer_auth(&self.keys[key])
            .json(&body)
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);
        resp.json().await.unwrap()
    }

    async fn real(&self, key: &str, model: &str, headers: &[(&str, &str)]) -> reqwest::Response {
        let mut req = reqwest::Client::new()
            .post(format!("{}/v1/chat/completions", self.url))
            .bearer_auth(&self.keys[key])
            .json(&chat_request(model, false));
        for (k, v) in headers {
            req = req.header(*k, *v);
        }
        req.send().await.unwrap()
    }
}

#[tokio::test]
async fn it_reports_the_chosen_rule_the_route_the_targets_and_why_other_rules_did_not_apply() {
    let r = rig().await;
    let report = r
        .explain(
            "full",
            json!({"model": "plain", "headers": {"X-Humpyard-Profile": "deep"}}),
        )
        .await;
    assert_eq!(report["outcome"], "ok", "{report}");
    assert_eq!(report["key"], "full");
    assert_eq!(
        report["selected"],
        json!({"route": "premium-only", "rule": "deep", "source": "selector"})
    );
    assert_eq!(report["budget"], "healthy");
    assert_eq!(report["fallback_on"], "all");
    assert_eq!(report["targets"][0]["target"], "premium");
    assert_eq!(report["targets"][0]["endpoints"][0]["state"], "healthy");

    let rules = report["rules"].as_array().unwrap();
    assert_eq!(rules.len(), 3);
    assert_eq!(rules[0]["rule"], "subagents");
    assert_eq!(rules[0]["matched"], false);
    assert_eq!(
        rules[0]["mismatches"],
        json!(["subagent: wanted true, got false"])
    );
    assert_eq!(rules[1]["matched"], true);
    assert_eq!(rules[1]["key_may_use_route"], true);
    assert_eq!(
        rules[2]["mismatches"],
        json!(["header x-client: wanted `open*`, but the request has none"])
    );
}

#[tokio::test]
async fn it_shows_when_a_matching_rule_is_skipped_because_the_key_may_not_use_its_route() {
    let r = rig().await;
    let report = r
        .explain(
            "limited",
            json!({"model": "plain", "headers": {"x-humpyard-profile": "deep"}}),
        )
        .await;
    assert_eq!(
        report["selected"],
        json!({"route": "plain", "rule": "default", "source": "requested_model"})
    );
    assert_eq!(report["rules"][1]["matched"], true);
    assert_eq!(report["rules"][1]["key_may_use_route"], false);
    assert_eq!(report["outcome"], "ok");
}

#[tokio::test]
async fn it_names_the_reason_a_request_would_be_refused() {
    let r = rig().await;
    let forbidden = r.explain("limited", json!({"model": "premium-only"})).await;
    assert_eq!(forbidden["outcome"], "forbidden", "{forbidden}");
    assert!(
        forbidden["message"]
            .as_str()
            .unwrap()
            .contains("premium-only")
    );

    let unknown = r.explain("full", json!({"model": "gpt-4o"})).await;
    assert_eq!(unknown["outcome"], "unknown_model");
    // ...unless a rule rescues the unknown name.
    let rescued = r
        .explain(
            "full",
            json!({"model": "gpt-4o", "headers": {"x-client": "openclaw"}}),
        )
        .await;
    assert_eq!(rescued["outcome"], "ok");
    assert_eq!(rescued["selected"]["route"], "cheap-first");
    assert_eq!(rescued["fallback_on"], json!(["overflow", "rate_limit"]));
    assert_eq!(rescued["targets"].as_array().unwrap().len(), 2);

    // Spend the "tight" key's single token budget with one real call, then ask again.
    assert_eq!(r.real("tight", "plain", &[]).await.status(), 200);
    tokio::time::sleep(Duration::from_millis(300)).await;
    let blocked = r.explain("tight", json!({"model": "plain"})).await;
    assert_eq!(blocked["outcome"], "budget_exhausted", "{blocked}");
    assert_eq!(
        r.real("tight", "plain", &[]).await.status(),
        402,
        "a real request agrees"
    );
}

#[tokio::test]
async fn it_never_calls_a_provider_or_writes_the_ledger() {
    let r = rig().await;
    for _ in 0..5 {
        r.explain(
            "full",
            json!({"model": "plain", "stream": true, "subagent": true}),
        )
        .await;
    }
    assert_eq!(r.calls.load(Ordering::SeqCst), 0);
    tokio::time::sleep(Duration::from_millis(300)).await;
    let rows = Ledger::open(&r.db)
        .await
        .unwrap()
        .entries_since(0)
        .await
        .unwrap();
    assert_eq!(rows.len(), 0, "{rows:?}");
}

#[tokio::test]
async fn its_answer_always_agrees_with_what_a_real_request_does() {
    let r = rig().await;
    let cases: [Case<'_>; 7] = [
        ("full", "plain", vec![], false),
        ("full", "plain", vec![("x-humpyard-profile", "deep")], false),
        ("full", "plain", vec![("x-client", "openclaw")], false),
        (
            "full",
            "plain",
            vec![("x-switchyard-is-subagent", "true")],
            true,
        ),
        (
            "limited",
            "plain",
            vec![("x-humpyard-profile", "deep")],
            false,
        ),
        (
            "limited",
            "plain",
            vec![("x-humpyard-profile", "deep"), ("x-client", "openclaw")],
            false,
        ),
        (
            "full",
            "premium-only",
            vec![("x-client", "openclaw")],
            false,
        ),
    ];
    for (key, model, headers, subagent) in cases {
        let hypothetical_headers: HashMap<&str, &str> = headers.iter().copied().collect();
        let report = r
            .explain(
                key,
                json!({"model": model, "headers": hypothetical_headers, "subagent": subagent}),
            )
            .await;
        let real = r.real(key, model, &headers).await;
        assert_eq!(real.status(), 200, "{key} {model} {headers:?}");
        assert_eq!(
            real.headers()["x-humpyard-route"],
            report["selected"]["route"].as_str().unwrap(),
            "{key} {model} {headers:?}: {report}"
        );
        assert_eq!(
            real.headers()["x-humpyard-rule"],
            report["selected"]["rule"].as_str().unwrap()
        );
    }
}

#[tokio::test]
async fn it_needs_a_key_ignores_credential_headers_and_rejects_bad_bodies() {
    let r = rig().await;
    let anonymous = reqwest::Client::new()
        .post(format!("{}/v1/route/explain", r.url))
        .json(&json!({"model": "plain"}))
        .send()
        .await
        .unwrap();
    assert_eq!(anonymous.status(), 401);

    // A credential header in the hypothetical request is ignored, never matched or echoed.
    let report = r
        .explain(
            "full",
            json!({"model": "plain", "headers": {"authorization": "Bearer sk-x"}}),
        )
        .await;
    assert_eq!(report["selected"]["rule"], "default");
    assert!(!report.to_string().contains("sk-x"));

    for body in [
        json!({}),
        json!({"model": "plain", "bogus": 1}),
        json!("text"),
    ] {
        let resp = reqwest::Client::new()
            .post(format!("{}/v1/route/explain", r.url))
            .bearer_auth(&r.keys["full"])
            .json(&body)
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 400, "{body}");
    }
}
