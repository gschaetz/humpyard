//! Unavailable endpoints (ADR 0023): a model that is gone (404), a rejected key (401) or a missing-model
//! code counts against its endpoint and the request moves on, by default on `fastest` and pool routes only.
//! Dead targets cool, are skipped, and are probed again. A request a provider rejects (400, 422) moves a pool
//! on without counting against the endpoint, within a bound (ADR 0026); other routes still end the request.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test scaffolding

mod common;

use std::fmt::Write as _;
use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use common::{chat_completion, chat_request, serve, usage};
use humpyard::clock::{Clock, SystemClock};
use humpyard::config::Config;
use humpyard::server::{self, Options};
use serde_json::{Value, json};

struct TestClock(AtomicI64);
impl Clock for TestClock {
    fn now_ms(&self) -> i64 {
        self.0.load(Ordering::SeqCst)
    }
}

/// A mock upstream: a status, an error body, and a count of the calls it received.
struct Upstream {
    status: Mutex<(u16, Value)>,
    calls: AtomicUsize,
}

impl Upstream {
    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

async fn completions(State(up): State<Arc<Upstream>>) -> Response {
    up.calls.fetch_add(1, Ordering::SeqCst);
    let (status, body) = up.status.lock().unwrap().clone();
    if status == 200 {
        return axum::Json(chat_completion("m", "ok", Some(usage(10, 5)))).into_response();
    }
    (StatusCode::from_u16(status).unwrap(), axum::Json(body)).into_response()
}

async fn upstream(status: u16, body: Value) -> (Arc<Upstream>, String) {
    let up = Arc::new(Upstream {
        status: Mutex::new((status, body)),
        calls: AtomicUsize::new(0),
    });
    let app = Router::new()
        .route("/chat/completions", post(completions))
        .with_state(up.clone());
    (up, format!("http://{}", serve(app).await))
}

fn not_found() -> Value {
    json!({"error": {"message": "The model `x` does not exist", "code": "model_not_found"}})
}

fn plain() -> Value {
    json!({"error": {"message": "nope"}})
}

struct Rig {
    url: String,
    clock: Arc<TestClock>,
    dead: Arc<Upstream>,
    second_dead: Arc<Upstream>,
    live: Arc<Upstream>,
}

/// Targets `a-dead` (private, strengths general), `b-live` (public, general) and `c-live-twin`, plus
/// `multi` (two endpoints: one dead, one live). `routes` is appended to the config.
async fn rig(dead_status: u16, dead_body: Value, routes: &str) -> Rig {
    let (dead, dead_url) = upstream(dead_status, dead_body.clone()).await;
    let (second_dead, second_dead_url) = upstream(dead_status, dead_body).await;
    let (live, live_url) = upstream(200, json!({})).await;
    let toml = format!(
        r#"listen = "127.0.0.1:0"
[health]
failure_threshold = 2
cooldown_secs = 10
max_cooldown_secs = 10
[providers.pd]
base_url = "{dead_url}"
api_key_env = "PATH"
max_retries = 0
[providers.pd2]
base_url = "{second_dead_url}"
api_key_env = "PATH"
max_retries = 0
[providers.pl]
base_url = "{live_url}"
api_key_env = "PATH"
max_retries = 0
[targets.a-dead]
class = "private"
strengths = ["general"]
endpoints = [{{ provider = "pd", model = "m" }}]
[targets.b-live]
class = "public"
strengths = ["general"]
endpoints = [{{ provider = "pl", model = "m" }}]
[targets.c-dead]
class = "public"
strengths = ["general"]
endpoints = [{{ provider = "pd2", model = "m" }}]
[targets.multi]
endpoints = [{{ provider = "pd", model = "m2" }}, {{ provider = "pl", model = "m2" }}]
{routes}
"#
    );
    let clock = Arc::new(TestClock(AtomicI64::new(SystemClock.now_ms())));
    let config = Config::from_toml(&toml, |n| std::env::var(n).ok()).unwrap();
    let app = server::router_with_options(
        config,
        Options {
            clock: Some(clock.clone()),
            ..Options::default()
        },
    )
    .await
    .unwrap();
    Rig {
        url: format!("http://{}", serve(app).await),
        clock,
        dead,
        second_dead,
        live,
    }
}

impl Rig {
    async fn ask(&self, route: &str) -> (u16, String) {
        let resp = reqwest::Client::new()
            .post(format!("{}/v1/chat/completions", self.url))
            .json(&chat_request(route, false))
            .send()
            .await
            .unwrap();
        let target = resp
            .headers()
            .get("x-humpyard-target")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
        (resp.status().as_u16(), target)
    }

    async fn state_of(&self, target: &str) -> String {
        let health: Value = reqwest::Client::new()
            .get(format!("{}/v1/health", self.url))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        health["targets"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["target"] == target)
            .unwrap()["endpoints"][0]["state"]
            .as_str()
            .unwrap()
            .to_string()
    }
}

const FASTEST: &str =
    "[routes.fast]\ntype = \"fastest\"\ntargets = [\"a-dead\", \"b-live\"]\nexplore = 0.0\n";
const POOL: &str =
    "[routes.pool]\ntype = \"fastest\"\npool = { strength = \"general\" }\nexplore = 0.0\n";
const PASS: &str = "[routes.pass]\ntype = \"passthrough\"\ntargets = [\"a-dead\", \"b-live\"]\n";
const PASS_POOL: &str =
    "[routes.passpool]\ntype = \"passthrough\"\npool = { strength = \"general\" }\n";
const PASS_OPT_IN: &str = "[routes.optin]\ntype = \"passthrough\"\ntargets = [\"a-dead\", \"b-live\"]\nfallback_on = [\"unavailable\"]\n";
const FAST_EXACT: &str = "[routes.exact]\ntype = \"fastest\"\ntargets = [\"a-dead\", \"b-live\"]\nexplore = 0.0\nfallback_on = [\"rate_limit\"]\n";

#[tokio::test]
async fn a_dead_first_target_no_longer_fails_a_fastest_route_or_a_pool() {
    for (status, body) in [
        (404, plain()),
        (401, plain()),
        (400, not_found()),
        (404, not_found()),
        // A permanently removed model, and a key with no credit left (found with a live free pool).
        (410, plain()),
        (402, plain()),
    ] {
        let rig = rig(status, body.clone(), &format!("{FASTEST}{POOL}")).await;
        for route in ["fast", "pool"] {
            for _ in 0..4 {
                let (code, target) = rig.ask(route).await;
                assert_eq!(
                    (code, target.as_str()),
                    (200, "b-live"),
                    "{route} with a first target answering {status} {body}"
                );
            }
        }
    }
}

#[tokio::test]
async fn a_rejected_request_ends_the_request_on_routes_that_do_not_opt_in() {
    for (status, body) in [(400, plain()), (422, plain())] {
        let rig = rig(status, body, &format!("{PASS}{FAST_EXACT}")).await;
        for route in ["pass", "exact"] {
            let (code, _) = rig.ask(route).await;
            assert_eq!(
                code, status,
                "{route}: a rejection is not an endpoint failure"
            );
        }
        assert_eq!(rig.live.calls(), 0, "the next target is not asked");
        for _ in 0..4 {
            rig.ask("pass").await;
        }
        assert_eq!(
            rig.state_of("a-dead").await,
            "healthy",
            "and it does not cool the endpoint"
        );
    }
}

/// A 400 or 422 says this provider rejects the request, not that it is unwell (ADR 0026): on a `fastest`
/// route and a pool the request moves on, and the endpoint is asked again every time, never counted as failing.
#[tokio::test]
async fn a_rejected_request_moves_a_pool_on_without_cooling_the_endpoint() {
    for status in [400u16, 422] {
        let rig = rig(status, plain(), &format!("{FASTEST}{POOL}")).await;
        for _ in 0..5 {
            assert_eq!(
                rig.ask("fast").await,
                (200, "b-live".to_string()),
                "{status}"
            );
        }
        assert_eq!(
            rig.dead.calls(),
            5,
            "it is asked every time, never skipped as cold"
        );
        assert_ne!(rig.state_of("a-dead").await, "cooling_down");
        assert_eq!(
            rig.ask("pool").await.0,
            200,
            "{status} on the pool route too"
        );
    }
}

#[tokio::test]
async fn a_route_can_opt_in_to_rejected() {
    let routes = "[routes.optin]\ntype = \"passthrough\"\ntargets = [\"a-dead\", \"b-live\"]\nfallback_on = [\"rejected\"]\n";
    let rig = rig(422, plain(), routes).await;
    assert_eq!(rig.ask("optin").await, (200, "b-live".to_string()));
}

#[tokio::test]
async fn a_rejection_moves_on_within_a_target_only_on_routes_that_allow_it() {
    let one = "[routes.one]\ntype = \"passthrough\"\ntargets = [\"multi\"]\n";
    let allow = "[routes.allow]\ntype = \"passthrough\"\ntargets = [\"multi\"]\nfallback_on = [\"rejected\"]\n";
    let rig = rig(400, plain(), &format!("{one}{allow}")).await;
    assert_eq!(
        rig.ask("one").await.0,
        400,
        "a plain route ends the request"
    );
    assert_eq!(
        rig.ask("allow").await.0,
        200,
        "an opted-in route tries the second endpoint"
    );
    assert_ne!(rig.state_of("a-dead").await, "cooling_down");
}

#[tokio::test]
async fn a_model_gone_400_is_still_unavailable() {
    let rig = rig(400, not_found(), FASTEST).await;
    for _ in 0..4 {
        assert_eq!(rig.ask("fast").await.0, 200);
    }
    assert_eq!(
        rig.state_of("a-dead").await,
        "cooling_down",
        "a missing model counts against the endpoint, unlike a rejection"
    );
}

#[tokio::test]
async fn other_routes_keep_their_behavior_unless_they_opt_in() {
    for status in [404u16, 401] {
        let rig = rig(status, plain(), &format!("{PASS}{PASS_OPT_IN}{FAST_EXACT}")).await;
        // A plain passthrough route returns the error and does not call the second target.
        let (code, _) = rig.ask("pass").await;
        assert_eq!(code, status);
        assert_eq!(rig.live.calls(), 0);
        // An explicit list is taken exactly as written, even on a fastest route.
        let (code, _) = rig.ask("exact").await;
        assert_eq!(code, status);
        assert_eq!(rig.live.calls(), 0);
        // Opting in moves on.
        let (code, target) = rig.ask("optin").await;
        assert_eq!((code, target.as_str()), (200, "b-live"));
    }
}

#[tokio::test]
async fn a_pool_of_any_route_type_moves_on_by_default() {
    let rig = rig(404, plain(), PASS_POOL).await;
    for _ in 0..3 {
        let (code, target) = rig.ask("passpool").await;
        assert_eq!((code, target.as_str()), (200, "b-live"));
    }
}

#[tokio::test]
async fn a_dead_endpoint_cools_is_skipped_and_probed_after_the_cooldown() {
    let rig = rig(404, plain(), FASTEST).await;
    // Two answers in a row (the threshold) cool it.
    for _ in 0..2 {
        assert_eq!(rig.ask("fast").await, (200, "b-live".to_string()));
    }
    assert_eq!(rig.dead.calls(), 2);
    assert_eq!(rig.state_of("a-dead").await, "cooling_down");
    // While it cools it is not asked at all.
    for _ in 0..4 {
        assert_eq!(rig.ask("fast").await.1, "b-live");
    }
    assert_eq!(rig.dead.calls(), 2);
    // After the cooldown one request probes it; it fails again and the request still succeeds.
    rig.clock.0.fetch_add(10_001, Ordering::SeqCst);
    assert_eq!(rig.ask("fast").await, (200, "b-live".to_string()));
    assert_eq!(rig.dead.calls(), 3, "exactly one probe");
    assert_eq!(rig.state_of("a-dead").await, "cooling_down");
}

#[tokio::test]
async fn health_counts_apply_to_every_route_even_where_the_request_is_not_moved_on() {
    // A passthrough route returns the 404, but its dead first target still cools, so the next request
    // skips it and is served.
    let rig = rig(404, plain(), PASS).await;
    for _ in 0..2 {
        assert_eq!(rig.ask("pass").await.0, 404);
    }
    assert_eq!(rig.state_of("a-dead").await, "cooling_down");
    assert_eq!(rig.ask("pass").await, (200, "b-live".to_string()));
}

#[tokio::test]
async fn a_403_counts_against_health_but_keeps_its_own_class() {
    let rig = rig(403, plain(), FASTEST).await;
    // 403 has its own class (forbidden), already allowed by default, so the request moves on.
    for _ in 0..2 {
        assert_eq!(rig.ask("fast").await, (200, "b-live".to_string()));
    }
    assert_eq!(rig.state_of("a-dead").await, "cooling_down");
}

#[tokio::test]
async fn a_target_with_several_endpoints_moves_on_within_itself_on_every_route() {
    let rig = rig(
        404,
        plain(),
        "[routes.one]\ntype = \"passthrough\"\ntargets = [\"multi\"]\n",
    )
    .await;
    for _ in 0..3 {
        assert_eq!(rig.ask("one").await.0, 200, "the second endpoint serves");
    }
    assert!(rig.live.calls() >= 3);
}

#[tokio::test]
async fn when_everything_is_gone_the_client_gets_the_last_status() {
    let routes =
        "[routes.allfast]\ntype = \"fastest\"\ntargets = [\"a-dead\", \"c-dead\"]\nexplore = 0.0\n";
    let rig = rig(404, not_found(), routes).await;
    let resp = reqwest::Client::new()
        .post(format!("{}/v1/chat/completions", rig.url))
        .json(&chat_request("allfast", false))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);
    let body: Value = resp.json().await.unwrap();
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap()
            .contains("does not exist"),
        "the provider's message comes through: {body}"
    );
    assert!(
        rig.dead.calls() >= 1 && rig.second_dead.calls() >= 1,
        "both were tried"
    );
}

#[tokio::test]
async fn a_strict_route_never_crosses_classes_to_get_away_from_a_dead_target() {
    let routes = "[routes.private-pool]\ntype = \"fastest\"\npool = { strength = \"general\" }\nallowed_classes = [\"private\"]\nexplore = 0.0\n";
    let rig = rig(404, plain(), routes).await;
    // Only the private target is allowed; it is dead, and the public one must not be used.
    let (code, _) = rig.ask("private-pool").await;
    assert_eq!(code, 404);
    assert_eq!(rig.live.calls(), 0);
}

#[tokio::test]
async fn explain_says_whether_a_route_moves_on_for_unavailable_answers() {
    let rig = rig(404, plain(), &format!("{FASTEST}{PASS}{PASS_OPT_IN}")).await;
    let explain = |route: &'static str| {
        let url = rig.url.clone();
        async move {
            reqwest::Client::new()
                .post(format!("{url}/v1/route/explain"))
                .json(&json!({"model": route}))
                .send()
                .await
                .unwrap()
                .json::<Value>()
                .await
                .unwrap()
        }
    };
    assert_eq!(explain("fast").await["fallback_includes_unavailable"], true);
    assert_eq!(
        explain("pass").await["fallback_includes_unavailable"],
        false
    );
    assert_eq!(
        explain("optin").await["fallback_includes_unavailable"],
        true
    );
    assert_eq!(explain("fast").await["fallback_includes_rejected"], true);
    assert_eq!(explain("pass").await["fallback_includes_rejected"], false);
    assert_eq!(explain("optin").await["fallback_includes_rejected"], false);
}

/// The exact set of client errors that move a `fastest` pool on (ADR 0023, ADR 0026): those that describe the
/// endpoint, 413, and 400 and 422 (a request this provider rejects). Every other 4xx ends the request. Pinning the
/// whole range means a status added to (or dropped from) the classification is a visible, deliberate change.
/// Statuses such as 405 and 407 are left as request errors until a provider is seen answering them.
#[tokio::test]
async fn exactly_the_endpoint_describing_client_errors_move_a_pool_on() {
    // 413 is `overflow`: too large for this endpoint, so the pool moves on (ADR 0023, amended again).
    const DESCRIBE_THE_ENDPOINT: [u16; 8] = [400, 401, 402, 403, 404, 410, 413, 422];
    for status in 400..=451u16 {
        if StatusCode::from_u16(status).is_err() || status == 429 || status == 408 {
            continue; // not a status, or a class of its own with its own tests
        }
        let rig = rig(status, plain(), &format!("{FASTEST}{POOL}")).await;
        let (code, target) = rig.ask("pool").await;
        if DESCRIBE_THE_ENDPOINT.contains(&status) {
            assert_eq!(
                (code, target.as_str()),
                (200, "b-live"),
                "{status} describes the endpoint or the rejected request and the pool moves on"
            );
        } else {
            assert_eq!(
                code, status,
                "{status} is a request error and ends the request"
            );
        }
    }
}

/// A 413 (a per-minute token cap, a body limit) says the request is too large for this endpoint, not that
/// the endpoint is unwell: the request moves on, but the endpoint is not counted as failing and is asked again.
#[tokio::test]
async fn a_413_moves_on_without_cooling_the_endpoint() {
    let rig = rig(413, plain(), FASTEST).await;
    for _ in 0..5 {
        assert_eq!(rig.ask("fast").await, (200, "b-live".to_string()));
    }
    assert_eq!(
        rig.dead.calls(),
        5,
        "it is asked every time, never skipped as cold"
    );
    assert_ne!(rig.state_of("a-dead").await, "cooling_down");
}

#[tokio::test]
async fn a_413_moves_on_within_a_target() {
    let rig = rig(
        413,
        plain(),
        "[routes.one]\ntype = \"passthrough\"\ntargets = [\"multi\"]\n",
    )
    .await;
    for _ in 0..3 {
        assert_eq!(rig.ask("one").await.0, 200, "the second endpoint serves");
    }
    assert_ne!(rig.state_of("a-dead").await, "cooling_down");
}

#[tokio::test]
async fn when_every_member_is_too_large_the_client_gets_the_413() {
    let routes =
        "[routes.allfast]\ntype = \"fastest\"\ntargets = [\"a-dead\", \"c-dead\"]\nexplore = 0.0\n";
    let rig = rig(
        413,
        json!({"error": {"message": "Request too large for model m"}}),
        routes,
    )
    .await;
    let resp = reqwest::Client::new()
        .post(format!("{}/v1/chat/completions", rig.url))
        .json(&chat_request("allfast", false))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 413);
    let body: Value = resp.json().await.unwrap();
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap()
            .contains("too large"),
        "the provider's message comes through: {body}"
    );
    assert!(
        rig.dead.calls() >= 1 && rig.second_dead.calls() >= 1,
        "both were tried"
    );
}

/// A pool of `rejecting` providers that answer 400 (each its own provider and target), a provider that serves
/// (in the pool only when `live_in_pool`) and a tail that serves. Returns the route's status and how many calls
/// the serving and tail upstreams received.
async fn rejecting_pool(rejecting: usize, live_in_pool: bool, route: &str) -> (u16, usize, usize) {
    let mut toml = String::from("listen = \"127.0.0.1:0\"\n");
    for i in 0..rejecting {
        let (_, url) = upstream(
            400,
            json!({"error": {"message": "Unsupported parameter(s): `x`"}}),
        )
        .await;
        write!(
            toml,
            "[providers.d{i}]\nbase_url = \"{url}\"\napi_key_env = \"PATH\"\nmax_retries = 0\n\
             [targets.d{i}]\nstrengths = [\"general\"]\nendpoints = [{{ provider = \"d{i}\", model = \"m\" }}]\n"
        )
        .unwrap();
    }
    let (live, live_url) = upstream(200, json!({})).await;
    let (tail, tail_url) = upstream(200, json!({})).await;
    let live_strength = if live_in_pool { "general" } else { "elsewhere" };
    write!(
        toml,
        "[providers.live]\nbase_url = \"{live_url}\"\napi_key_env = \"PATH\"\nmax_retries = 0\n\
         [targets.live]\nstrengths = [\"{live_strength}\"]\nendpoints = [{{ provider = \"live\", model = \"m\" }}]\n\
         [providers.tail]\nbase_url = \"{tail_url}\"\napi_key_env = \"PATH\"\nmax_retries = 0\n\
         [targets.tail]\nendpoints = [{{ provider = \"tail\", model = \"m\" }}]\n{route}\n"
    )
    .unwrap();
    let config = Config::from_toml(&toml, |n| std::env::var(n).ok()).unwrap();
    let url = format!(
        "http://{}",
        serve(server::router(config).await.unwrap()).await
    );
    let resp = reqwest::Client::new()
        .post(format!("{url}/v1/chat/completions"))
        .json(&chat_request("pool", false))
        .send()
        .await
        .unwrap();
    (resp.status().as_u16(), live.calls(), tail.calls())
}

const OPEN_POOL: &str =
    "[routes.pool]\ntype = \"fastest\"\npool = { strength = \"general\" }\nexplore = 0.0\n";
const GUARDED_POOL: &str = "[routes.pool]\ntype = \"fastest\"\npool = { strength = \"general\" }\nexplore = 0.0\n\
                            then = { targets = [\"tail\"] }\n";

/// Two providers that reject a request do not stop it (the third serves), but three do: a request wrong on three
/// providers is the request, and the client gets the last rejection with its real message (ADR 0026).
#[tokio::test]
async fn three_different_providers_rejecting_a_request_stop_it() {
    let (code, live_calls, _) = rejecting_pool(2, true, OPEN_POOL).await;
    assert_eq!(code, 200, "two rejections: the next member serves");
    assert_eq!(live_calls, 1);
    let (code, live_calls, _) = rejecting_pool(3, true, OPEN_POOL).await;
    assert_eq!(code, 400, "three rejections: the request is the problem");
    assert_eq!(live_calls, 0, "and no further member is tried");
}

/// The guarded tail is reached only by the classes its `on` list names, `overflow` by default, so a request every
/// member rejects never reaches a paid tail (ADR 0024, ADR 0026). Two rejecting members and no live one: the
/// bound is not reached, so it is the guard that keeps the tail out.
#[tokio::test]
async fn a_rejection_does_not_reach_the_guarded_tail() {
    let (code, _, tail_calls) = rejecting_pool(2, false, GUARDED_POOL).await;
    assert_eq!(code, 400, "the client gets the rejection");
    assert_eq!(tail_calls, 0, "and the tail is not called");
}

/// A tail that lists `rejected` does take the request once the members have rejected it.
#[tokio::test]
async fn a_tail_that_lists_rejected_takes_the_request() {
    let route = "[routes.pool]\ntype = \"fastest\"\npool = { strength = \"general\" }\nexplore = 0.0\n\
                 then = { targets = [\"tail\"], on = [\"rejected\"] }\n";
    let (code, _, tail_calls) = rejecting_pool(2, false, route).await;
    assert_eq!(code, 200);
    assert_eq!(tail_calls, 1);
}
