//! Actors (ADR 0037): what a key says about its owner reaches `GET /v1/key/info` and the effective configuration,
//! changes with a reload, and changes nothing a key may do.
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

const ACTORS: &str = r#"
[actors.gary]
kind = "human"
display_name = "Gary"
[actors.sam]
kind = "human"
[actors.claude-code]
kind = "agent"
owner = "gary"
[actors.openclaw]
kind = "agent"
"#;

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

fn config_text(base: &str, hash: &str, key_settings: &str, ledger: &std::path::Path) -> String {
    format!(
        r#"listen = "127.0.0.1:0"
[ledger]
path = "{ledger}"
[keys.k]
sha256 = "{hash}"
read_config = true
{key_settings}
[providers.p]
base_url = "{base}"
api_key_env = "PATH"
max_retries = 0
[targets.hold]
endpoints = [{{ provider = "p", model = "hold" }}]
[targets.quick]
endpoints = [{{ provider = "p", model = "quick" }}]
{ACTORS}
"#,
        ledger = ledger.display()
    )
}

async fn start(key_settings: &str) -> Gateway {
    let (gate, _) = watch::channel(false);
    let app = Router::new()
        .route("/chat/completions", post(completions))
        .with_state(Arc::new(Upstream { gate: gate.clone() }));
    let base = format!("http://{}", serve(app).await);
    let dir = tempfile::tempdir().unwrap();
    let (key, hash) = humpyard::auth::generate_key().unwrap();
    let path = dir.path().join("humpyard.toml");
    let text = config_text(&base, &hash, key_settings, &dir.path().join("ledger.db"));
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
    async fn get(&self, path: &str) -> Value {
        reqwest::Client::new()
            .get(format!("{}{path}", self.url))
            .bearer_auth(&self.key)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap()
    }

    async fn info(&self) -> Value {
        self.get("/v1/key/info").await
    }

    async fn send(&self, route: &str, stream: bool) -> reqwest::Response {
        reqwest::Client::new()
            .post(format!("{}/v1/chat/completions", self.url))
            .bearer_auth(&self.key)
            .json(&json!({"model": route, "stream": stream, "messages": [{"role": "user", "content": "hi"}]}))
            .send()
            .await
            .unwrap()
    }

    /// Rewrites the key's settings and asks for a reload.
    async fn reload_with(&self, key_settings: &str) {
        let ledger = self.path.with_file_name("ledger.db");
        std::fs::write(
            &self.path,
            config_text(&self.base, &self.hash, key_settings, &ledger),
        )
        .unwrap();
        self.reload.send(()).await.unwrap();
    }

    /// Waits until key info names `actor`.
    async fn wait_for_actor(&self, actor: Value) {
        for _ in 0..150 {
            if self.info().await["actor"] == actor {
                return;
            }
            tokio::time::sleep(Duration::from_millis(40)).await;
        }
        panic!("the reload never moved the key to {actor}");
    }
}

#[tokio::test]
async fn key_info_names_the_actor_and_the_person_it_acts_for() {
    let gw = start("actor = \"claude-code\"").await;
    let info = gw.info().await;
    assert_eq!(info["actor"], "claude-code");
    assert_eq!(info["owner"], "gary");
}

#[tokio::test]
async fn a_human_actors_key_rolls_up_to_that_human_and_an_agent_without_an_owner_to_nobody() {
    let gw = start("actor = \"gary\"").await;
    let info = gw.info().await;
    assert_eq!(
        (&info["actor"], &info["owner"]),
        (&json!("gary"), &json!("gary"))
    );
    gw.reload_with("actor = \"openclaw\"").await;
    gw.wait_for_actor(json!("openclaw")).await;
    assert_eq!(gw.info().await["owner"], Value::Null);
}

#[tokio::test]
async fn a_key_without_an_actor_has_neither_and_is_otherwise_unchanged() {
    let gw = start("").await;
    let info = gw.info().await;
    assert_eq!(info["actor"], Value::Null);
    assert_eq!(info["owner"], Value::Null);
    assert_eq!(info["id"], "k");
    assert_eq!(gw.send("quick", false).await.status(), 200);
}

#[tokio::test]
async fn the_effective_configuration_lists_each_actor_with_the_keys_that_name_it_and_no_secret() {
    let gw = start("actor = \"claude-code\"").await;
    let view = gw.get("/v1/config").await;
    let actors = &view["actors"];
    assert_eq!(actors["claude-code"]["kind"], "agent");
    assert_eq!(actors["claude-code"]["owner"], "gary");
    assert_eq!(actors["claude-code"]["keys"], json!(["k"]));
    assert_eq!(actors["claude-code"]["source"], "file");
    assert_eq!(actors["gary"]["display_name"], "Gary");
    assert_eq!(actors["gary"]["keys"], json!([]));
    assert_eq!(view["keys"]["k"]["actor"], "claude-code");
    assert_eq!(view["keys"]["k"]["owner"], "gary");
    let text = view.to_string();
    assert!(
        !text.contains(&gw.hash) && !text.contains(&gw.key),
        "{text}"
    );
}

#[tokio::test]
async fn a_reload_moves_a_key_to_its_new_actor_and_a_stream_in_flight_finishes() {
    let gw = start("actor = \"claude-code\"").await;
    let mut first = gw.send("hold", true).await;
    assert_eq!(first.status(), 200);
    first.chunk().await.unwrap();

    gw.reload_with("actor = \"sam\"").await;
    gw.wait_for_actor(json!("sam")).await;
    let info = gw.info().await;
    assert_eq!(
        (&info["actor"], &info["owner"]),
        (&json!("sam"), &json!("sam"))
    );

    gw.gate.send(true).unwrap();
    let rest = first.text().await.unwrap();
    assert!(rest.contains("[DONE]"), "{rest}");
}

#[tokio::test]
async fn an_actor_changes_nothing_a_key_may_do() {
    // The same key with and without an actor, a human's and an agent's: the same limits and the same answer.
    let mut seen = Vec::new();
    for settings in [
        "",
        "actor = \"gary\"",
        "actor = \"claude-code\"",
        "actor = \"openclaw\"",
    ] {
        let gw = start(&format!("daily_tokens = 5000\n{settings}")).await;
        let info = gw.info().await;
        seen.push((info["limits"].clone(), info["state"].clone()));
        assert_eq!(gw.send("quick", false).await.status(), 200);
    }
    assert!(seen.windows(2).all(|pair| pair[0] == pair[1]), "{seen:?}");
}
