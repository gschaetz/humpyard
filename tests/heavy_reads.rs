//! Heavy reads (statistics and session views) through the real endpoints (ADR 0036): one at a time per surface, shed
//! with 503 and `Retry-After` and never queued, identical reads sharing one result, and each caller its own.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // integration tests fail loudly on purpose

mod common;

use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::Router;
use axum::routing::post;
use common::{chat_completion, chat_request, serve, usage};
use humpyard::config::Config;
use humpyard::server::{self, HeavyReads, Options, Surface};
use serde_json::Value;

struct Keys {
    alice: String,
    bob: String,
    reader: String,
    writer: String,
}

struct Fixture {
    base: String,
    heavy: Arc<HeavyReads>,
    keys: Keys,
}

async fn fixture(name: &str, heavy: HeavyReads) -> Fixture {
    let upstream = serve(Router::new().route(
        "/chat/completions",
        post(|| async { axum::Json(chat_completion("m", "ok", Some(usage(20, 5)))) }),
    ))
    .await;
    let dir = common::scratch().join(format!("heavy-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let make = || humpyard::auth::generate_key().unwrap();
    let ((alice, alice_hash), (bob, bob_hash)) = (make(), make());
    let ((reader, reader_hash), (writer, writer_hash)) = (make(), make());
    let toml = format!(
        r#"listen = "127.0.0.1:0"
[ledger]
path = "{}"
[providers.p]
base_url = "http://{upstream}"
api_key_env = "PATH"
max_retries = 0
[targets.t]
endpoints = [{{ provider = "p", model = "m", price = {{ input = 0.0, output = 0.0 }} }}]
[routes.r]
type = "passthrough"
targets = ["t"]
[keys.alice]
sha256 = "{alice_hash}"
[keys.bob]
sha256 = "{bob_hash}"
[admin.credentials.reader]
sha256 = "{reader_hash}"
scopes = ["read"]
[admin.credentials.writer]
sha256 = "{writer_hash}"
scopes = ["overrides"]
"#,
        dir.join("ledger.db").display()
    );
    let heavy = Arc::new(heavy);
    let config = Config::from_toml(&toml, |n| std::env::var(n).ok()).unwrap();
    let router = server::router_with_options(
        config,
        Options {
            heavy_reads: Some(heavy.clone()),
            ..Options::default()
        },
    )
    .await
    .unwrap();
    Fixture {
        base: format!("http://{}", serve(router).await),
        heavy,
        keys: Keys {
            alice,
            bob,
            reader,
            writer,
        },
    }
}

async fn get(base: &str, path: &str, key: &str) -> (u16, Option<String>, Value) {
    let response = reqwest::Client::new()
        .get(format!("{base}{path}"))
        .bearer_auth(key)
        .send()
        .await
        .unwrap();
    let status = response.status().as_u16();
    let retry_after = response
        .headers()
        .get("retry-after")
        .map(|v| v.to_str().unwrap().to_string());
    (
        status,
        retry_after,
        response.json().await.unwrap_or(Value::Null),
    )
}

async fn ask(base: &str, key: &str, times: usize) {
    let client = reqwest::Client::new();
    for _ in 0..times {
        let r = client
            .post(format!("{base}/v1/chat/completions"))
            .bearer_auth(key)
            .json(&chat_request("r", false))
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 200);
    }
}

/// Waits until a read holds the surface's gate.
async fn gate_taken(heavy: &HeavyReads, surface: Surface) {
    let started = Instant::now();
    while !heavy.gate_taken(surface) {
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "no read took the gate"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[tokio::test]
async fn a_second_heavy_admin_read_while_one_runs_is_shed_with_retry_after() {
    let f = fixture("second", HeavyReads::default()).await;
    let pause = f.heavy.pause();
    let first = {
        let (base, key) = (f.base.clone(), f.keys.reader.clone());
        tokio::spawn(async move { get(&base, "/admin/v1/stats?window=1h", &key).await })
    };
    gate_taken(&f.heavy, Surface::Admin).await;

    let started = Instant::now();
    let (status, retry_after, body) =
        get(&f.base, "/admin/v1/stats?window=2h", &f.keys.reader).await;
    assert_eq!(status, 503, "{body}");
    assert_eq!(retry_after.as_deref(), Some("5"));
    assert_eq!(body["error"]["code"], "admin_busy");
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "it must not queue"
    );
    // A session view is a heavy read too.
    let (status, _, body) = get(&f.base, "/admin/v1/stats?group_by=session", &f.keys.reader).await;
    assert_eq!(
        (status, body["error"]["code"].as_str()),
        (503, Some("admin_busy"))
    );

    drop(pause);
    assert_eq!(first.await.unwrap().0, 200);
}

#[tokio::test]
async fn the_same_read_twice_is_computed_once_and_says_when() {
    let f = fixture("twice", HeavyReads::default()).await;
    let (s1, _, first) = get(&f.base, "/admin/v1/stats?window=1h", &f.keys.reader).await;
    let (s2, _, second) = get(&f.base, "/admin/v1/stats?window=1h", &f.keys.reader).await;
    assert_eq!((s1, s2), (200, 200));
    assert!(first["computed_at_ms"].as_i64().unwrap() > 0, "{first}");
    assert_eq!(first, second, "the second answer is the first");
    assert_eq!(f.heavy.computations(), 1);
}

#[tokio::test]
async fn five_identical_reads_at_once_are_one_computation_and_none_is_refused() {
    let f = fixture("five", HeavyReads::default()).await;
    let pause = f.heavy.pause();
    let readers: Vec<_> = (0..5)
        .map(|_| {
            let (base, key) = (f.base.clone(), f.keys.reader.clone());
            tokio::spawn(async move { get(&base, "/admin/v1/stats?window=6h", &key).await })
        })
        .collect();
    gate_taken(&f.heavy, Surface::Admin).await;
    tokio::time::sleep(Duration::from_millis(200)).await; // the other four arrive and wait on the first
    drop(pause);
    let mut answers = Vec::new();
    for reader in readers {
        let (status, _, body) = reader.await.unwrap();
        assert_eq!(status, 200, "{body}");
        answers.push(body);
    }
    assert!(answers.windows(2).all(|w| w[0] == w[1]));
    assert_eq!(f.heavy.computations(), 1);
}

#[tokio::test]
async fn reads_that_differ_are_computed_for_their_own_parameters() {
    let f = fixture("differ", HeavyReads::default()).await;
    let (_, _, by_route) = get(&f.base, "/admin/v1/stats?group_by=route", &f.keys.reader).await;
    let (_, _, by_target) = get(&f.base, "/admin/v1/stats?group_by=target", &f.keys.reader).await;
    let (_, _, filtered) = get(
        &f.base,
        "/admin/v1/stats?group_by=route&key=alice",
        &f.keys.reader,
    )
    .await;
    assert_eq!(by_route["group_by"], "route");
    assert_eq!(by_target["group_by"], "target");
    assert_eq!(filtered["group_by"], "route");
    assert_eq!(f.heavy.computations(), 3);
}

#[tokio::test]
async fn light_admin_reads_are_answered_while_a_heavy_one_runs() {
    let f = fixture("light", HeavyReads::default()).await;
    let pause = f.heavy.pause();
    let heavy = {
        let (base, key) = (f.base.clone(), f.keys.reader.clone());
        tokio::spawn(async move { get(&base, "/admin/v1/stats", &key).await })
    };
    gate_taken(&f.heavy, Surface::Admin).await;
    for path in ["/admin/v1/whoami", "/admin/v1/health", "/admin/v1/config"] {
        let (status, _, body) = get(&f.base, path, &f.keys.reader).await;
        assert_eq!(status, 200, "{path}: {body}");
    }
    drop(pause);
    assert_eq!(heavy.await.unwrap().0, 200);
}

#[tokio::test]
async fn a_credential_without_the_scope_is_refused_and_does_not_take_the_gate() {
    let f = fixture("scope", HeavyReads::default()).await;
    let (status, _, body) = get(&f.base, "/admin/v1/stats", &f.keys.writer).await;
    assert_eq!(status, 403, "{body}");
    assert_eq!(body["error"]["code"], "admin_scope_required");
    assert!(!f.heavy.gate_taken(Surface::Admin));
    assert_eq!(f.heavy.computations(), 0);
    // And it cannot read a result a permitted caller made.
    assert_eq!(get(&f.base, "/admin/v1/stats", &f.keys.reader).await.0, 200);
    let (status, _, _) = get(&f.base, "/admin/v1/stats", &f.keys.writer).await;
    assert_eq!(status, 403);
}

#[tokio::test]
async fn two_keys_reading_the_same_statistics_get_their_own_results() {
    let f = fixture("keys", HeavyReads::default()).await;
    ask(&f.base, &f.keys.alice, 3).await;
    ask(&f.base, &f.keys.bob, 1).await;
    let (_, _, alice) = get(&f.base, "/v1/stats?group_by=route", &f.keys.alice).await;
    let (_, _, bob) = get(&f.base, "/v1/stats?group_by=route", &f.keys.bob).await;
    assert_eq!(alice["data"][0]["requests"], 3, "{alice}");
    assert_eq!(bob["data"][0]["requests"], 1, "{bob}");
    assert_eq!(f.heavy.computations(), 2);
    // Alice reading again within the freshness gets her first answer, not Bob's.
    let (_, _, again) = get(&f.base, "/v1/stats?group_by=route", &f.keys.alice).await;
    assert_eq!(again, alice);
    assert_eq!(f.heavy.computations(), 2);
}

#[tokio::test]
async fn a_client_key_cannot_hold_the_administrative_gate() {
    let f = fixture("surfaces", HeavyReads::default()).await;
    let _client_gate = f.heavy.hold(Surface::Client).unwrap();
    let (status, retry_after, body) = get(&f.base, "/v1/stats", &f.keys.alice).await;
    assert_eq!(status, 503, "{body}");
    assert_eq!(retry_after.as_deref(), Some("5"));
    assert_eq!(body["error"]["code"], "stats_busy");
    let (status, _, body) = get(&f.base, "/v1/stats/switches", &f.keys.alice).await;
    assert_eq!(
        (status, body["error"]["code"].as_str()),
        (503, Some("stats_busy"))
    );
    // The operator's surface is a gate of its own.
    assert_eq!(get(&f.base, "/admin/v1/stats", &f.keys.reader).await.0, 200);
}

#[tokio::test]
async fn the_switches_report_states_when_it_was_computed() {
    let f = fixture("switches", HeavyReads::default()).await;
    let (status, _, first) = get(&f.base, "/v1/stats/switches", &f.keys.alice).await;
    assert_eq!(status, 200, "{first}");
    assert!(first["computed_at_ms"].as_i64().unwrap() > 0, "{first}");
    let (_, _, second) = get(&f.base, "/v1/stats/switches", &f.keys.alice).await;
    assert_eq!(first, second);
    assert_eq!(f.heavy.computations(), 1);
}

#[tokio::test]
async fn with_no_freshness_every_read_is_computed_and_sees_new_requests() {
    let f = fixture("fresh", HeavyReads::with_ttl(Duration::ZERO)).await;
    ask(&f.base, &f.keys.alice, 1).await;
    let (_, _, one) = get(&f.base, "/v1/stats?group_by=route", &f.keys.alice).await;
    ask(&f.base, &f.keys.alice, 1).await;
    let (_, _, two) = get(&f.base, "/v1/stats?group_by=route", &f.keys.alice).await;
    assert_eq!(one["data"][0]["requests"], 1);
    assert_eq!(two["data"][0]["requests"], 2);
    assert_eq!(f.heavy.computations(), 2);
}
