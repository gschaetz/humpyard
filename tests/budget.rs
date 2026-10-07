//! Budget enforcement end to end: limits, restricted tiers, 402s, key info, restart recovery.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};
use std::time::Duration;

use axum::Router;
use axum::extract::State;
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use serde_json::{Value, json};
use switchyard_conductor::auth::generate_key;
use switchyard_conductor::clock::Clock;
use switchyard_conductor::config::Config;
use switchyard_conductor::ledger::Ledger;
use switchyard_conductor::server::{self, Options};

async fn completions(
    State(calls): State<Arc<AtomicUsize>>,
    axum::Json(body): axum::Json<Value>,
) -> Response {
    calls.fetch_add(1, Ordering::SeqCst);
    let model = body["model"].as_str().unwrap().to_string();
    if model == "delay-m" {
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    axum::Json(json!({"id": "c", "object": "chat.completion", "created": 1, "model": model,
        "choices": [{"index": 0, "message": {"role": "assistant", "content": model}, "finish_reason": "stop"}],
        "usage": {"prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15}}))
    .into_response()
}

async fn serve(app: Router) -> SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    addr
}

struct TestClock(AtomicI64);
impl Clock for TestClock {
    fn now_ms(&self) -> i64 {
        self.0.load(Ordering::SeqCst)
    }
}

/// Keys: each test uses the key whose limits it needs. Prices (USD per million tokens) make every
/// call cost: fast 20 micro-USD, smart 250, free 0, with 15 tokens each.
const TEMPLATE: &str = r#"
[budget]
restricted_at = 0.8
restricted_max_output_price = 5.0

[targets.fast]
endpoints = [{ provider = "mock", model = "fast-m", price = { input = 1.0, output = 2.0 } }]
[targets.smart]
endpoints = [{ provider = "mock", model = "smart-m", price = { input = 10.0, output = 30.0 } }]
[targets.free]
endpoints = [{ provider = "mock", model = "free-m", price = { input = 0.0, output = 0.0 } }]
[targets.delay]
endpoints = [{ provider = "mock", model = "delay-m", price = { input = 1.0, output = 2.0 } }]

[routes.auto]
type = "stage_router"
efficient = ["fast"]
capable = ["smart"]
[routes.chain]
type = "passthrough"
targets = ["smart", "free"]
[routes.premium]
type = "passthrough"
targets = ["smart"]
"#;

struct Gateway {
    url: String,
    upstream_calls: Arc<AtomicUsize>,
    keys: Vec<(String, String)>,
}

struct Setup {
    dir: tempfile::TempDir,
    upstream: SocketAddr,
    calls: Arc<AtomicUsize>,
    keys: Vec<(String, String, String)>, // (id, key, hash)
    limits: Vec<String>,                 // per-key TOML limit lines
}

impl Setup {
    async fn new(limits: &[(&str, &str)]) -> Self {
        let calls = Arc::new(AtomicUsize::new(0));
        let upstream = serve(
            Router::new()
                .route("/chat/completions", post(completions))
                .with_state(calls.clone()),
        )
        .await;
        let keys = limits
            .iter()
            .map(|(id, _)| {
                let (key, hash) = generate_key().unwrap();
                (id.to_string(), key, hash)
            })
            .collect();
        Self {
            dir: tempfile::tempdir().unwrap(),
            upstream,
            calls,
            keys,
            limits: limits.iter().map(|(_, l)| l.to_string()).collect(),
        }
    }

    fn db(&self) -> PathBuf {
        self.dir.path().join("ledger.db")
    }

    fn config(&self) -> Config {
        let mut toml = format!(
            "listen = \"127.0.0.1:0\"\n[ledger]\npath = \"{}\"\n[providers.mock]\nbase_url = \"http://{}\"\napi_key_env = \"K\"\nmax_retries = 0\n",
            self.db().display(),
            self.upstream
        );
        for ((id, _, hash), limits) in self.keys.iter().zip(&self.limits) {
            toml += &format!("[keys.{id}]\nsha256 = \"{hash}\"\n{limits}\n");
        }
        toml += TEMPLATE;
        Config::from_toml(&toml, |_| Some("key".into())).unwrap()
    }

    async fn start(&self, options: Options) -> Gateway {
        let addr = serve(
            server::router_with_options(self.config(), options)
                .await
                .unwrap(),
        )
        .await;
        Gateway {
            url: format!("http://{addr}"),
            upstream_calls: self.calls.clone(),
            keys: self
                .keys
                .iter()
                .map(|(id, key, _)| (id.clone(), key.clone()))
                .collect(),
        }
    }
}

impl Gateway {
    fn key(&self, id: &str) -> &str {
        &self.keys.iter().find(|(k, _)| k == id).unwrap().1
    }

    async fn post(&self, id: &str, path: &str, body: Value) -> reqwest::Response {
        reqwest::Client::new()
            .post(format!("{}{path}", self.url))
            .bearer_auth(self.key(id))
            .json(&body)
            .send()
            .await
            .unwrap()
    }

    async fn chat(&self, id: &str, model: &str) -> reqwest::Response {
        self.post(
            id,
            "/v1/chat/completions",
            json!({"model": model, "messages": [{"role": "user", "content": "hi"}]}),
        )
        .await
    }

    async fn info(&self, id: &str) -> Value {
        reqwest::Client::new()
            .get(format!("{}/v1/key/info", self.url))
            .bearer_auth(self.key(id))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap()
    }

    fn calls(&self) -> usize {
        self.upstream_calls.load(Ordering::SeqCst)
    }
}

fn target(resp: &reqwest::Response) -> String {
    resp.headers()
        .get("x-conductor-target")
        .map(|v| v.to_str().unwrap().to_string())
        .unwrap_or_default()
}

/// A tool turn whose result failed, so the stage router escalates.
fn failing_turn(model: &str) -> Value {
    json!({"model": model, "messages": [
        {"role": "user", "content": "fix the build"},
        {"role": "assistant", "content": null, "tool_calls": [{"id": "call_1", "type": "function",
            "function": {"name": "Bash", "arguments": "{\"command\":\"cargo test\"}"}}]},
        {"role": "tool", "tool_call_id": "call_1", "content": "fatal runtime error: out of memory"}]})
}

#[tokio::test]
async fn an_exhausted_key_gets_402_in_every_protocol_before_any_upstream_call() {
    let setup = Setup::new(&[("tight", "daily_tokens = 15")]).await;
    let g = setup.start(Options::default()).await;
    assert_eq!(g.chat("tight", "fast").await.status(), 200); // spends exactly the 15-token limit
    let before = g.calls();

    let chat = g.chat("tight", "fast").await;
    assert_eq!(chat.status(), 402);
    let body: Value = chat.json().await.unwrap();
    let message = body["error"]["message"].as_str().unwrap();
    assert!(
        message.contains("daily_tokens") && message.contains("tight"),
        "{message}"
    );

    let anthropic = g.post("tight", "/v1/messages", json!({"model": "fast", "max_tokens": 5, "messages": [{"role": "user", "content": "hi"}]})).await;
    assert_eq!(anthropic.status(), 402);
    assert_eq!(anthropic.json::<Value>().await.unwrap()["type"], "error");
    assert_eq!(
        g.post(
            "tight",
            "/v1/responses",
            json!({"model": "fast", "input": "hi"})
        )
        .await
        .status(),
        402
    );
    assert_eq!(
        g.calls(),
        before,
        "no upstream call after the budget was reached"
    );
}

#[tokio::test]
async fn spend_is_visible_to_the_very_next_request() {
    let setup = Setup::new(&[("live", "daily_tokens = 40")]).await;
    let g = setup.start(Options::default()).await;
    for _ in 0..3 {
        assert_eq!(g.chat("live", "fast").await.status(), 200); // 15, 30, 45 tokens
    }
    assert_eq!(g.chat("live", "fast").await.status(), 402);
}

#[tokio::test]
async fn free_only_keys_continue_on_free_targets_and_get_402_when_none_is_eligible() {
    let setup = Setup::new(&[(
        "freeloader",
        "daily_tokens = 15\nover_budget = \"free_only\"",
    )])
    .await;
    let g = setup.start(Options::default()).await;
    let first = g.chat("freeloader", "chain").await;
    assert_eq!(
        (first.status().as_u16(), target(&first).as_str()),
        (200, "smart")
    );
    // Over budget now: the route's priced target is excluded, its free target serves.
    let second = g.chat("freeloader", "chain").await;
    assert_eq!(
        (second.status().as_u16(), target(&second).as_str()),
        (200, "free")
    );
    // A route with no free target cannot continue.
    assert_eq!(g.chat("freeloader", "premium").await.status(), 402);
}

#[tokio::test]
async fn a_restricted_key_is_routed_away_from_expensive_targets() {
    // $0.0003 daily limit: one 250 micro-USD smart call puts the key at 83%.
    let setup = Setup::new(&[("near", "daily_usd = 0.0003")]).await;
    let g = setup.start(Options::default()).await;
    assert_eq!(g.chat("near", "smart").await.status(), 200);
    assert_eq!(g.info("near").await["state"], "restricted");
    // This turn would escalate to smart; smart now exceeds the price ceiling.
    let resp = g
        .post("near", "/v1/chat/completions", failing_turn("auto"))
        .await;
    assert_eq!(
        (resp.status().as_u16(), target(&resp).as_str()),
        (200, "fast")
    );
}

#[tokio::test]
async fn key_info_reflects_spend_right_after_a_request() {
    let setup = Setup::new(&[("info", "daily_usd = 1.0\nmonthly_tokens = 1000")]).await;
    let g = setup.start(Options::default()).await;
    assert_eq!(g.chat("info", "fast").await.status(), 200);
    let info = g.info("info").await;
    assert_eq!(info["id"], "info");
    assert_eq!(info["state"], "healthy");
    assert_eq!(info["limits"]["daily_usd"], 1.0);
    assert_eq!(info["spend"]["daily_usd"], 0.00002);
    assert_eq!(info["spend"]["monthly_tokens"], 15);
    assert_eq!(info["remaining"]["monthly_tokens"], 985.0);
    assert!(info["limits"]["monthly_usd"].is_null());
    let anon = reqwest::get(format!("{}/v1/key/info", g.url))
        .await
        .unwrap();
    assert_eq!(anon.status(), 401);
}

#[tokio::test]
async fn in_flight_requests_may_overshoot_but_later_ones_are_refused() {
    let setup = Setup::new(&[("edge", "daily_tokens = 15")]).await;
    let g = Arc::new(setup.start(Options::default()).await);
    let tasks: Vec<_> = (0..5)
        .map(|_| {
            let g = g.clone();
            tokio::spawn(async move { g.chat("edge", "delay").await.status().as_u16() })
        })
        .collect();
    for task in tasks {
        assert_eq!(
            task.await.unwrap(),
            200,
            "all five started while the key was under its limit"
        );
    }
    assert_eq!(g.info("edge").await["spend"]["daily_tokens"], 75);
    assert_eq!(g.chat("edge", "fast").await.status(), 402);
}

#[tokio::test]
async fn spend_survives_a_restart_because_it_is_recovered_from_the_ledger() {
    let setup = Setup::new(&[("sticky", "daily_tokens = 15")]).await;
    let first = setup.start(Options::default()).await;
    assert_eq!(first.chat("sticky", "fast").await.status(), 200);
    // Wait for the asynchronous writer, as a real shutdown would.
    let reader = Ledger::open(&setup.db()).await.unwrap();
    for _ in 0..100 {
        if !reader.entries_since(0).await.unwrap().is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(30)).await;
    }
    drop(first);

    let second = setup.start(Options::default()).await; // new process state, same ledger file
    assert_eq!(second.info("sticky").await["spend"]["daily_tokens"], 15);
    assert_eq!(second.chat("sticky", "fast").await.status(), 402);
}

#[tokio::test]
async fn the_daily_budget_resets_at_utc_midnight() {
    let setup = Setup::new(&[("daily", "daily_tokens = 15\nmonthly_tokens = 1000")]).await;
    let start = 1_791_000_000_000_i64;
    let clock = Arc::new(TestClock(AtomicI64::new(start)));
    let g = setup
        .start(Options {
            clock: Some(clock.clone()),
            ..Options::default()
        })
        .await;
    assert_eq!(g.chat("daily", "fast").await.status(), 200);
    assert_eq!(g.chat("daily", "fast").await.status(), 402);
    clock.0.fetch_add(86_400_000, Ordering::SeqCst);
    assert_eq!(
        g.chat("daily", "fast").await.status(),
        200,
        "a new UTC day starts from zero"
    );
    assert_eq!(
        g.info("daily").await["spend"]["monthly_tokens"],
        30,
        "the month keeps counting"
    );
}

/// The whole story: an escalating agent spends its way from healthy, to restricted (steered to the
/// cheap target), to blocked.
#[tokio::test]
async fn an_escalating_route_walks_a_key_from_healthy_to_restricted_to_blocked() {
    // $0.0006 limit; smart calls cost 250 micro-USD, fast calls 20.
    let setup = Setup::new(&[("agent", "daily_usd = 0.0006")]).await;
    let g = setup.start(Options::default()).await;
    let mut served = Vec::new();
    let mut final_status = 0;
    for _ in 0..12 {
        let resp = g
            .post("agent", "/v1/chat/completions", failing_turn("auto"))
            .await;
        final_status = resp.status().as_u16();
        if final_status != 200 {
            break;
        }
        served.push(target(&resp));
    }
    assert_eq!(
        &served[..2],
        ["smart", "smart"],
        "healthy: failing turns escalate ({served:?})"
    );
    assert!(
        served[2..].iter().all(|t| t == "fast"),
        "restricted: steered to the cheap target ({served:?})"
    );
    assert!(
        served.len() > 3,
        "the cheap target served several requests before the cap"
    );
    assert_eq!(final_status, 402, "then the key is blocked");
    assert_eq!(g.info("agent").await["state"], "exhausted");
}
