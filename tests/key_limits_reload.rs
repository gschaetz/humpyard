//! Key limits across a configuration reload: a changed limit applies from the next request, a key that is
//! no longer limited loses its counters, and a request in flight keeps its permit.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test scaffolding

mod common;

use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::extract::State;
use axum::response::IntoResponse;
use axum::response::sse::{Event, Sse};
use axum::routing::post;
use common::{chat_completion, chunk, serve, usage};
use humpyard::config::Config;
use humpyard::server::{self, Reload};
use serde_json::{Value, json};
use tokio::sync::{mpsc, watch};

struct Upstream {
    gate: watch::Sender<bool>,
}

/// `hold` streams a chunk, waits at the gate and finishes; anything else answers at once.
async fn completions(
    State(up): State<Arc<Upstream>>,
    axum::Json(body): axum::Json<Value>,
) -> axum::response::Response {
    if body["model"] == "hold" {
        let events = async_stream::stream! {
            yield Ok::<_, Infallible>(Event::default().data(chunk(json!({"role": "assistant", "content": "Hel"}), None).to_string()));
            let mut open = up.gate.subscribe();
            open.wait_for(|open| *open).await.unwrap();
            yield Ok(Event::default().data(chunk(json!({}), Some("stop")).to_string()));
            yield Ok(Event::default().data("[DONE]"));
        };
        return Sse::new(events).into_response();
    }
    axum::Json(chat_completion("quick", "ok", Some(usage(10, 5)))).into_response()
}

struct Gateway {
    url: String,
    base: String,
    hash: String,
    key: String,
    path: std::path::PathBuf,
    reload: mpsc::Sender<()>,
    gate: watch::Sender<bool>,
    _stop: mpsc::Sender<()>,
    _dir: tempfile::TempDir,
}

fn config_text(
    base: &str,
    hash: &str,
    limits: &str,
    extra: &str,
    ledger: &std::path::Path,
) -> String {
    format!(
        r#"listen = "127.0.0.1:0"
[ledger]
path = "{ledger}"
[keys.k]
sha256 = "{hash}"
{limits}
[providers.p]
base_url = "{base}"
api_key_env = "PATH"
max_retries = 0
[targets.hold]
endpoints = [{{ provider = "p", model = "hold" }}]
[targets.quick]
endpoints = [{{ provider = "p", model = "quick" }}]
{extra}
"#,
        ledger = ledger.display()
    )
}

async fn start(limits: &str) -> Gateway {
    start_with(limits, "").await
}

/// Starts with `extra` appended to the configuration (a `[profiles.*]` table, say).
async fn start_with(limits: &str, extra: &str) -> Gateway {
    let (gate, _) = watch::channel(false);
    let app = Router::new()
        .route("/chat/completions", post(completions))
        .with_state(Arc::new(Upstream { gate: gate.clone() }));
    let base = format!("http://{}", serve(app).await);
    let dir = tempfile::tempdir().unwrap();
    let (key, hash) = humpyard::auth::generate_key().unwrap();
    let path = dir.path().join("humpyard.toml");
    let text = config_text(&base, &hash, limits, extra, &dir.path().join("ledger.db"));
    std::fs::write(&path, &text).unwrap();
    let config = Config::from_toml(&text, |n| std::env::var(n).ok()).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let (reload, triggers) = mpsc::channel(4);
    let (stop, signals) = mpsc::channel(4);
    tokio::spawn(server::run(
        config,
        listener,
        signals,
        Some(Reload {
            path: path.clone(),
            triggers,
        }),
    ));
    Gateway {
        url,
        base,
        hash,
        key,
        path,
        reload,
        gate,
        _stop: stop,
        _dir: dir,
    }
}

impl Gateway {
    async fn send(&self, route: &str, stream: bool) -> reqwest::Response {
        reqwest::Client::new()
            .post(format!("{}/v1/chat/completions", self.url))
            .bearer_auth(&self.key)
            .json(&json!({"model": route, "stream": stream, "messages": [{"role": "user", "content": "hi"}]}))
            .send()
            .await
            .unwrap()
    }

    async fn status(&self, route: &str) -> u16 {
        self.send(route, false).await.status().as_u16()
    }

    /// Rewrites the config with new limits for the key and asks for a reload.
    async fn reload_with(&self, limits: &str) {
        self.reload_all(limits, "").await;
    }

    /// Like [`Self::reload_with`], with `extra` appended to the configuration as well.
    async fn reload_all(&self, limits: &str, extra: &str) {
        let ledger = self.path.with_file_name("ledger.db");
        let text = config_text(&self.base, &self.hash, limits, extra, &ledger);
        std::fs::write(&self.path, text).unwrap();
        self.reload.send(()).await.unwrap();
    }

    /// Waits until a quick request answers `status`. Only for a state that, once reached, stays reached
    /// without sending more requests than the wait itself does.
    async fn wait_for_status(&self, status: u16) {
        for _ in 0..150 {
            if self.status("quick").await == status {
                return;
            }
            tokio::time::sleep(Duration::from_millis(40)).await;
        }
        panic!("the reload never made a request answer {status}");
    }
}

#[tokio::test]
async fn a_reload_that_raises_a_limit_admits_the_next_request() {
    let gw = start("max_requests_per_minute = 2").await;
    assert_eq!(gw.status("quick").await, 200);
    assert_eq!(gw.status("quick").await, 200);
    assert_eq!(gw.status("quick").await, 429);
    gw.reload_with("max_requests_per_minute = 50").await;
    gw.wait_for_status(200).await;
    assert_eq!(gw.status("quick").await, 200);
}

#[tokio::test]
async fn a_reload_that_lowers_a_limit_applies_it_to_the_requests_already_counted() {
    let gw = start("max_requests_per_minute = 50").await;
    for _ in 0..3 {
        assert_eq!(gw.status("quick").await, 200);
    }
    gw.reload_with("max_requests_per_minute = 3").await;
    gw.wait_for_status(429).await;
}

#[tokio::test]
async fn a_key_that_loses_its_limits_and_gets_one_again_starts_with_an_empty_window() {
    let gw = start("max_requests_per_minute = 1").await;
    assert_eq!(gw.status("quick").await, 200);
    assert_eq!(gw.status("quick").await, 429);
    gw.reload_with("").await;
    gw.wait_for_status(200).await;
    gw.reload_with("max_requests_per_minute = 1").await;
    // The requests made while it was unlimited, and before, are forgotten: one is admitted, the next is not.
    let mut admitted = false;
    for _ in 0..150 {
        if gw.status("quick").await == 200 {
            admitted = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(40)).await;
    }
    // Before the limit is back, requests are admitted without limit; once it is, exactly one more passes.
    assert!(admitted);
    gw.wait_for_status(429).await;
}

#[tokio::test]
async fn a_stream_in_flight_keeps_its_permit_across_a_reload() {
    let gw = start("max_concurrent = 1").await;
    let mut first = gw.send("hold", true).await;
    assert_eq!(first.status(), 200);
    assert!(first.chunk().await.unwrap().is_some());
    assert_eq!(gw.send("hold", true).await.status(), 429);

    gw.reload_with("max_concurrent = 2").await;
    // Wait for the reload to show: a second stream is admitted. The first still holds its place, so a
    // third is refused.
    let mut second = None;
    for _ in 0..150 {
        let response = gw.send("hold", true).await;
        if response.status() == 200 {
            second = Some(response);
            break;
        }
        tokio::time::sleep(Duration::from_millis(40)).await;
    }
    let mut second = second.expect("the raised limit never applied");
    assert!(second.chunk().await.unwrap().is_some());
    assert_eq!(gw.send("hold", true).await.status(), 429);

    // Both finish, and the permits taken before the reload come back to the counter now in force.
    gw.gate.send_replace(true);
    while first.chunk().await.unwrap().is_some() {}
    while second.chunk().await.unwrap().is_some() {}
    gw.wait_for_status(200).await;
}

const TEAM: &str = "[profiles.team]\nmax_requests_per_minute = ";

#[tokio::test]
async fn a_limit_inherited_from_a_profile_is_enforced_and_follows_a_reload_of_the_profile() {
    let gw = start_with("profile = \"team\"", &format!("{TEAM}2\n")).await;
    assert_eq!(gw.status("quick").await, 200);
    assert_eq!(gw.status("quick").await, 200);
    assert_eq!(
        gw.status("quick").await,
        429,
        "the profile's limit, inherited"
    );
    gw.reload_all("profile = \"team\"", &format!("{TEAM}50\n"))
        .await;
    gw.wait_for_status(200).await;
}

#[tokio::test]
async fn a_reload_that_would_widen_a_key_past_its_profile_is_refused_and_the_old_limit_keeps_serving()
 {
    let gw = start_with(
        "profile = \"team\"\nmax_requests_per_minute = 3",
        &format!("{TEAM}10\n"),
    )
    .await;
    // The profile drops to 2, under the key's own 3: the file no longer loads.
    gw.reload_all(
        "profile = \"team\"\nmax_requests_per_minute = 3",
        &format!("{TEAM}2\n"),
    )
    .await;
    let mut refused = false;
    for _ in 0..150 {
        let health: Value = reqwest::Client::new()
            .get(format!("{}/v1/health", gw.url))
            .bearer_auth(&gw.key)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        if health["reload"]["ok"] == false {
            let message = health["reload"]["message"].as_str().unwrap();
            assert!(
                message.contains("keys.k.max_requests_per_minute") && message.contains("`team`"),
                "{message}"
            );
            refused = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(40)).await;
    }
    assert!(refused, "the reload was never refused");
    // The previous configuration still serves, with the key's own limit of 3.
    for _ in 0..3 {
        assert_eq!(gw.status("quick").await, 200);
    }
    assert_eq!(gw.status("quick").await, 429);
}
