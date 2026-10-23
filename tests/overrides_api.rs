//! Runtime overrides over HTTP (spec: `gateway-api`, `request-routing`; `add-runtime-overrides` tasks 1.3 and
//! 1.4): who may write, what may be named, and what a `disable_target` or `pin_route` does to a request.
//! and what a `drain_provider` does to the endpoints inside a target (part 2b).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test scaffolding

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicUsize, Ordering};
use std::time::Duration;

use axum::Router;
use axum::extract::State;
use axum::response::IntoResponse;
use axum::routing::post;
use common::{chat_completion, chat_request, serve, usage};
use humpyard::clock::Clock;
use humpyard::config::Config;
use humpyard::server::{self, Options};
use serde_json::{Value, json};

struct TestClock(AtomicI64);
impl Clock for TestClock {
    fn now_ms(&self) -> i64 {
        self.0.load(Ordering::SeqCst)
    }
}

struct Upstream(AtomicUsize, AtomicBool);

async fn completions(State(up): State<Arc<Upstream>>) -> axum::response::Response {
    up.0.fetch_add(1, Ordering::SeqCst);
    if up.1.load(Ordering::SeqCst) {
        return (
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            axum::Json(json!({"error": {"message": "down"}})),
        )
            .into_response();
    }
    axum::Json(chat_completion("m", "ok", Some(usage(10, 5)))).into_response()
}

async fn upstream() -> (Arc<Upstream>, String) {
    let up = Arc::new(Upstream(AtomicUsize::new(0), AtomicBool::new(false)));
    let app = Router::new()
        .route("/chat/completions", post(completions))
        .with_state(up.clone());
    (up, format!("http://{}", serve(app).await))
}

struct Rig {
    url: String,
    ops: String,
    reader: String,
    app: String,
    clock: Arc<TestClock>,
    a: Arc<Upstream>,
    b: Arc<Upstream>,
    _dir: tempfile::TempDir,
}

/// Targets `a` and `b` (private) and `c` (public), each on its own mock upstream. Routes: `duo` (a, b), `lone`
/// (a), `strict` (a, c; private only). Keys: `ops` may manage overrides, `reader` may read the configuration,
/// `app` may do neither. `ledger` and `keys` switch those parts of the configuration off.
async fn rig(ledger: bool, keys: bool) -> Rig {
    let (a, a_url) = upstream().await;
    let (b, b_url) = upstream().await;
    let (_c, c_url) = upstream().await;
    let dir = tempfile::tempdir().unwrap();
    let (ops, ops_hash) = humpyard::auth::generate_key().unwrap();
    let (reader, reader_hash) = humpyard::auth::generate_key().unwrap();
    let (app, app_hash) = humpyard::auth::generate_key().unwrap();
    let ledger_toml = if ledger {
        format!(
            "[ledger]\npath = \"{}\"\n",
            dir.path().join("l.db").display()
        )
    } else {
        String::new()
    };
    let keys_toml = if keys {
        format!(
            "[keys.ops]\nsha256 = \"{ops_hash}\"\nmanage_overrides = true\n\
             [keys.reader]\nsha256 = \"{reader_hash}\"\nread_config = true\n\
             [keys.app]\nsha256 = \"{app_hash}\"\n"
        )
    } else {
        String::new()
    };
    let toml = format!(
        r#"listen = "127.0.0.1:0"
# The test clock sits in 1970; retention would prune each decision as it is written.
decision_retention_days = 0
{ledger_toml}
[overrides]
default_secs = 3600
max_secs = 7200
{keys_toml}
[health]
failure_threshold = 1
cooldown_secs = 60
max_cooldown_secs = 60
[providers.pa]
base_url = "{a_url}"
api_key_env = "K"
max_retries = 0
[providers.pb]
base_url = "{b_url}"
api_key_env = "K"
max_retries = 0
[providers.pc]
base_url = "{c_url}"
api_key_env = "K"
max_retries = 0
[targets.a]
class = "private"
endpoints = [{{ provider = "pa", model = "m" }}]
[targets.b]
class = "private"
endpoints = [{{ provider = "pb", model = "m" }}]
[targets.c]
class = "public"
endpoints = [{{ provider = "pc", model = "m" }}]
[targets.mixed]
class = "private"
endpoints = [{{ provider = "pa", model = "m" }}, {{ provider = "pb", model = "m" }}]
[routes.via_mixed]
type = "passthrough"
targets = ["mixed"]
[routes.duo]
type = "passthrough"
targets = ["a", "b"]
[routes.lone]
type = "passthrough"
targets = ["a"]
[routes.strict]
type = "passthrough"
targets = ["a", "c"]
allowed_classes = ["private"]
"#
    );
    let clock = Arc::new(TestClock(AtomicI64::new(1_000_000)));
    let config = Config::from_toml(&toml, |_| Some("k".into())).unwrap();
    let router = server::router_with_options(
        config,
        Options {
            clock: Some(clock.clone()),
            ..Options::default()
        },
    )
    .await
    .unwrap();
    Rig {
        url: format!("http://{}", serve(router).await),
        ops,
        reader,
        app,
        clock,
        a,
        b,
        _dir: dir,
    }
}

impl Rig {
    async fn create(&self, key: Option<&str>, body: Value) -> (u16, Value) {
        let mut request = reqwest::Client::new().post(format!("{}/v1/overrides", self.url));
        if let Some(key) = key {
            request = request.bearer_auth(key);
        }
        let response = request.json(&body).send().await.unwrap();
        (
            response.status().as_u16(),
            response.json().await.unwrap_or(Value::Null),
        )
    }

    async fn list(&self, key: Option<&str>) -> (u16, Value) {
        let mut request = reqwest::Client::new().get(format!("{}/v1/overrides", self.url));
        if let Some(key) = key {
            request = request.bearer_auth(key);
        }
        let response = request.send().await.unwrap();
        (
            response.status().as_u16(),
            response.json().await.unwrap_or(Value::Null),
        )
    }

    async fn remove(&self, key: Option<&str>, id: i64) -> (u16, Value) {
        let mut request = reqwest::Client::new().delete(format!("{}/v1/overrides/{id}", self.url));
        if let Some(key) = key {
            request = request.bearer_auth(key);
        }
        let response = request.send().await.unwrap();
        (
            response.status().as_u16(),
            response.json().await.unwrap_or(Value::Null),
        )
    }

    /// Status and the serving target (empty when the request failed), as `app` or, on an open gateway, anyone.
    async fn ask(&self, route: &str) -> (u16, String) {
        let mut request = reqwest::Client::new().post(format!("{}/v1/chat/completions", self.url));
        if !self.app.is_empty() {
            request = request.bearer_auth(&self.app);
        }
        let response = request
            .json(&chat_request(route, false))
            .send()
            .await
            .unwrap();
        let target = response
            .headers()
            .get("x-humpyard-target")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
        (response.status().as_u16(), target)
    }

    /// Like `ask`, but the provider that served it.
    async fn ask_provider(&self, route: &str) -> (u16, String) {
        let response = reqwest::Client::new()
            .post(format!("{}/v1/chat/completions", self.url))
            .bearer_auth(&self.app)
            .json(&chat_request(route, false))
            .send()
            .await
            .unwrap();
        let provider = response
            .headers()
            .get("x-humpyard-provider")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
        (response.status().as_u16(), provider)
    }

    async fn explain(&self, route: &str) -> Value {
        reqwest::Client::new()
            .post(format!("{}/v1/route/explain", self.url))
            .bearer_auth(&self.app)
            .json(&json!({"model": route}))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap()
    }

    async fn decision(&self, route: &str) -> Value {
        for _ in 0..100 {
            let body: Value = reqwest::Client::new()
                .get(format!("{}/v1/decisions?route={route}&limit=1", self.url))
                .bearer_auth(&self.app)
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

    fn advance(&self, secs: i64) {
        self.clock.0.fetch_add(secs * 1000, Ordering::SeqCst);
    }
}

fn disable(target: &str) -> Value {
    json!({"kind": "disable_target", "subject": target, "reason": "bad answers"})
}

fn pin(route: &str, target: &str) -> Value {
    json!({"kind": "pin_route", "route": route, "subject": target, "reason": "keep it on one"})
}

#[tokio::test]
async fn a_disabled_target_is_skipped_and_never_called() {
    let rig = rig(true, true).await;
    assert_eq!(rig.ask("duo").await, (200, "a".into()));
    let called = rig.a.0.load(Ordering::SeqCst);

    let (status, created) = rig.create(Some(&rig.ops), disable("a")).await;
    assert_eq!(status, 201, "{created}");
    assert_eq!(created["created_by"], "ops");
    assert_eq!(
        created["expires_ms"].as_i64().unwrap(),
        1_000_000 + 3_600_000,
        "the configured default"
    );

    for _ in 0..3 {
        assert_eq!(rig.ask("duo").await, (200, "b".into()));
    }
    assert_eq!(
        rig.a.0.load(Ordering::SeqCst),
        called,
        "the disabled target is not called"
    );
    assert_eq!(rig.b.0.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn an_override_expires_with_no_other_action() {
    let rig = rig(true, true).await;
    let mut body = disable("a");
    body["expires_in_secs"] = json!(3600);
    assert_eq!(rig.create(Some(&rig.ops), body).await.0, 201);
    assert_eq!(rig.ask("duo").await.1, "b");
    rig.advance(3599);
    assert_eq!(
        rig.ask("duo").await.1,
        "b",
        "still in force a second before"
    );
    rig.advance(2);
    assert_eq!(rig.ask("duo").await.1, "a", "eligible again at the expiry");
    assert_eq!(rig.list(Some(&rig.ops)).await.1["overrides"], json!([]));
}

#[tokio::test]
async fn removing_an_override_takes_effect_for_the_next_request() {
    let rig = rig(true, true).await;
    let id = rig.create(Some(&rig.ops), disable("a")).await.1["id"]
        .as_i64()
        .unwrap();
    assert_eq!(rig.ask("duo").await.1, "b");
    let (status, removed) = rig.remove(Some(&rig.ops), id).await;
    assert_eq!((status, removed["removed"].clone()), (200, json!(true)));
    assert_eq!(rig.ask("duo").await.1, "a");
    let (status, again) = rig.remove(Some(&rig.ops), id).await;
    assert_eq!(status, 404, "{again}");
    assert_eq!(again["error"]["code"], "override_not_found");
}

#[tokio::test]
async fn disabling_the_last_target_refuses_and_removing_it_restores_service() {
    let rig = rig(true, true).await;
    let id = rig.create(Some(&rig.ops), disable("a")).await.1["id"]
        .as_i64()
        .unwrap();
    let before = rig.a.0.load(Ordering::SeqCst);
    let response = reqwest::Client::new()
        .post(format!("{}/v1/chat/completions", rig.url))
        .bearer_auth(&rig.app)
        .json(&chat_request("lone", false))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 503);
    let text = response.text().await.unwrap();
    assert!(
        text.contains("override"),
        "the refusal says an override caused it: {text}"
    );
    assert_eq!(
        rig.a.0.load(Ordering::SeqCst),
        before,
        "refused before any upstream call"
    );
    assert_eq!(rig.decision("lone").await["outcome"], "override_no_target");

    // `explain` plans with the same overrides, so it does not promise what a request would not get.
    assert_eq!(rig.explain("lone").await["outcome"], "override_no_target");

    rig.remove(Some(&rig.ops), id).await;
    assert_eq!(rig.ask("lone").await, (200, "a".into()));
}

#[tokio::test]
async fn a_pin_cuts_a_route_to_its_target_and_pins_add_up() {
    let rig = rig(true, true).await;
    let (_, first) = rig.create(Some(&rig.ops), pin("duo", "b")).await;
    for _ in 0..2 {
        assert_eq!(rig.ask("duo").await.1, "b");
    }
    assert_eq!(rig.a.0.load(Ordering::SeqCst), 0);
    // Another route is untouched.
    assert_eq!(rig.ask("lone").await.1, "a");

    // A second pin on the same route keeps both targets it names.
    rig.create(Some(&rig.ops), pin("duo", "a")).await;
    assert_eq!(
        rig.ask("duo").await.1,
        "a",
        "back in the candidates, first in the route's order"
    );

    rig.remove(Some(&rig.ops), first["id"].as_i64().unwrap())
        .await;
    assert_eq!(rig.ask("duo").await.1, "a");
}

#[tokio::test]
async fn a_pin_never_adds_reach_or_crosses_a_class() {
    let rig = rig(true, true).await;
    let (status, body) = rig.create(Some(&rig.ops), pin("duo", "c")).await;
    assert_eq!(status, 400, "not one of the route's targets: {body}");
    assert_eq!(body["error"]["code"], "invalid_override");
    let (status, body) = rig.create(Some(&rig.ops), pin("strict", "c")).await;
    assert_eq!(
        status, 400,
        "a private-only route cannot be pinned to a public target: {body}"
    );
    assert_eq!(rig.create(Some(&rig.ops), pin("nope", "a")).await.0, 400);
    assert_eq!(
        rig.list(Some(&rig.ops)).await.1["overrides"],
        json!([]),
        "refused ones are not stored"
    );
}

#[tokio::test]
async fn only_a_key_that_opted_in_may_write_and_a_reader_may_list() {
    let rig = rig(true, true).await;
    let (status, body) = rig.create(Some(&rig.app), disable("a")).await;
    assert_eq!(status, 403);
    assert_eq!(body["error"]["code"], "overrides_not_manageable");
    assert_eq!(
        rig.create(Some(&rig.reader), disable("a")).await.0,
        403,
        "reading config is not managing"
    );
    let id = rig.create(Some(&rig.ops), disable("a")).await.1["id"]
        .as_i64()
        .unwrap();

    assert_eq!(rig.remove(Some(&rig.app), id).await.0, 403);
    assert_eq!(rig.remove(Some(&rig.reader), id).await.0, 403);
    assert_eq!(rig.list(Some(&rig.app)).await.0, 403);
    let (status, listed) = rig.list(Some(&rig.reader)).await;
    assert_eq!(status, 200);
    assert_eq!(listed["overrides"][0]["subject"], "a");
    assert_eq!(listed["overrides"][0]["created_by"], "ops");
    assert_eq!(listed["overrides"][0]["stale"], false);
    assert_eq!(
        rig.create(None, disable("a")).await.0,
        401,
        "a key is required when keys are configured"
    );
}

#[tokio::test]
async fn an_override_must_be_bounded_and_name_something_real() {
    let rig = rig(true, true).await;
    for (body, why) in [
        (
            json!({"kind": "disable_target", "subject": "a", "reason": "r", "expires_in_secs": 7201}),
            "beyond the maximum",
        ),
        (
            json!({"kind": "disable_target", "subject": "a", "reason": "r", "expires_in_secs": 0}),
            "no time at all",
        ),
        (
            json!({"kind": "disable_target", "subject": "ghost", "reason": "r"}),
            "unknown target",
        ),
        (
            json!({"kind": "disable_target", "subject": "a", "reason": " "}),
            "no reason",
        ),
        (
            json!({"kind": "freeze", "subject": "a", "reason": "r"}),
            "unknown kind",
        ),
        (
            json!({"kind": "disable_target", "subject": "a", "route": "duo", "reason": "r"}),
            "a route on a disable",
        ),
        (
            json!({"kind": "drain_provider", "subject": "ghost", "reason": "r"}),
            "unknown provider",
        ),
        (
            json!({"kind": "drain_provider", "subject": "pa", "route": "duo", "reason": "r"}),
            "a route on a drain",
        ),
    ] {
        let (status, response) = rig.create(Some(&rig.ops), body).await;
        assert_eq!(status, 400, "{why}: {response}");
    }
    assert_eq!(
        rig.list(Some(&rig.ops)).await.1["overrides"],
        json!([]),
        "nothing was stored"
    );
    let mut longest = disable("a");
    longest["expires_in_secs"] = json!(7200);
    assert_eq!(
        rig.create(Some(&rig.ops), longest).await.0,
        201,
        "the maximum itself is allowed"
    );
}

#[tokio::test]
async fn without_a_ledger_nothing_can_be_created() {
    let rig = rig(false, true).await;
    let (status, body) = rig.create(Some(&rig.ops), disable("a")).await;
    assert_eq!(status, 409, "{body}");
    assert_eq!(body["error"]["code"], "overrides_need_ledger");
    assert_eq!(rig.list(Some(&rig.ops)).await.1["overrides"], json!([]));
    assert_eq!(rig.remove(Some(&rig.ops), 1).await.0, 404);
    assert_eq!(rig.ask("duo").await.1, "a", "routing is unchanged");
}

#[tokio::test]
async fn an_open_gateway_allows_overrides_to_anyone() {
    let mut rig = rig(true, false).await;
    rig.app = String::new();
    let (status, created) = rig.create(None, disable("a")).await;
    assert_eq!(status, 201, "{created}");
    assert_eq!(created["created_by"], "anonymous");
    assert_eq!(rig.ask("duo").await.1, "b");
}

fn drain(provider: &str) -> Value {
    json!({"kind": "drain_provider", "subject": provider, "reason": "provider is down"})
}

#[tokio::test]
async fn a_drained_provider_is_never_called_and_a_mixed_target_serves_through_the_others() {
    let rig = rig(true, true).await;
    assert_eq!(
        rig.ask_provider("via_mixed").await,
        (200, "pa".into()),
        "pa is first"
    );
    let called = rig.a.0.load(Ordering::SeqCst);

    let (status, created) = rig.create(Some(&rig.ops), drain("pa")).await;
    assert_eq!(status, 201, "{created}");
    for _ in 0..3 {
        assert_eq!(rig.ask_provider("via_mixed").await, (200, "pb".into()));
    }
    assert_eq!(
        rig.a.0.load(Ordering::SeqCst),
        called,
        "no endpoint of pa is called"
    );

    // A target whose every endpoint is on pa is left out, and the route moves on.
    let before = rig.a.0.load(Ordering::SeqCst);
    assert_eq!(rig.ask("duo").await, (200, "b".into()));
    assert_eq!(rig.a.0.load(Ordering::SeqCst), before);
}

#[tokio::test]
async fn a_drain_ends_at_its_expiry_and_when_removed() {
    let rig = rig(true, true).await;
    let mut body = drain("pa");
    body["expires_in_secs"] = json!(600);
    rig.create(Some(&rig.ops), body).await;
    assert_eq!(rig.ask_provider("via_mixed").await.1, "pb");
    rig.advance(601);
    assert_eq!(
        rig.ask_provider("via_mixed").await.1,
        "pa",
        "eligible again at the expiry"
    );

    let id = rig.create(Some(&rig.ops), drain("pa")).await.1["id"]
        .as_i64()
        .unwrap();
    assert_eq!(rig.ask_provider("via_mixed").await.1, "pb");
    rig.remove(Some(&rig.ops), id).await;
    assert_eq!(
        rig.ask_provider("via_mixed").await.1,
        "pa",
        "removal takes effect for the next request"
    );
}

#[tokio::test]
async fn draining_the_only_provider_a_route_has_refuses_the_request() {
    let rig = rig(true, true).await;
    let id = rig.create(Some(&rig.ops), drain("pa")).await.1["id"]
        .as_i64()
        .unwrap();
    let before = rig.a.0.load(Ordering::SeqCst);
    assert_eq!(rig.ask("lone").await.0, 503);
    assert_eq!(
        rig.a.0.load(Ordering::SeqCst),
        before,
        "refused before any upstream call"
    );
    assert_eq!(rig.decision("lone").await["outcome"], "override_no_target");
    rig.remove(Some(&rig.ops), id).await;
    assert_eq!(rig.ask("lone").await, (200, "a".into()));
}

#[tokio::test]
async fn health_never_brings_a_drained_endpoint_back() {
    // pa is drained and pb is failing, so the target's only usable endpoint is cooling down. Health fails
    // open and tries it anyway; it must never fall back to the drained pa.
    let rig = rig(true, true).await;
    rig.b.1.store(true, Ordering::SeqCst);
    rig.create(Some(&rig.ops), drain("pa")).await;
    let before = rig.a.0.load(Ordering::SeqCst);
    for _ in 0..3 {
        let (status, _) = rig.ask_provider("via_mixed").await;
        assert!(status >= 500, "pb is down and pa is drained: {status}");
    }
    assert!(
        rig.b.0.load(Ordering::SeqCst) >= 3,
        "the cooling endpoint is still tried"
    );
    assert_eq!(
        rig.a.0.load(Ordering::SeqCst),
        before,
        "the drained endpoint never is"
    );
}
