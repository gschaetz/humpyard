//! The per-key concurrency limit (`max_concurrent`): a request holds a permit from admission until its response
//! has ended, and every way a request can end gives the permit back. The upstream here is held open by a gate
//! the test controls, so "in flight" is a state the test can sit in.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test scaffolding

mod common;

use std::convert::Infallible;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use axum::Router;
use axum::extract::State;
use axum::response::IntoResponse;
use axum::response::sse::{Event, Sse};
use axum::routing::post;
use common::{chat_completion, chunk, serve, usage};
use humpyard::config::Config;
use humpyard::server;
use serde_json::{Value, json};
use tokio::sync::watch;

struct Upstream {
    /// Requests that have reached the upstream.
    arrivals: AtomicUsize,
    /// Held-open responses wait until this is `true`.
    gate: watch::Sender<bool>,
}

impl Upstream {
    async fn wait_at_gate(&self) {
        let mut open = self.gate.subscribe();
        open.wait_for(|open| *open).await.unwrap();
    }
}

/// The upstream's model name picks what it does: `hold` streams a chunk, waits at the gate, then finishes;
/// `slowbuf` waits at the gate and answers in one piece; `boom` fails; anything else answers at once.
async fn completions(
    State(up): State<Arc<Upstream>>,
    axum::Json(body): axum::Json<Value>,
) -> axum::response::Response {
    up.arrivals.fetch_add(1, Ordering::SeqCst);
    match body["model"].as_str().unwrap_or("") {
        "hold" => {
            let events = async_stream::stream! {
                yield Ok::<_, Infallible>(Event::default().data(chunk(json!({"role": "assistant", "content": "Hel"}), None).to_string()));
                up.wait_at_gate().await;
                yield Ok(Event::default().data(chunk(json!({"content": "lo"}), None).to_string()));
                yield Ok(Event::default().data(chunk(json!({}), Some("stop")).to_string()));
                yield Ok(Event::default().data("[DONE]"));
            };
            Sse::new(events).into_response()
        }
        "slowbuf" => {
            up.wait_at_gate().await;
            axum::Json(chat_completion("slowbuf", "ok", Some(usage(10, 5)))).into_response()
        }
        "boom" => (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            axum::Json(json!({"error": {"message": "boom"}})),
        )
            .into_response(),
        model => axum::Json(chat_completion(model, "ok", Some(usage(10, 5)))).into_response(),
    }
}

struct Rig {
    url: String,
    upstream: Arc<Upstream>,
    /// One request at a time.
    one: String,
    /// Four at a time.
    four: String,
    /// One at a time and a roomy request rate: both limits on one key.
    both: String,
    /// No limit.
    free: String,
    _dir: tempfile::TempDir,
}

async fn rig() -> Rig {
    let (gate, _) = watch::channel(false);
    let upstream = Arc::new(Upstream {
        arrivals: AtomicUsize::new(0),
        gate,
    });
    let app = Router::new()
        .route("/chat/completions", post(completions))
        .with_state(upstream.clone());
    let base = format!("http://{}", serve(app).await);
    let dir = tempfile::tempdir().unwrap();
    let (one, one_hash) = humpyard::auth::generate_key().unwrap();
    let (four, four_hash) = humpyard::auth::generate_key().unwrap();
    let (both, both_hash) = humpyard::auth::generate_key().unwrap();
    let (free, free_hash) = humpyard::auth::generate_key().unwrap();
    let toml = format!(
        r#"listen = "127.0.0.1:0"
[ledger]
path = "{db}"
[keys.one]
sha256 = "{one_hash}"
max_concurrent = 1
[keys.four]
sha256 = "{four_hash}"
max_concurrent = 4
[keys.both]
sha256 = "{both_hash}"
max_concurrent = 1
max_requests_per_minute = 100
[keys.free]
sha256 = "{free_hash}"
[providers.p]
base_url = "{base}"
api_key_env = "K"
max_retries = 0
[targets.hold]
endpoints = [{{ provider = "p", model = "hold" }}]
[targets.slowbuf]
endpoints = [{{ provider = "p", model = "slowbuf" }}]
[targets.boom]
endpoints = [{{ provider = "p", model = "boom" }}]
[targets.quick]
endpoints = [{{ provider = "p", model = "quick" }}]
"#,
        db = dir.path().join("ledger.db").display(),
    );
    let config = Config::from_toml(&toml, |_| Some("k".into())).unwrap();
    Rig {
        url: format!(
            "http://{}",
            serve(server::router(config).await.unwrap()).await
        ),
        upstream,
        one,
        four,
        both,
        free,
        _dir: dir,
    }
}

impl Rig {
    async fn send(&self, key: &str, route: &str, stream: bool) -> reqwest::Response {
        reqwest::Client::new()
            .post(format!("{}/v1/chat/completions", self.url))
            .bearer_auth(key)
            .json(&json!({"model": route, "stream": stream, "messages": [{"role": "user", "content": "hi"}]}))
            .send()
            .await
            .unwrap()
    }

    async fn status(&self, key: &str, route: &str) -> u16 {
        self.send(key, route, false).await.status().as_u16()
    }

    fn open_gate(&self) {
        self.upstream.gate.send_replace(true);
    }

    fn close_gate(&self) {
        self.upstream.gate.send_replace(false);
    }

    fn arrivals(&self) -> usize {
        self.upstream.arrivals.load(Ordering::SeqCst)
    }

    async fn wait_for_arrivals(&self, n: usize) {
        for _ in 0..200 {
            if self.arrivals() >= n {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!(
            "only {} requests reached the upstream, wanted {n}",
            self.arrivals()
        );
    }

    /// A quick request of `key` is admitted within a few seconds: its permit has come back. (The permit of a
    /// stream the client left comes back when the server notices, which takes a moment.)
    async fn admitted_soon(&self, key: &str) {
        for _ in 0..150 {
            match self.status(key, "quick").await {
                200 => return,
                429 => tokio::time::sleep(Duration::from_millis(20)).await,
                other => panic!("unexpected status {other}"),
            }
        }
        panic!("the key's permit was never returned");
    }

    /// Opens a stream and reads its first chunk, so the request is in flight at the upstream.
    async fn open_stream(&self, key: &str) -> reqwest::Response {
        let mut stream = self.send(key, "hold", true).await;
        assert_eq!(stream.status(), 200);
        let first = stream.chunk().await.unwrap().unwrap();
        assert!(String::from_utf8_lossy(&first).contains("Hel"));
        stream
    }
}

#[tokio::test]
async fn an_open_stream_holds_the_permit_and_the_next_request_is_429_with_retry_after_one() {
    let rig = rig().await;
    let mut stream = rig.open_stream(&rig.one).await;
    assert_eq!(rig.arrivals(), 1);

    let refused = rig.send(&rig.one, "quick", false).await;
    assert_eq!(refused.status(), 429);
    assert_eq!(refused.headers()["retry-after"], "1");
    let body: Value = refused.json().await.unwrap();
    assert_eq!(body["error"]["code"], "rate_limit_exceeded");
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap()
            .contains("in flight"),
        "{body}"
    );
    assert_eq!(rig.arrivals(), 1, "the refused request reached no upstream");

    // The stream itself is unaffected.
    rig.open_gate();
    let mut rest = String::new();
    while let Some(piece) = stream.chunk().await.unwrap() {
        rest.push_str(&String::from_utf8_lossy(&piece));
    }
    assert!(rest.contains("lo") && rest.contains("[DONE]"), "{rest}");
}

#[tokio::test]
async fn finishing_the_stream_frees_the_permit() {
    let rig = rig().await;
    let mut stream = rig.open_stream(&rig.one).await;
    assert_eq!(rig.status(&rig.one, "quick").await, 429);
    rig.open_gate();
    while stream.chunk().await.unwrap().is_some() {}
    rig.admitted_soon(&rig.one).await;
}

#[tokio::test]
async fn a_client_that_disconnects_frees_the_permit() {
    let rig = rig().await;
    let stream = rig.open_stream(&rig.one).await;
    assert_eq!(rig.status(&rig.one, "quick").await, 429);
    drop(stream); // the client goes away mid-stream; the gate stays closed
    rig.admitted_soon(&rig.one).await;
}

#[tokio::test]
async fn an_upstream_error_frees_the_permit() {
    let rig = rig().await;
    let failed = rig.status(&rig.one, "boom").await;
    assert!(failed >= 400, "{failed}");
    assert_eq!(rig.status(&rig.one, "quick").await, 200);
    // And again, so a leak of one per failure would show.
    assert!(rig.status(&rig.one, "boom").await >= 400);
    assert_eq!(rig.status(&rig.one, "quick").await, 200);
}

#[tokio::test]
async fn a_slow_buffered_request_holds_the_permit_until_it_is_sent() {
    let rig = rig().await;
    let url = rig.url.clone();
    let key = rig.one.clone();
    let slow = tokio::spawn(async move {
        reqwest::Client::new()
            .post(format!("{url}/v1/chat/completions"))
            .bearer_auth(key)
            .json(&json!({"model": "slowbuf", "messages": [{"role": "user", "content": "hi"}]}))
            .send()
            .await
            .unwrap()
            .status()
            .as_u16()
    });
    rig.wait_for_arrivals(1).await;
    assert_eq!(rig.status(&rig.one, "quick").await, 429);
    rig.open_gate();
    assert_eq!(slow.await.unwrap(), 200);
    rig.admitted_soon(&rig.one).await;
}

#[tokio::test]
async fn the_in_flight_count_returns_to_zero_after_a_burst() {
    let rig = rig().await;
    // Twenty at once against a limit of four, all held at the closed gate.
    let mut tasks = Vec::new();
    for _ in 0..20 {
        let url = rig.url.clone();
        let key = rig.four.clone();
        tasks.push(tokio::spawn(async move {
            reqwest::Client::new()
                .post(format!("{url}/v1/chat/completions"))
                .bearer_auth(key)
                .json(&json!({"model": "slowbuf", "messages": [{"role": "user", "content": "hi"}]}))
                .send()
                .await
                .unwrap()
                .status()
                .as_u16()
        }));
    }
    rig.wait_for_arrivals(4).await;
    tokio::time::sleep(Duration::from_millis(300)).await; // let the other sixteen be refused
    assert_eq!(rig.arrivals(), 4, "no more than four reached the upstream");
    rig.open_gate();
    let mut statuses = Vec::new();
    for task in tasks {
        statuses.push(task.await.unwrap());
    }
    assert_eq!(statuses.iter().filter(|s| **s == 200).count(), 4);
    assert_eq!(statuses.iter().filter(|s| **s == 429).count(), 16);

    // The count is back to zero: exactly four are admitted again, and a fifth is not.
    rig.close_gate();
    let mut again = Vec::new();
    for _ in 0..4 {
        let url = rig.url.clone();
        let key = rig.four.clone();
        again.push(tokio::spawn(async move {
            reqwest::Client::new()
                .post(format!("{url}/v1/chat/completions"))
                .bearer_auth(key)
                .json(&json!({"model": "slowbuf", "messages": [{"role": "user", "content": "hi"}]}))
                .send()
                .await
                .unwrap()
                .status()
                .as_u16()
        }));
    }
    rig.wait_for_arrivals(8).await;
    assert_eq!(
        rig.status(&rig.four, "quick").await,
        429,
        "a fifth is refused"
    );
    rig.open_gate();
    for task in again {
        assert_eq!(task.await.unwrap(), 200);
    }
}

#[tokio::test]
async fn a_key_without_a_limit_and_another_key_are_unaffected() {
    let rig = rig().await;
    let _stream = rig.open_stream(&rig.one).await;
    assert_eq!(rig.status(&rig.one, "quick").await, 429);
    assert_eq!(rig.status(&rig.four, "quick").await, 200);
    for _ in 0..5 {
        assert_eq!(rig.status(&rig.free, "quick").await, 200);
    }
}

#[tokio::test]
async fn a_key_with_both_limits_has_the_concurrency_limit_enforced_alongside_the_rate() {
    // `both` allows 100 requests a minute and one at a time. Finished requests return their permit, and an open
    // stream still holds the one permit while the rate has plenty of room.
    let rig = rig().await;
    for _ in 0..3 {
        assert_eq!(rig.status(&rig.both, "quick").await, 200);
    }
    let _stream = rig.open_stream(&rig.both).await;
    assert_eq!(rig.status(&rig.both, "quick").await, 429);
}
