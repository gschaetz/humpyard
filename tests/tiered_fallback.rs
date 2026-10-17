//! A guarded tail on a pool route (ADR 0024): fixed targets reached only when a listed failure class ends
//! the pool's members. Members still fail over among themselves.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test scaffolding

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use axum::Router;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::post;
use common::{chat_completion, serve, usage};
use humpyard::config::Config;
use humpyard::server;
use serde_json::{Value, json};

fn load(toml: &str) -> Result<Config, String> {
    Config::from_toml(toml, |n| std::env::var(n).ok()).map_err(|e| format!("{e}"))
}

struct Upstream {
    status: u16,
    hits: AtomicU64,
}

async fn completions(State(up): State<Arc<Upstream>>) -> axum::response::Response {
    up.hits.fetch_add(1, Ordering::SeqCst);
    if up.status == 200 {
        axum::Json(chat_completion("m", "ok", Some(usage(10, 5)))).into_response()
    } else {
        (
            StatusCode::from_u16(up.status).unwrap(),
            axum::Json(json!({"error": {"message": "no", "type": "x"}})),
        )
            .into_response()
    }
}

/// An upstream answering with `status`, and the counter of calls it received.
async fn upstream(status: u16) -> (String, Arc<Upstream>) {
    let up = Arc::new(Upstream {
        status,
        hits: AtomicU64::new(0),
    });
    let url = format!(
        "http://{}",
        serve(
            Router::new()
                .route("/chat/completions", post(completions))
                .with_state(up.clone())
        )
        .await
    );
    (url, up)
}

/// Two free pool members (window 4000) answering `member_status`, and a paid tail (window 200000) that always
/// answers; the route's `then` line is `then`.
async fn gateway(member_status: u16, then: &str) -> (String, Arc<Upstream>, Arc<Upstream>) {
    let (m1, u1) = upstream(member_status).await;
    let (m2, _) = upstream(member_status).await;
    let (paid, up) = upstream(200).await;
    let toml = format!(
        r#"listen = "127.0.0.1:0"
[providers.p1]
base_url = "{m1}"
api_key_env = "PATH"
max_retries = 0
[providers.p2]
base_url = "{m2}"
api_key_env = "PATH"
max_retries = 0
[providers.pp]
base_url = "{paid}"
api_key_env = "PATH"
max_retries = 0
[targets.free1]
strengths = ["general"]
endpoints = [{{ provider = "p1", model = "m", context_window = 4000 }}]
[targets.free2]
strengths = ["general"]
endpoints = [{{ provider = "p2", model = "m", context_window = 4000 }}]
[targets.paid]
endpoints = [{{ provider = "pp", model = "m", context_window = 200000 }}]
[routes.free-then-paid]
type = "fastest"
explore = 0.0
pool = {{ strength = "general" }}
{then}
"#
    );
    let config = load(&toml).unwrap();
    let base = format!(
        "http://{}",
        serve(server::router(config).await.unwrap()).await
    );
    (base, u1, up)
}

async fn ask(base: &str, content: &str) -> (u16, String) {
    let resp = reqwest::Client::new()
        .post(format!("{base}/v1/chat/completions"))
        .json(
            &json!({"model": "free-then-paid", "messages": [{"role": "user", "content": content}]}),
        )
        .send()
        .await
        .unwrap();
    let status = resp.status().as_u16();
    let target = resp
        .headers()
        .get("x-humpyard-target")
        .map(|v| v.to_str().unwrap().to_string())
        .unwrap_or_default();
    (status, target)
}

#[tokio::test]
async fn cold_members_are_tried_again_before_a_paid_tail_is_used() {
    // The members fail with a server error, which the guard does not take. After the first failures they are
    // cold; skipping them must not hand the traffic to the paid tail.
    let (m1, u1) = upstream(500).await;
    let (paid, up) = upstream(200).await;
    let toml = format!(
        r#"listen = "127.0.0.1:0"
[health]
failure_threshold = 1
skip_cold_targets = true
[providers.p1]
base_url = "{m1}"
api_key_env = "PATH"
max_retries = 0
[providers.pp]
base_url = "{paid}"
api_key_env = "PATH"
max_retries = 0
[targets.free1]
strengths = ["general"]
endpoints = [{{ provider = "p1", model = "m" }}]
[targets.paid]
endpoints = [{{ provider = "pp", model = "m" }}]
[routes.free-then-paid]
type = "fastest"
explore = 0.0
pool = {{ strength = "general" }}
then = {{ targets = ["paid"] }}
"#
    );
    let base = format!(
        "http://{}",
        serve(server::router(load(&toml).unwrap()).await.unwrap()).await
    );
    for _ in 0..5 {
        assert!(
            ask(&base, "hi").await.0 >= 500,
            "the members' error is the answer"
        );
    }
    assert_eq!(up.hits.load(Ordering::SeqCst), 0, "paid was never used");
    assert_eq!(
        u1.hits.load(Ordering::SeqCst),
        5,
        "the cold member was still tried each time"
    );
}

const THEN: &str = r#"then = { targets = ["paid"] }"#;

#[tokio::test]
async fn a_prompt_too_big_for_every_member_goes_straight_to_the_tail() {
    let (base, free, paid) = gateway(200, THEN).await;
    let big = "word ".repeat(12_000); // far beyond the members' 4000-token windows
    assert_eq!(ask(&base, &big).await, (200, "paid".to_string()));
    assert_eq!(
        free.hits.load(Ordering::SeqCst),
        0,
        "no wasted attempt on a member"
    );
    assert_eq!(paid.hits.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn members_are_used_first_and_the_tail_stays_unused_while_they_answer() {
    let (base, free, paid) = gateway(200, THEN).await;
    for _ in 0..3 {
        let (status, target) = ask(&base, "hi").await;
        assert_eq!(status, 200);
        assert!(target.starts_with("free"), "{target}");
    }
    assert!(free.hits.load(Ordering::SeqCst) >= 1);
    assert_eq!(paid.hits.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn members_still_fail_over_among_themselves_on_a_class_the_tail_does_not_take() {
    // One member rate-limits, the other answers: the guard (overflow only) must not stop member failover.
    let (m1, u1) = upstream(429).await;
    let (m2, u2) = upstream(200).await;
    let (paid, up) = upstream(200).await;
    let toml = format!(
        r#"listen = "127.0.0.1:0"
[providers.p1]
base_url = "{m1}"
api_key_env = "PATH"
max_retries = 0
[providers.p2]
base_url = "{m2}"
api_key_env = "PATH"
max_retries = 0
[providers.pp]
base_url = "{paid}"
api_key_env = "PATH"
max_retries = 0
[targets.free1]
strengths = ["general"]
endpoints = [{{ provider = "p1", model = "m" }}]
[targets.free2]
strengths = ["general"]
endpoints = [{{ provider = "p2", model = "m" }}]
[targets.paid]
endpoints = [{{ provider = "pp", model = "m" }}]
[routes.free-then-paid]
type = "fastest"
explore = 0.0
pool = {{ strength = "general" }}
{THEN}
"#
    );
    let base = format!(
        "http://{}",
        serve(server::router(load(&toml).unwrap()).await.unwrap()).await
    );
    for _ in 0..4 {
        assert_eq!(ask(&base, "hi").await, (200, "free2".to_string()));
    }
    assert!(u1.hits.load(Ordering::SeqCst) >= 1);
    assert!(u2.hits.load(Ordering::SeqCst) >= 1);
    assert_eq!(up.hits.load(Ordering::SeqCst), 0, "the tail was not needed");
}

#[tokio::test]
async fn a_failure_class_the_guard_does_not_list_ends_the_request_without_calling_the_tail() {
    let (base, _free, paid) = gateway(429, THEN).await;
    let (status, _) = ask(&base, "hi").await;
    assert_eq!(status, 429, "the members' rate limit is the answer");
    assert_eq!(
        paid.hits.load(Ordering::SeqCst),
        0,
        "the paid tail was never called"
    );
}

#[tokio::test]
async fn a_listed_failure_class_reaches_the_tail() {
    let (base, _free, paid) = gateway(
        429,
        r#"then = { targets = ["paid"], on = ["rate_limit", "overflow"] }"#,
    )
    .await;
    assert_eq!(ask(&base, "hi").await, (200, "paid".to_string()));
    assert_eq!(paid.hits.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn explain_shows_the_tail_after_the_members_with_its_guard() {
    let (base, _, _) = gateway(200, THEN).await;
    let report: Value = reqwest::Client::new()
        .post(format!("{base}/v1/route/explain"))
        .json(&json!({"model": "free-then-paid"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(report["tail"]["targets"], json!(["paid"]));
    assert_eq!(report["tail"]["on"], json!(["overflow"]));
    let order: Vec<&str> = report["targets"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["target"].as_str().unwrap())
        .collect();
    assert_eq!(order.last(), Some(&"paid"), "{order:?}");
    assert_eq!(order.len(), 3);
}

const HEAD: &str = r#"listen = "127.0.0.1:0"
[providers.p]
base_url = "http://127.0.0.1:1"
api_key_env = "PATH"
[targets.a]
strengths = ["general"]
class = "public"
endpoints = [{ provider = "p", model = "a" }]
[targets.b]
class = "paid"
endpoints = [{ provider = "p", model = "b" }]
"#;

fn route(extra: &str) -> Result<(), String> {
    load(&format!(
        "{HEAD}[routes.r]\ntype = \"fastest\"\npool = {{ strength = \"general\" }}\n{extra}\n"
    ))
    .map(|_| ())
}

#[test]
fn a_tail_is_checked_when_the_configuration_loads() {
    assert!(route(r#"then = { targets = ["b"] }"#).is_ok());
    assert!(route(r#"then = { targets = ["b"], on = ["timeout", "overflow"] }"#).is_ok());
    let unknown = route(r#"then = { targets = ["nope"] }"#).unwrap_err();
    assert!(unknown.contains("unknown target `nope`"), "{unknown}");
    let member = route(r#"then = { targets = ["a"] }"#).unwrap_err();
    assert!(member.contains("also a pool member"), "{member}");
    let empty = route("then = { targets = [] }").unwrap_err();
    assert!(empty.contains("must not be empty"), "{empty}");
    let twice = route(r#"then = { targets = ["b", "b"] }"#).unwrap_err();
    assert!(twice.contains("listed twice"), "{twice}");
    let no_classes = route(r#"then = { targets = ["b"], on = [] }"#).unwrap_err();
    assert!(no_classes.contains("then.on"), "{no_classes}");
}

#[test]
fn a_tail_needs_a_pool_and_a_class_the_route_allows() {
    let no_pool = load(&format!(
        "{HEAD}[routes.r]\ntype = \"fastest\"\ntargets = [\"a\"]\nthen = {{ targets = [\"b\"] }}\n"
    ))
    .unwrap_err();
    assert!(no_pool.contains("only for a pool route"), "{no_pool}");
    let forbidden =
        route("allowed_classes = [\"public\"]\nthen = { targets = [\"b\"] }").unwrap_err();
    assert!(forbidden.contains("not of an allowed class"), "{forbidden}");
    assert!(
        route("allowed_classes = [\"public\", \"paid\"]\nthen = { targets = [\"b\"] }").is_ok()
    );
}
