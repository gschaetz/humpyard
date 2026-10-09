//! Hot reload: the running gateway re-reads its config file on request, swaps in the new
//! configuration atomically, never lets a bad file take effect, refuses settings that need a
//! restart, and keeps the state of what did not change.
//!
//! `run` is driven exactly as the binary drives it, with a trigger channel standing in for SIGHUP.
//! Provider keys come from `PATH`, an environment variable every process has, so reloads (which
//! read the real environment) work without mutating it.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test scaffolding

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use axum::Router;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::sse::{Event, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use common::{chat_completion, chat_request, chunk, serve, usage};
use humpyard::config::Config;
use humpyard::server::{self, Reload};
use serde_json::{Value, json};
use tokio::sync::mpsc;

/// An upstream that answers with its own label; `fail` makes it answer 503; streams are slow.
struct Upstream {
    label: &'static str,
    fail: std::sync::atomic::AtomicBool,
    calls: AtomicUsize,
}

async fn completions(
    State(up): State<Arc<Upstream>>,
    axum::Json(body): axum::Json<Value>,
) -> Response {
    up.calls.fetch_add(1, Ordering::SeqCst);
    if up.fail.load(Ordering::SeqCst) {
        return (StatusCode::SERVICE_UNAVAILABLE, "down").into_response();
    }
    if body["stream"] == true {
        let label = up.label;
        let events = async_stream::stream! {
            for part in 0..3 {
                let text = format!("{label}{part} ");
                yield Ok::<_, std::io::Error>(Event::default().data(
                    chunk(json!({"role": "assistant", "content": text}), None).to_string()));
                tokio::time::sleep(Duration::from_millis(300)).await;
            }
            yield Ok(Event::default().data(
                chunk(json!({}), Some("stop")).to_string()));
            yield Ok(Event::default().data(
                json!({"id": "c", "object": "chat.completion.chunk", "created": 1, "model": "m",
                       "choices": [], "usage": usage(10, 5)}).to_string()));
            yield Ok(Event::default().data("[DONE]"));
        };
        return Sse::new(events).into_response();
    }
    axum::Json(chat_completion("m", up.label, Some(usage(10, 5)))).into_response()
}

async fn upstream(label: &'static str) -> (Arc<Upstream>, String) {
    let up = Arc::new(Upstream {
        label,
        fail: std::sync::atomic::AtomicBool::new(false),
        calls: AtomicUsize::new(0),
    });
    let app = Router::new()
        .route("/chat/completions", post(completions))
        .with_state(up.clone());
    (up, format!("http://{}", serve(app).await))
}

struct Gateway {
    url: String,
    path: std::path::PathBuf,
    reload: mpsc::Sender<()>,
    stop: mpsc::Sender<()>,
    task: tokio::task::JoinHandle<Result<(), String>>,
    key: Option<String>,
    _dir: tempfile::TempDir,
}

/// Providers `pa` and `pb` (key from `PATH`), targets `a` and `b`, routes `r1` (a) and `r2` (b);
/// `head` goes before the tables (top-level settings and tables like `[ledger]`), `tail` after.
fn config_text(head: &str, a_url: &str, b_url: &str, tail: &str) -> String {
    format!(
        r#"listen = "127.0.0.1:0"
{head}
[providers.pa]
base_url = "{a_url}"
api_key_env = "PATH"
max_retries = 0
[providers.pb]
base_url = "{b_url}"
api_key_env = "PATH"
max_retries = 0
[targets.a]
endpoints = [{{ provider = "pa", model = "ma" }}]
[targets.b]
endpoints = [{{ provider = "pb", model = "mb" }}]
[routes.r1]
type = "passthrough"
targets = ["a"]
[routes.r2]
type = "passthrough"
targets = ["b"]
{tail}
"#
    )
}

async fn start(text: &str, key: Option<String>) -> Gateway {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("humpyard.toml");
    std::fs::write(&path, text).unwrap();
    let config = Config::from_toml(text, |n| std::env::var(n).ok()).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let (reload, triggers) = mpsc::channel(4);
    let (stop, signals) = mpsc::channel(4);
    let task = tokio::spawn(server::run(
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
        path,
        reload,
        stop,
        task,
        key,
        _dir: dir,
    }
}

impl Gateway {
    fn write(&self, text: &str) {
        std::fs::write(&self.path, text).unwrap();
    }

    async fn reload_now(&self) {
        self.reload.send(()).await.unwrap();
    }

    async fn get_health(&self) -> Value {
        let mut req = reqwest::Client::new().get(format!("{}/v1/health", self.url));
        if let Some(key) = &self.key {
            req = req.bearer_auth(key);
        }
        req.send().await.unwrap().json().await.unwrap()
    }

    async fn fingerprint(&self) -> String {
        self.get_health().await["config"]["fingerprint"]
            .as_str()
            .unwrap()
            .to_string()
    }

    /// Waits until more reload attempts have finished than `before` and returns the health report.
    async fn after_reload(&self, before: u64) -> Value {
        for _ in 0..100 {
            let health = self.get_health().await;
            if health["reload"]["attempts"].as_u64().unwrap_or(0) > before {
                return health;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("the reload did not finish");
    }

    async fn ask(&self, model: &str, headers: &[(&str, &str)]) -> (u16, String) {
        let mut req = reqwest::Client::new()
            .post(format!("{}/v1/chat/completions", self.url))
            .json(&chat_request(model, false));
        if let Some(key) = &self.key {
            req = req.bearer_auth(key);
        }
        for (k, v) in headers {
            req = req.header(*k, *v);
        }
        let resp = req.send().await.unwrap();
        let status = resp.status().as_u16();
        let body: Value = resp.json().await.unwrap_or(Value::Null);
        (
            status,
            body["choices"][0]["message"]["content"]
                .as_str()
                .unwrap_or("")
                .to_string(),
        )
    }

    async fn shut_down(self) {
        self.stop.send(()).await.unwrap();
        let _ = tokio::time::timeout(Duration::from_secs(10), self.task).await;
    }
}

const RULE_TO_B: &str = r#"
[[select]]
name = "to-b"
when = { header = { "x-case" = "b" } }
route = "r2"
"#;

#[tokio::test]
async fn an_edited_rule_takes_effect_without_a_restart() {
    let (_a, a_url) = upstream("A").await;
    let (_b, b_url) = upstream("B").await;
    let g = start(&config_text("", &a_url, &b_url, ""), None).await;
    let before = g.fingerprint().await;
    assert_eq!(before.len(), 12);
    assert_eq!(
        g.ask("r1", &[("x-case", "b")]).await,
        (200, "A".into()),
        "no rule yet"
    );
    let health = g.get_health().await;
    assert_eq!(health["reload"]["ok"], Value::Null, "nothing reloaded yet");

    g.write(&config_text("", &a_url, &b_url, RULE_TO_B));
    g.reload_now().await;
    let health = g.after_reload(0).await;
    assert_eq!(health["reload"]["ok"], true, "{health}");
    assert_ne!(health["config"]["fingerprint"].as_str().unwrap(), before);
    assert_eq!(
        g.ask("r1", &[("x-case", "b")]).await,
        (200, "B".into()),
        "the rule now applies"
    );
    assert_eq!(g.ask("r1", &[]).await, (200, "A".into()));
    g.shut_down().await;
}

#[tokio::test]
async fn a_bad_file_never_takes_effect_and_a_fixed_one_does() {
    let (_a, a_url) = upstream("A").await;
    let (_b, b_url) = upstream("B").await;
    let good = config_text("", &a_url, &b_url, "");
    let g = start(&good, None).await;
    let before = g.fingerprint().await;

    // A rule naming a route that does not exist is invalid.
    g.write(&config_text(
        "",
        &a_url,
        &b_url,
        "[[select]]\nroute = \"nowhere\"\n",
    ));
    g.reload_now().await;
    let health = g.after_reload(0).await;
    assert_eq!(health["reload"]["ok"], false, "{health}");
    assert!(
        health["reload"]["message"]
            .as_str()
            .unwrap()
            .contains("nowhere"),
        "{health}"
    );
    assert_eq!(
        health["config"]["fingerprint"],
        before.as_str(),
        "the old config is still loaded"
    );
    assert_eq!(
        g.ask("r1", &[]).await,
        (200, "A".into()),
        "and still serving"
    );

    // So is a file that is not even TOML, or missing.
    let attempt = health["reload"]["attempts"].as_u64().unwrap_or(0);
    g.write("this is = not [valid");
    g.reload_now().await;
    let health = g.after_reload(attempt).await;
    assert_eq!(health["reload"]["ok"], false);
    assert_eq!(g.fingerprint().await, before);

    // Fixing the file is enough; no restart.
    let attempt = health["reload"]["attempts"].as_u64().unwrap_or(0);
    g.write(&config_text("", &a_url, &b_url, RULE_TO_B));
    g.reload_now().await;
    let health = g.after_reload(attempt).await;
    assert_eq!(health["reload"]["ok"], true, "{health}");
    assert_eq!(g.ask("r1", &[("x-case", "b")]).await, (200, "B".into()));
    g.shut_down().await;
}

#[tokio::test]
async fn settings_that_need_a_restart_refuse_the_whole_reload() {
    let (_a, a_url) = upstream("A").await;
    let (_b, b_url) = upstream("B").await;
    let g = start(&config_text("", &a_url, &b_url, ""), None).await;
    let before = g.fingerprint().await;

    // The edit also contains a perfectly good rule: nothing of it may apply.
    let mut text = config_text("", &a_url, &b_url, RULE_TO_B);
    text = text.replace("127.0.0.1:0", "127.0.0.1:9");
    g.write(&text);
    g.reload_now().await;
    let health = g.after_reload(0).await;
    assert_eq!(health["reload"]["ok"], false);
    let message = health["reload"]["message"].as_str().unwrap();
    assert!(
        message.contains("listen") && message.contains("restart"),
        "{message}"
    );
    assert_eq!(g.fingerprint().await, before);
    assert_eq!(
        g.ask("r1", &[("x-case", "b")]).await,
        (200, "A".into()),
        "the good rule did not apply either"
    );

    let attempt = health["reload"]["attempts"].as_u64().unwrap_or(0);
    let mut grace = config_text("", &a_url, &b_url, "");
    grace = grace.replace("listen =", "shutdown_grace_secs = 5\nlisten =");
    g.write(&grace);
    g.reload_now().await;
    let health = g.after_reload(attempt).await;
    assert!(
        health["reload"]["message"]
            .as_str()
            .unwrap()
            .contains("shutdown_grace_secs")
    );

    // Turning keys on for a keyless gateway is a restart too.
    let attempt = health["reload"]["attempts"].as_u64().unwrap_or(0);
    let (_key, hash) = humpyard::auth::generate_key().unwrap();
    g.write(&config_text(
        &format!("[keys.k]\nsha256 = \"{hash}\""),
        &a_url,
        &b_url,
        "",
    ));
    g.reload_now().await;
    let health = g.after_reload(attempt).await;
    assert_eq!(health["reload"]["ok"], false);
    assert!(
        health["reload"]["message"]
            .as_str()
            .unwrap()
            .contains("keys")
    );
    g.shut_down().await;
}

#[tokio::test]
async fn a_running_stream_finishes_on_the_configuration_it_started_with() {
    let (_a, a_url) = upstream("A").await;
    let (_b, b_url) = upstream("B").await;
    let g = start(&config_text("", &a_url, &b_url, ""), None).await;
    let stream = reqwest::Client::new()
        .post(format!("{}/v1/chat/completions", g.url))
        .json(&chat_request("r1", true))
        .send()
        .await
        .unwrap();
    assert_eq!(stream.status(), 200);

    // While it is streaming from A, repoint route r1 at B and reload.
    let repointed = config_text("", &a_url, &b_url, "").replace(
        "[routes.r1]\ntype = \"passthrough\"\ntargets = [\"a\"]",
        "[routes.r1]\ntype = \"passthrough\"\ntargets = [\"b\"]",
    );
    g.write(&repointed);
    g.reload_now().await;
    let health = g.after_reload(0).await;
    assert_eq!(health["reload"]["ok"], true, "{health}");
    assert_eq!(
        g.ask("r1", &[]).await,
        (200, "B".into()),
        "new requests follow the new config"
    );

    let text = stream.text().await.unwrap();
    assert!(text.contains("A0") && text.contains("A2"), "{text}");
    assert!(!text.contains('B'), "the stream stayed on A: {text}");
    g.shut_down().await;
}

#[tokio::test]
async fn endpoint_health_survives_an_unrelated_reload() {
    let (a, a_url) = upstream("A").await;
    let (_b, b_url) = upstream("B").await;
    let (key, hash) = humpyard::auth::generate_key().unwrap();
    // One failure opens the breaker for a minute; route `fb` reaches `a` and falls back to `b`.
    let head = format!(
        "[health]\nfailure_threshold = 1\ncooldown_secs = 60\n[keys.k]\nsha256 = \"{hash}\"\n"
    );
    let tail = "[routes.fb]\ntype = \"passthrough\"\ntargets = [\"a\", \"b\"]\n";
    let g = start(&config_text(&head, &a_url, &b_url, tail), Some(key)).await;
    a.fail.store(true, Ordering::SeqCst);
    assert_eq!(
        g.ask("fb", &[]).await,
        (200, "B".into()),
        "a fails, b serves"
    );
    assert_eq!(h_target(&g.get_health().await, "a"), "cooling_down");

    // An unrelated edit (a new rule) must not make the gateway forget that `a` is cooling down.
    g.write(&config_text(
        &head,
        &a_url,
        &b_url,
        &format!("{tail}{RULE_TO_B}"),
    ));
    g.reload_now().await;
    assert_eq!(g.after_reload(0).await["reload"]["ok"], true);
    assert_eq!(
        h_target(&g.get_health().await, "a"),
        "cooling_down",
        "health carried over"
    );
    g.shut_down().await;
}

fn h_target(health: &Value, target: &str) -> String {
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

#[tokio::test]
async fn a_changed_endpoint_starts_fresh_and_keys_reload_with_the_config() {
    let (failing, a_url) = upstream("A").await;
    let (_, moved_url) = upstream("A2").await;
    let (_b, b_url) = upstream("B").await;
    let (k1, h1) = humpyard::auth::generate_key().unwrap();
    let (k2, h2) = humpyard::auth::generate_key().unwrap();
    let head = |hashes: &[&str]| {
        let keys: Vec<String> = hashes
            .iter()
            .enumerate()
            .map(|(i, h)| format!("[keys.k{i}]\nsha256 = \"{h}\"\n"))
            .collect();
        format!(
            "[health]\nfailure_threshold = 1\ncooldown_secs = 60\n{}",
            keys.join("")
        )
    };
    let tail = "[routes.fb]\ntype = \"passthrough\"\ntargets = [\"a\", \"b\"]\n";
    let g = start(
        &config_text(&head(&[&h1]), &a_url, &b_url, tail),
        Some(k1.clone()),
    )
    .await;
    failing.fail.store(true, Ordering::SeqCst);
    assert_eq!(g.ask("fb", &[]).await, (200, "B".into()));
    assert_eq!(h_target(&g.get_health().await, "a"), "cooling_down");

    // The same target now points at a different URL: a different endpoint, so a clean slate.
    // A second key is added at the same time; the new key works without a restart.
    g.write(&config_text(&head(&[&h1, &h2]), &moved_url, &b_url, tail));
    g.reload_now().await;
    assert_eq!(g.after_reload(0).await["reload"]["ok"], true);
    assert_eq!(h_target(&g.get_health().await, "a"), "healthy");
    assert_eq!(
        g.ask("fb", &[]).await,
        (200, "A2".into()),
        "served by the new URL"
    );
    let with_new_key = reqwest::Client::new()
        .post(format!("{}/v1/chat/completions", g.url))
        .bearer_auth(&k2)
        .json(&chat_request("fb", false))
        .send()
        .await
        .unwrap();
    assert_eq!(with_new_key.status(), 200);

    // And removing it again revokes it.
    let attempt = g.get_health().await["reload"]["attempts"]
        .as_u64()
        .unwrap_or(0);
    g.write(&config_text(&head(&[&h1]), &moved_url, &b_url, tail));
    g.reload_now().await;
    g.after_reload(attempt).await;
    let revoked = reqwest::Client::new()
        .post(format!("{}/v1/chat/completions", g.url))
        .bearer_auth(&k2)
        .json(&chat_request("fb", false))
        .send()
        .await
        .unwrap();
    assert_eq!(revoked.status(), 401);
    g.shut_down().await;
}

#[tokio::test]
async fn budget_counters_survive_a_reload_and_new_limits_apply() {
    let (_a, a_url) = upstream("A").await;
    let (_b, b_url) = upstream("B").await;
    let dir = tempfile::tempdir().unwrap();
    let ledger = dir.path().join("ledger.db");
    let (key, hash) = humpyard::auth::generate_key().unwrap();
    let head = |limit: u64| {
        format!(
            "[ledger]\npath = \"{}\"\n[keys.k]\nsha256 = \"{hash}\"\ndaily_tokens = {limit}\n",
            ledger.display()
        )
    };
    // Each request costs 15 tokens; the limit of 25 allows two (the second ends over it).
    let g = start(&config_text(&head(25), &a_url, &b_url, ""), Some(key)).await;
    assert_eq!(g.ask("r1", &[]).await.0, 200);

    // An unrelated reload must not reset what the key already spent.
    g.write(&config_text(&head(25), &a_url, &b_url, RULE_TO_B));
    g.reload_now().await;
    assert_eq!(g.after_reload(0).await["reload"]["ok"], true);
    assert_eq!(
        g.ask("r1", &[]).await.0,
        200,
        "15 + 15 = 30 tokens, over the limit now"
    );
    assert_eq!(
        g.ask("r1", &[]).await.0,
        402,
        "the counters were kept across the reload"
    );

    // Raising the limit takes effect at once.
    let attempt = g.get_health().await["reload"]["attempts"]
        .as_u64()
        .unwrap_or(0);
    g.write(&config_text(&head(1000), &a_url, &b_url, RULE_TO_B));
    g.reload_now().await;
    g.after_reload(attempt).await;
    assert_eq!(g.ask("r1", &[]).await.0, 200);
    g.shut_down().await;
}

#[tokio::test]
async fn a_burst_of_reload_requests_ends_on_the_latest_file() {
    let (_a, a_url) = upstream("A").await;
    let (_b, b_url) = upstream("B").await;
    let g = start(&config_text("", &a_url, &b_url, ""), None).await;
    g.write(&config_text("", &a_url, &b_url, RULE_TO_B));
    for _ in 0..4 {
        let _ = g.reload.try_send(());
    }
    g.after_reload(0).await;
    // Whatever was merged, the end state is the latest file, and serving is unaffected.
    for _ in 0..40 {
        if g.ask("r1", &[("x-case", "b")]).await.1 == "B" {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(g.ask("r1", &[("x-case", "b")]).await, (200, "B".into()));
    g.shut_down().await;
}

#[tokio::test]
async fn polling_picks_up_an_edited_file_without_any_signal_and_tries_a_bad_one_only_once() {
    let (_a, a_url) = upstream("A").await;
    let (_b, b_url) = upstream("B").await;
    // No trigger is ever sent: this is how a distroless container, where nothing can send SIGHUP,
    // picks up a changed ConfigMap.
    let head = "reload_poll_secs = 1";
    let g = start(&config_text(head, &a_url, &b_url, ""), None).await;
    let before = g.fingerprint().await;
    g.write(&config_text(head, &a_url, &b_url, RULE_TO_B));
    let health = g.after_reload(0).await;
    assert_eq!(health["reload"]["ok"], true, "{health}");
    assert_ne!(health["config"]["fingerprint"].as_str().unwrap(), before);
    assert_eq!(g.ask("r1", &[("x-case", "b")]).await, (200, "B".into()));

    // An unchanged file is not reloaded again, and a broken one is attempted once, not every second.
    let attempts = health["reload"]["attempts"].as_u64().unwrap();
    g.write("broken [");
    tokio::time::sleep(Duration::from_millis(3500)).await;
    let health = g.get_health().await;
    assert_eq!(health["reload"]["ok"], false, "{health}");
    assert_eq!(health["reload"]["attempts"].as_u64().unwrap(), attempts + 1);
    assert_eq!(
        g.ask("r1", &[("x-case", "b")]).await,
        (200, "B".into()),
        "still serving"
    );
    g.shut_down().await;
}

#[test]
fn reload_poll_secs_is_validated_and_off_by_default() {
    let base = config_text("", "http://127.0.0.1:1", "http://127.0.0.1:2", "");
    let off = Config::from_toml(&base, |_| Some("k".into())).unwrap();
    assert_eq!(off.reload_poll_secs, 0);
    let on = base.replace("listen =", "reload_poll_secs = 30\nlisten =");
    assert_eq!(
        Config::from_toml(&on, |_| Some("k".into()))
            .unwrap()
            .reload_poll_secs,
        30
    );
    let too_slow = base.replace("listen =", "reload_poll_secs = 3601\nlisten =");
    let message = Config::from_toml(&too_slow, |_| Some("k".into()))
        .err()
        .unwrap()
        .to_string();
    assert!(message.contains("reload_poll_secs"), "{message}");
}
