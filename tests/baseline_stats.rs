//! Pricing the same calls at a baseline endpoint (spec: `routing-statistics`, change `add-baseline-cost`): what the
//! admin statistics say the traffic would have cost, what it cost, and what they refuse.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test scaffolding

mod common;

use axum::Router;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::post;
use common::{chat_completion, chat_request, serve, usage};
use humpyard::config::Config;
use humpyard::server;
use serde_json::{Value, json};

/// The model the request names decides the answer: `free-m` and `paid-m` answer with 1,000 prompt and 500 completion
/// tokens, `nousage-m` answers with no usage, and `bad-m` fails.
async fn completions(
    State(()): State<()>,
    axum::Json(body): axum::Json<Value>,
) -> axum::response::Response {
    match body["model"].as_str().unwrap() {
        "bad-m" => (
            StatusCode::INTERNAL_SERVER_ERROR,
            axum::Json(json!({"error": {"message": "no"}})),
        )
            .into_response(),
        "nousage-m" => axum::Json(chat_completion("m", "ok", None)).into_response(),
        _ => axum::Json(chat_completion("m", "ok", Some(usage(1_000, 500)))).into_response(),
    }
}

struct Rig {
    url: String,
    client_key: String,
    admin_key: String,
    _dir: tempfile::TempDir,
}

/// Targets: `free` (price 0), `paid` ($1 input, $2 output per million), `nousage`, `bad`, and baselines `big` ($10
/// and $30), `nameless` (no price) and `tiered`, none of which a route uses. One call at `big` costs 1,000 x 10 plus
/// 500 x 30 = 25,000 micro-USD; one at `paid`, 2,000.
async fn rig() -> Rig {
    let upstream = format!(
        "http://{}",
        serve(
            Router::new()
                .route("/chat/completions", post(completions))
                .with_state(())
        )
        .await
    );
    let (client_key, client_hash) = humpyard::auth::generate_key().unwrap();
    let (admin_key, admin_hash) = humpyard::auth::generate_key().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let toml = format!(
        r#"listen = "127.0.0.1:0"
[ledger]
path = "{}"
[keys.k]
sha256 = "{client_hash}"
[admin.credentials.root]
sha256 = "{admin_hash}"
scopes = ["read"]
[providers.p]
base_url = "{upstream}"
api_key_env = "PATH"
max_retries = 0
[targets.free]
endpoints = [{{ provider = "p", model = "free-m", price = {{ input = 0.0, output = 0.0 }} }}]
[targets.paid]
endpoints = [{{ provider = "p", model = "paid-m", price = {{ input = 1.0, output = 2.0 }} }}]
[targets.nousage]
endpoints = [{{ provider = "p", model = "nousage-m", price = {{ input = 1.0, output = 2.0 }} }}]
[targets.bad]
endpoints = [{{ provider = "p", model = "bad-m", price = {{ input = 1.0, output = 2.0 }} }}]
[targets.big]
endpoints = [{{ provider = "p", model = "big", price = {{ input = 10.0, output = 30.0 }} }}]
[targets.medium]
endpoints = [{{ provider = "p", model = "medium", price = {{ input = 5.0, output = 15.0 }} }}]
[targets.nameless]
endpoints = [{{ provider = "p", model = "nameless" }}]
[targets.tiered]
endpoints = [{{ provider = "p", model = "tiered", price = {{ input = 1.0, output = 2.0, tiers = [{{ above_tokens = 100000, input = 3.0, output = 6.0 }}] }} }}]
[routes.free-route]
type = "passthrough"
targets = ["free"]
[routes.paid-route]
type = "passthrough"
targets = ["paid"]
[routes.nousage-route]
type = "passthrough"
targets = ["nousage"]
[routes.bad-route]
type = "passthrough"
targets = ["bad"]
"#,
        dir.path().join("ledger.db").display()
    );
    let config = Config::from_toml(&toml, |n| std::env::var(n).ok()).unwrap();
    let url = format!(
        "http://{}",
        serve(server::router(config).await.unwrap()).await
    );
    Rig {
        url,
        client_key,
        admin_key,
        _dir: dir,
    }
}

impl Rig {
    async fn send(&self, route: &str, times: usize) {
        for _ in 0..times {
            reqwest::Client::new()
                .post(format!("{}/v1/chat/completions", self.url))
                .bearer_auth(&self.client_key)
                .json(&chat_request(route, false))
                .send()
                .await
                .unwrap();
        }
    }

    /// Three free calls, two paid ones, one with no usage and one that fails.
    async fn traffic(&self) {
        self.send("free-route", 3).await;
        self.send("paid-route", 2).await;
        self.send("nousage-route", 1).await;
        self.send("bad-route", 1).await;
    }

    async fn stats(&self, query: &str) -> (u16, Value) {
        let response = reqwest::Client::new()
            .get(format!("{}/admin/v1/stats?{query}", self.url))
            .bearer_auth(&self.admin_key)
            .send()
            .await
            .unwrap();
        (
            response.status().as_u16(),
            response.json().await.unwrap_or(Value::Null),
        )
    }
}

fn group<'a>(answer: &'a Value, name: &str) -> &'a Value {
    answer["data"]
        .as_array()
        .unwrap()
        .iter()
        .find(|g| g["group"] == name)
        .unwrap_or_else(|| panic!("no group {name} in {answer}"))
}

fn close(value: &Value, expected: f64) {
    let got = value
        .as_f64()
        .unwrap_or_else(|| panic!("not a number: {value}"));
    assert!((got - expected).abs() < 1e-9, "{got} is not {expected}");
}

#[tokio::test]
async fn free_traffic_against_a_paid_baseline_shows_what_it_would_have_cost() {
    let rig = rig().await;
    rig.traffic().await;
    let (status, answer) = rig.stats("group_by=route&baseline=p/big").await;
    assert_eq!(status, 200, "{answer}");
    let free = &group(&answer, "free-route")["vs_baseline"];
    // Three calls of 25,000 micro-USD each at the baseline, nothing actually paid: the whole of it saved.
    close(&free["baseline_cost_usd"], 0.075);
    close(&free["cost_usd"], 0.0);
    close(&free["saved_usd"], 0.075);
    close(&free["saved_ratio"], 1.0);
    assert_eq!(free["calls_priced"], 3);
}

#[tokio::test]
async fn partly_paid_traffic_saves_the_baseline_cost_minus_what_was_paid() {
    let rig = rig().await;
    rig.traffic().await;
    let (_, answer) = rig.stats("group_by=route&baseline=p/big").await;
    let paid = &group(&answer, "paid-route")["vs_baseline"];
    // Two calls: 50,000 micro-USD at the baseline, 4,000 actually (1,000 x 1 + 500 x 2 each).
    close(&paid["baseline_cost_usd"], 0.05);
    close(&paid["cost_usd"], 0.004);
    close(&paid["saved_usd"], 0.046);
    close(&paid["saved_ratio"], 0.92);
    let summary = &answer["summary"];
    close(&summary["baseline_cost_usd"], 0.125);
    close(&summary["cost_usd"], 0.004);
    close(&summary["saved_usd"], 0.121);
    close(&summary["saved_ratio"], 0.968);
    assert_eq!(
        (&summary["calls_priced"], &summary["calls_left_out"]),
        (&json!(5), &json!(1))
    );
}

#[tokio::test]
async fn a_group_with_no_priced_calls_has_no_ratio_and_a_call_without_usage_is_left_out() {
    let rig = rig().await;
    rig.traffic().await;
    let (_, answer) = rig.stats("group_by=route&baseline=p/big").await;
    let failing = &group(&answer, "bad-route")["vs_baseline"];
    close(&failing["baseline_cost_usd"], 0.0);
    close(&failing["cost_usd"], 0.0);
    assert_eq!(failing["saved_ratio"], Value::Null);
    assert_eq!(
        (&failing["calls_priced"], &failing["calls_left_out"]),
        (&json!(0), &json!(0))
    );
    // The call whose provider reported no usage is in neither cost, and is counted as left out.
    let silent = &group(&answer, "nousage-route")["vs_baseline"];
    close(&silent["baseline_cost_usd"], 0.0);
    close(&silent["cost_usd"], 0.0);
    assert_eq!(
        (&silent["calls_priced"], &silent["calls_left_out"]),
        (&json!(0), &json!(1))
    );
}

#[tokio::test]
async fn every_grouping_adds_up_to_the_same_summary() {
    let rig = rig().await;
    rig.traffic().await;
    for group_by in ["route", "target", "endpoint", "actor", "owner"] {
        let (status, answer) = rig
            .stats(&format!("group_by={group_by}&baseline=p/big"))
            .await;
        assert_eq!(status, 200, "{group_by}: {answer}");
        let groups = answer["data"].as_array().unwrap();
        let sum = |field: &str| {
            groups
                .iter()
                .map(|g| g["vs_baseline"][field].as_f64().unwrap())
                .sum::<f64>()
        };
        close(
            &answer["summary"]["baseline_cost_usd"],
            sum("baseline_cost_usd"),
        );
        close(&answer["summary"]["cost_usd"], sum("cost_usd"));
        close(&answer["summary"]["baseline_cost_usd"], 0.125);
        close(&answer["summary"]["cost_usd"], 0.004);
    }
}

#[tokio::test]
async fn the_answer_names_its_baseline_its_prices_and_what_the_figure_is() {
    let rig = rig().await;
    rig.traffic().await;
    let (_, answer) = rig.stats("baseline=p/big").await;
    let baseline = &answer["baseline"];
    assert_eq!(baseline["endpoint"], "p/big");
    close(&baseline["input_per_million_usd"], 10.0);
    close(&baseline["cached_input_per_million_usd"], 10.0);
    close(&baseline["output_per_million_usd"], 30.0);
    let basis = baseline["basis"].as_str().unwrap();
    assert!(
        basis.contains("estimate") && basis.contains("tokens") && basis.contains("as well"),
        "{basis}"
    );
}

#[tokio::test]
async fn without_a_baseline_the_answer_is_what_it_was() {
    let rig = rig().await;
    rig.traffic().await;
    let (status, answer) = rig.stats("group_by=route").await;
    assert_eq!(status, 200);
    assert!(
        answer.get("baseline").is_none() && answer.get("summary").is_none(),
        "{answer}"
    );
    for g in answer["data"].as_array().unwrap() {
        assert!(g.get("vs_baseline").is_none(), "{g}");
    }
}

#[tokio::test]
async fn a_baseline_that_cannot_be_priced_is_refused_naming_why() {
    let rig = rig().await;
    rig.traffic().await;
    for (baseline, why) in [
        ("p/nope", "is not one"),
        ("big", "is not one"),
        ("", "is not one"),
        ("p/nameless", "no price"),
        ("p/tiered", "tiered"),
    ] {
        let (status, body) = rig.stats(&format!("baseline={baseline}")).await;
        assert_eq!(status, 400, "{baseline}: {body}");
        let message = body["error"]["message"].as_str().unwrap();
        assert!(message.contains(why), "{baseline}: {message}");
    }
}

#[tokio::test]
async fn a_hostile_baseline_is_only_ever_text_in_a_refusal() {
    let rig = rig().await;
    rig.traffic().await;
    let hostile = "p/big'; DROP TABLE usage; --<script>";
    let encoded: String = hostile
        .bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() {
                char::from(b).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect();
    let response = reqwest::Client::new()
        .get(format!("{}/admin/v1/stats?baseline={encoded}", rig.url))
        .bearer_auth(&rig.admin_key)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 400);
    let body: Value = response.json().await.unwrap();
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap()
            .contains("is not one")
    );
    // The ledger is untouched: a normal read still sees the traffic.
    let (status, answer) = rig.stats("baseline=p/big").await;
    assert_eq!(status, 200);
    close(&answer["summary"]["baseline_cost_usd"], 0.125);
}

#[tokio::test]
async fn the_client_statistics_do_not_accept_a_baseline() {
    let rig = rig().await;
    rig.traffic().await;
    let response = reqwest::Client::new()
        .get(format!("{}/v1/stats?baseline=p/big", rig.url))
        .bearer_auth(&rig.client_key)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 400);
    let body: Value = response.json().await.unwrap();
    let message = body.to_string();
    assert!(
        message.contains("unknown parameter `baseline`"),
        "{message}"
    );
    // And the endpoint's price is nowhere in it.
    assert!(!message.contains("30"), "{message}");
}

#[tokio::test]
async fn two_baselines_do_not_share_a_cached_result() {
    let rig = rig().await;
    rig.traffic().await;
    let (_, big) = rig.stats("baseline=p/big").await;
    let (_, medium) = rig.stats("baseline=p/medium").await;
    assert_eq!(big["baseline"]["endpoint"], "p/big");
    assert_eq!(medium["baseline"]["endpoint"], "p/medium");
    // $5 and $15 against $10 and $30: half the baseline cost.
    close(&medium["summary"]["baseline_cost_usd"], 0.0625);
    close(&big["summary"]["baseline_cost_usd"], 0.125);
}
