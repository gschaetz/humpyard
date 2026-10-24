//! The per-request cost ceiling (`max_request_usd`): a target is removed from a request only when the
//! request is certain to cost more than the ceiling on it, and a request nothing can serve within the ceiling
//! is refused with its own code before any upstream call.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test scaffolding

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use axum::Router;
use axum::extract::State;
use axum::response::IntoResponse;
use axum::routing::post;
use common::{chat_completion, serve, usage};
use humpyard::config::Config;
use humpyard::server;
use serde_json::{Value, json};

struct Upstream {
    calls: AtomicUsize,
}

async fn completions(State(up): State<Arc<Upstream>>) -> axum::response::Response {
    up.calls.fetch_add(1, Ordering::SeqCst);
    axum::Json(chat_completion("m", "ok", Some(usage(10, 5)))).into_response()
}

struct Rig {
    url: String,
    capped: String,
    open: String,
    credential: String,
    upstream: Arc<Upstream>,
    _dir: tempfile::TempDir,
}

/// Prices are USD per million tokens. Targets: `dear` (10 in, 30 out), `free`, `mixed` (a dear endpoint with a
/// cheap failover), `tiered` (10 in, 30 out, but a tier at 1,000,000 prompt tokens that costs 0.1 and 0.3).
/// Routes: `ladder` (dear, free), `only-dear`, `only-mixed`, `only-tiered`. Key `capped` has a ceiling of
/// $0.05 and a roomy daily budget; key `open` has no ceiling.
async fn rig() -> Rig {
    let upstream = Arc::new(Upstream {
        calls: AtomicUsize::new(0),
    });
    let app = Router::new()
        .route("/chat/completions", post(completions))
        .with_state(upstream.clone());
    let base = format!("http://{}", serve(app).await);
    let dir = tempfile::tempdir().unwrap();
    let (capped, capped_hash) = humpyard::auth::generate_key().unwrap();
    let (open, open_hash) = humpyard::auth::generate_key().unwrap();
    let (credential, credential_hash) = humpyard::auth::generate_key().unwrap();
    let toml = format!(
        r#"listen = "127.0.0.1:0"
[ledger]
path = "{db}"
[metrics]
enabled = true
bearer_sha256 = "{credential_hash}"
[keys.capped]
sha256 = "{capped_hash}"
max_request_usd = 0.05
daily_usd = 100.0
[keys.open]
sha256 = "{open_hash}"
[providers.p]
base_url = "{base}"
api_key_env = "K"
max_retries = 0
[targets.dear]
endpoints = [{{ provider = "p", model = "m", price = {{ input = 10.0, output = 30.0 }} }}]
[targets.free]
endpoints = [{{ provider = "p", model = "m", price = {{ input = 0.0, output = 0.0 }} }}]
[targets.mixed]
endpoints = [
  {{ provider = "p", model = "m", price = {{ input = 10.0, output = 30.0 }} }},
  {{ provider = "p", model = "m", price = {{ input = 0.1, output = 0.1 }} }},
]
[targets.tiered]
endpoints = [{{ provider = "p", model = "m", price = {{ input = 10.0, output = 30.0, tiers = [{{ above_tokens = 1000000, input = 0.1, output = 0.3 }}] }} }}]
[routes.ladder]
type = "passthrough"
targets = ["dear", "free"]
[routes.only-dear]
type = "passthrough"
targets = ["dear"]
[routes.only-mixed]
type = "passthrough"
targets = ["mixed"]
[routes.only-tiered]
type = "passthrough"
targets = ["tiered"]
"#,
        db = dir.path().join("ledger.db").display(),
    );
    let config = Config::from_toml(&toml, |_| Some("k".into())).unwrap();
    Rig {
        url: format!(
            "http://{}",
            serve(server::router(config).await.unwrap()).await
        ),
        capped,
        open,
        credential,
        upstream,
        _dir: dir,
    }
}

/// A prompt whose low estimate is `tokens` (6 bytes per token).
fn prompt(route: &str, tokens: usize) -> Value {
    json!({"model": route, "messages": [{"role": "user", "content": "a".repeat(tokens * 6)}]})
}

impl Rig {
    /// Status, the serving target (empty on failure) and the JSON body.
    async fn send(&self, key: &str, body: &Value) -> (u16, String, Value) {
        let resp = reqwest::Client::new()
            .post(format!("{}/v1/chat/completions", self.url))
            .bearer_auth(key)
            .json(body)
            .send()
            .await
            .unwrap();
        let status = resp.status().as_u16();
        let target = resp
            .headers()
            .get("x-humpyard-target")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
        (status, target, resp.json().await.unwrap_or(Value::Null))
    }

    async fn explain(&self, key: &str, body: &Value) -> Value {
        reqwest::Client::new()
            .post(format!("{}/v1/route/explain", self.url))
            .bearer_auth(key)
            .json(body)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap()
    }

    async fn decision(&self, key: &str, route: &str) -> Value {
        for _ in 0..100 {
            let body: Value = reqwest::Client::new()
                .get(format!("{}/v1/decisions?route={route}&limit=1", self.url))
                .bearer_auth(key)
                .send()
                .await
                .unwrap()
                .json()
                .await
                .unwrap();
            if let Some(first) = body["data"].as_array().and_then(|d| d.first()) {
                return first.clone();
            }
            tokio::time::sleep(Duration::from_millis(30)).await;
        }
        panic!("no decision for {route}");
    }

    async fn metrics(&self) -> String {
        reqwest::Client::new()
            .get(format!("{}/metrics", self.url))
            .bearer_auth(&self.credential)
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap()
    }

    fn calls(&self) -> usize {
        self.upstream.calls.load(Ordering::SeqCst)
    }
}

#[tokio::test]
async fn a_large_prompt_removes_the_dear_target_and_the_free_one_serves() {
    let rig = rig().await;
    // 10,000 tokens at $10 per million is $0.10 on `dear`, over the $0.05 ceiling.
    let (status, target, _) = rig.send(&rig.capped, &prompt("ladder", 10_000)).await;
    assert_eq!((status, target.as_str()), (200, "free"));
}

#[tokio::test]
async fn a_small_prompt_keeps_the_dear_target() {
    let rig = rig().await;
    let (status, target, _) = rig.send(&rig.capped, &prompt("ladder", 100)).await;
    assert_eq!((status, target.as_str()), (200, "dear"));
}

#[tokio::test]
async fn a_key_without_a_ceiling_routes_exactly_as_before() {
    let rig = rig().await;
    let (status, target, _) = rig.send(&rig.open, &prompt("ladder", 10_000)).await;
    assert_eq!((status, target.as_str()), (200, "dear"));
}

#[tokio::test]
async fn an_unstated_output_length_is_not_assumed_but_a_stated_one_counts() {
    let rig = rig().await;
    // 4,000 prompt tokens cost $0.04 alone: under the ceiling, and no output limit is assumed.
    let (_, target, _) = rig.send(&rig.capped, &prompt("ladder", 4_000)).await;
    assert_eq!(target, "dear");
    // Asking for 1,000 output tokens adds $0.03, so the request is certain to cost $0.07.
    let mut asking = prompt("ladder", 4_000);
    asking["max_tokens"] = json!(1_000);
    let (_, target, _) = rig.send(&rig.capped, &asking).await;
    assert_eq!(target, "free");
}

#[tokio::test]
async fn a_cheap_failover_endpoint_keeps_the_target() {
    let rig = rig().await;
    // The dear endpoint would cost $0.10, the cheap one $0.001: the target can serve within the ceiling.
    let (status, target, _) = rig.send(&rig.capped, &prompt("only-mixed", 10_000)).await;
    assert_eq!((status, target.as_str()), (200, "mixed"));
}

#[tokio::test]
async fn the_lowest_tier_rate_is_used_for_the_bound() {
    let rig = rig().await;
    // The base rate would cost $0.10, but a tier is cheaper, so the request is not certain to exceed.
    let (status, target, _) = rig.send(&rig.capped, &prompt("only-tiered", 10_000)).await;
    assert_eq!((status, target.as_str()), (200, "tiered"));
}

#[tokio::test]
async fn a_request_too_dear_for_every_target_is_refused_before_any_upstream_call() {
    let rig = rig().await;
    let (status, _, body) = rig.send(&rig.capped, &prompt("only-dear", 10_000)).await;
    assert_eq!(status, 402);
    assert_eq!(body["error"]["code"], "request_cost_ceiling");
    let message = body["error"]["message"].as_str().unwrap();
    assert!(
        message.contains("0.100000") && message.contains("0.05"),
        "{message}"
    );
    assert_eq!(rig.calls(), 0);
}

#[tokio::test]
async fn the_decision_records_the_ceiling_and_the_targets_it_removed() {
    let rig = rig().await;
    rig.send(&rig.capped, &prompt("ladder", 10_000)).await;
    let decision = rig.decision(&rig.capped, "ladder").await;
    assert_eq!(
        decision["cost_ceiling"],
        json!({"max_request_micro_usd": 50_000,
               "removed": [{"target": "dear", "lower_bound_micro_usd": 100_000}]})
    );
    assert_eq!(decision["outcome"], "ok");
}

#[tokio::test]
async fn a_refusal_is_recorded_with_its_own_outcome() {
    let rig = rig().await;
    rig.send(&rig.capped, &prompt("only-dear", 10_000)).await;
    let decision = rig.decision(&rig.capped, "only-dear").await;
    assert_eq!(decision["outcome"], "request_cost_ceiling");
    assert_eq!(decision["status"], 402);
    assert_eq!(decision["cost_ceiling"]["removed"][0]["target"], "dear");
}

#[tokio::test]
async fn explain_lists_the_removal_with_the_ceiling_and_the_lower_bound() {
    let rig = rig().await;
    let report = rig
        .explain(
            &rig.capped,
            &json!({"model": "ladder", "prompt_tokens": 10_000}),
        )
        .await;
    assert_eq!(
        report["cost_ceiling"],
        json!({"max_request_micro_usd": 50_000,
               "removed": [{"target": "dear", "lower_bound_micro_usd": 100_000}]})
    );
    // The requested output length counts too.
    let report = rig
        .explain(
            &rig.capped,
            &json!({"model": "ladder", "prompt_tokens": 4_000, "max_output_tokens": 1_000}),
        )
        .await;
    assert_eq!(
        report["cost_ceiling"]["removed"][0]["lower_bound_micro_usd"],
        70_000
    );
    // A key without a ceiling has nothing to report.
    let report = rig
        .explain(
            &rig.open,
            &json!({"model": "ladder", "prompt_tokens": 10_000}),
        )
        .await;
    assert_eq!(report["cost_ceiling"], Value::Null);
}

#[tokio::test]
async fn the_metric_counts_removals_and_refusals() {
    let rig = rig().await;
    rig.send(&rig.capped, &prompt("ladder", 10_000)).await;
    rig.send(&rig.capped, &prompt("only-dear", 10_000)).await;
    // A request the ceiling does not affect counts for neither.
    rig.send(&rig.capped, &prompt("ladder", 100)).await;
    rig.decision(&rig.capped, "ladder").await;
    let mut text = String::new();
    for _ in 0..100 {
        text = rig.metrics().await;
        if text.contains(r#"humpyard_cost_ceiling_total{outcome="refused"#) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(30)).await;
    }
    assert!(
        text.contains(r#"humpyard_cost_ceiling_total{outcome="removed"} 1"#),
        "{text}"
    );
    assert!(
        text.contains(r#"humpyard_cost_ceiling_total{outcome="refused"} 1"#),
        "{text}"
    );
}
