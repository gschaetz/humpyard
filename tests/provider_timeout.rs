//! What a provider's `timeout_secs` covers (spec: `provider-pool`, change
//! `bound-the-wait-for-a-response-not-the-whole-stream`, ADR 0031): the wait for a response to start, the idle
//! time of a stream that has started, and the whole of a buffered call. Fake upstreams with a 1 s limit.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test scaffolding

mod common;

use std::convert::Infallible;
use std::time::{Duration, Instant};

use axum::Router;
use axum::extract::Json;
use axum::response::sse::{Event, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use common::{chunk, serve};
use futures::StreamExt;
use humpyard::config::Config;
use humpyard::server;
use serde_json::{Value, json};

/// A chat-completion chunk that carries `text`.
fn text_event(text: &str) -> Event {
    Event::default().data(chunk(json!({"content": text}), None).to_string())
}

/// A gateway with one passthrough route `r` over `upstream`, and a provider `timeout_secs` of `limit`.
async fn gateway(upstream: Router, limit: u64) -> String {
    let upstream = format!("http://{}", serve(upstream).await);
    let toml = format!(
        "listen = \"127.0.0.1:0\"\n\
         [providers.p]\nbase_url = \"{upstream}\"\napi_key_env = \"K\"\nmax_retries = 0\ntimeout_secs = {limit}\n\
         [targets.t]\nendpoints = [{{ provider = \"p\", model = \"m\" }}]\n\
         [routes.r]\ntype = \"passthrough\"\ntargets = [\"t\"]\n"
    );
    let config = Config::from_toml(&toml, |_| Some("k".into())).unwrap();
    format!(
        "http://{}",
        serve(server::router(config).await.unwrap()).await
    )
}

fn request(stream: bool) -> Value {
    json!({"model": "r", "stream": stream, "messages": [{"role": "user", "content": "hi"}]})
}

/// Sends a streaming request; returns when the headers arrived and the whole body.
async fn stream_body(url: &str) -> (Duration, String) {
    let started = Instant::now();
    let response = reqwest::Client::new()
        .post(format!("{url}/v1/chat/completions"))
        .json(&request(true))
        .send()
        .await
        .unwrap();
    let headers_after = started.elapsed();
    let mut stream = response.bytes_stream();
    let mut out = Vec::new();
    while let Some(Ok(bytes)) = stream.next().await {
        out.extend_from_slice(&bytes);
    }
    (headers_after, String::from_utf8_lossy(&out).into_owned())
}

/// The upstream sends SSE keep-alive comments for a while, and only then its first event.
fn keepalive_then_data() -> Response {
    let events = async_stream::stream! {
        for _ in 0..8 {
            yield Ok::<_, Infallible>(Event::default().comment("keep-alive"));
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        yield Ok(text_event("hello"));
        yield Ok(Event::default().data(chunk(json!({}), Some("stop")).to_string()));
        yield Ok(Event::default().data("[DONE]"));
    };
    Sse::new(events).into_response()
}

#[tokio::test]
async fn keepalive_comments_are_not_events_and_a_response_starts_at_its_first_event() {
    let url = gateway(
        Router::new().route(
            "/chat/completions",
            post(|| async { keepalive_then_data() }),
        ),
        30,
    )
    .await;
    let (headers_after, body) = stream_body(&url).await;
    // The call returned only after the first event: the client's headers did not come while only comments had.
    assert!(
        headers_after >= Duration::from_millis(700),
        "headers came after {headers_after:?}, before the first event"
    );
    // The comments were not turned into events.
    assert!(!body.contains("keep-alive"), "{body}");
    assert!(body.contains("hello"), "{body}");
}

// --- the wait for a response to start ------------------------------------------------------------------------

/// A provider that accepts the request and then says nothing.
async fn silent() -> Response {
    tokio::time::sleep(Duration::from_secs(30)).await;
    Json(json!({})).into_response()
}

/// A provider that answers at once, streaming or not.
async fn quick(Json(body): Json<Value>) -> Response {
    if body["stream"] == true {
        let events = async_stream::stream! {
            yield Ok::<_, Infallible>(text_event("hello"));
            yield Ok(Event::default().data(chunk(json!({}), Some("stop")).to_string()));
            yield Ok(Event::default().data("[DONE]"));
        };
        return Sse::new(events).into_response();
    }
    Json(json!({"id": "c", "object": "chat.completion", "created": 1, "model": "m",
        "choices": [{"index": 0, "message": {"role": "assistant", "content": "hello"}, "finish_reason": "stop"}]}))
    .into_response()
}

/// Two providers `slow` and `fast` in one target, `slow` first, both with a `limit` second timeout, and a ledger
/// with key `alice` so the attempts can be read. Returns the gateway URL, the key and the temp dir.
async fn failover_gateway(slow: Router, limit: u64) -> (String, String, tempfile::TempDir) {
    let slow = format!("http://{}", serve(slow).await);
    let fast = format!(
        "http://{}",
        serve(Router::new().route("/chat/completions", post(quick))).await
    );
    let dir = tempfile::tempdir().unwrap();
    let (alice, hash) = humpyard::auth::generate_key().unwrap();
    let toml = format!(
        "listen = \"127.0.0.1:0\"\n[ledger]\npath = \"{}\"\n[keys.alice]\nsha256 = \"{hash}\"\n\
         [providers.slow]\nbase_url = \"{slow}\"\napi_key_env = \"K\"\nmax_retries = 0\ntimeout_secs = {limit}\n\
         [providers.fast]\nbase_url = \"{fast}\"\napi_key_env = \"K\"\nmax_retries = 0\ntimeout_secs = {limit}\n\
         [targets.t]\nendpoints = [{{ provider = \"slow\", model = \"m\" }}, {{ provider = \"fast\", model = \"m\" }}]\n\
         [routes.r]\ntype = \"passthrough\"\ntargets = [\"t\"]\n",
        dir.path().join("ledger.db").display()
    );
    let config = Config::from_toml(&toml, |_| Some("k".into())).unwrap();
    let url = format!(
        "http://{}",
        serve(server::router(config).await.unwrap()).await
    );
    (url, alice, dir)
}

/// The attempts of the newest decision, as `(provider, outcome)`.
async fn attempts(url: &str, key: &str) -> Vec<(String, String)> {
    let body: Value = reqwest::Client::new()
        .get(format!("{url}/v1/decisions?limit=1"))
        .bearer_auth(key)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    body["data"][0]["attempts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| {
            (
                a["provider"].as_str().unwrap().to_string(),
                a["outcome"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

async fn ask(url: &str, key: &str, stream: bool) -> (Duration, reqwest::Response) {
    let started = Instant::now();
    let response = reqwest::Client::new()
        .post(format!("{url}/v1/chat/completions"))
        .bearer_auth(key)
        .json(&request(stream))
        .send()
        .await
        .unwrap();
    (started.elapsed(), response)
}

#[tokio::test]
async fn a_provider_that_never_starts_fails_over_within_the_limit_streamed_and_buffered() {
    let (url, key, _dir) = failover_gateway(
        Router::new().route("/chat/completions", post(|| async { silent().await })),
        1,
    )
    .await;
    for stream in [true, false] {
        let (took, response) = ask(&url, &key, stream).await;
        assert_eq!(response.status(), 200, "stream={stream}");
        assert!(
            took >= Duration::from_millis(900) && took < Duration::from_secs(4),
            "failed over after {took:?} (stream={stream})"
        );
        assert_eq!(response.headers()["x-humpyard-provider"], "fast");
        let body = response.text().await.unwrap();
        assert!(body.contains("hello"), "{body}");
        assert_eq!(
            attempts(&url, &key).await,
            [
                ("slow".to_string(), "timeout".to_string()),
                ("fast".to_string(), "ok".to_string())
            ],
            "stream={stream}"
        );
    }
}

#[tokio::test]
async fn a_provider_that_answers_in_time_is_not_affected() {
    let url = gateway(Router::new().route("/chat/completions", post(quick)), 1).await;
    let (_, streamed) = stream_body(&url).await;
    assert!(streamed.contains("hello"), "{streamed}");
}

// --- the idle limit on a started stream ----------------------------------------------------------------------

/// A stream whose events arrive `first` after the request and then every `gap`, `count` in all, then a normal
/// finish. With `silent_after` set it sends that many events and goes quiet for 30 s instead of finishing.
fn paced(first: Duration, gap: Duration, count: usize, silent_after: Option<usize>) -> Router {
    Router::new().route(
        "/chat/completions",
        post(move || async move {
            let events = async_stream::stream! {
                tokio::time::sleep(first).await;
                for n in 0..count {
                    if silent_after == Some(n) {
                        tokio::time::sleep(Duration::from_secs(30)).await;
                    }
                    yield Ok::<_, Infallible>(text_event(&format!("part{n} ")));
                    tokio::time::sleep(gap).await;
                }
                yield Ok(Event::default().data(chunk(json!({}), Some("stop")).to_string()));
                yield Ok(Event::default().data("[DONE]"));
            };
            Sse::new(events)
        }),
    )
}

#[tokio::test]
async fn a_stream_longer_than_the_limit_that_keeps_producing_completes() {
    // 8 events 400 ms apart: about 3.2 s in all against a 1 s limit, never silent for more than 400 ms.
    let url = gateway(
        paced(
            Duration::from_millis(100),
            Duration::from_millis(400),
            8,
            None,
        ),
        1,
    )
    .await;
    let started = Instant::now();
    let (_, body) = stream_body(&url).await;
    assert!(
        started.elapsed() > Duration::from_millis(2500),
        "the stream was short"
    );
    for n in 0..8 {
        assert!(
            body.contains(&format!("part{n} ")),
            "part{n} missing: {body}"
        );
    }
    assert!(body.trim_end().ends_with("[DONE]"), "{body}");
    assert!(!body.contains("idle"), "{body}");
}

#[tokio::test]
async fn a_slow_start_followed_by_a_steady_stream_is_not_ended_when_the_wait_would_have_run_out() {
    // The first event comes at 800 ms (inside the 1 s wait); the stream then runs on to about 3 s.
    let url = gateway(
        paced(
            Duration::from_millis(800),
            Duration::from_millis(400),
            6,
            None,
        ),
        1,
    )
    .await;
    let (headers_after, body) = stream_body(&url).await;
    assert!(
        headers_after >= Duration::from_millis(700),
        "{headers_after:?}"
    );
    for n in 0..6 {
        assert!(
            body.contains(&format!("part{n} ")),
            "part{n} missing: {body}"
        );
    }
    assert!(body.trim_end().ends_with("[DONE]"), "{body}");
}

#[tokio::test]
async fn a_stream_that_goes_silent_mid_answer_ends_with_the_timeout_error() {
    logs_on();
    let dir = tempfile::tempdir().unwrap();
    let upstream = format!(
        "http://{}",
        serve(paced(
            Duration::from_millis(50),
            Duration::from_millis(50),
            4,
            Some(2)
        ))
        .await
    );
    let toml = format!(
        "listen = \"127.0.0.1:0\"\n[ledger]\npath = \"{}\"\n\
         [providers.p]\nbase_url = \"{upstream}\"\napi_key_env = \"K\"\nmax_retries = 0\ntimeout_secs = 1\n\
         [targets.t]\nendpoints = [{{ provider = \"p\", model = \"m\" }}]\n\
         [routes.r]\ntype = \"passthrough\"\ntargets = [\"t\"]\n",
        dir.path().join("ledger.db").display()
    );
    let config = Config::from_toml(&toml, |_| Some("k".into())).unwrap();
    let url = format!(
        "http://{}",
        serve(server::router(config).await.unwrap()).await
    );
    let started = Instant::now();
    let (_, body) = stream_body(&url).await;
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "ended after {:?}",
        started.elapsed()
    );
    assert!(body.contains("part0 ") && body.contains("part1 "), "{body}");
    assert!(!body.contains("part2 "), "{body}");
    assert!(
        body.contains("upstream request timed out: stream was idle for 1000 ms"),
        "{body}"
    );
    // The log line says why the stream failed.
    assert!(
        common::logs()
            .contents()
            .lines()
            .any(|l| l.contains("stream iteration failed")
                && l.contains("stream was idle for 1000 ms")),
        "no log line names the idle limit"
    );
    // The call is recorded as failed.
    let ledger = humpyard::ledger::Ledger::open(&dir.path().join("ledger.db"))
        .await
        .unwrap();
    for _ in 0..200 {
        if let Some(entry) = ledger.entries_since(0).await.unwrap().into_iter().next() {
            assert_eq!(entry.outcome, "error");
            return;
        }
        tokio::time::sleep(Duration::from_millis(30)).await;
    }
    panic!("no usage row was written");
}

/// Installs the log capture the log-line tests read.
fn logs_on() {
    let _ = common::logs();
}

// --- what a timeout says -------------------------------------------------------------------------------------

/// The recorded message of the first attempt of the newest decision.
async fn first_attempt_message(url: &str, key: &str) -> String {
    let body: Value = reqwest::Client::new()
        .get(format!("{url}/v1/decisions?limit=1"))
        .bearer_auth(key)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    body["data"][0]["attempts"][0]["message"]
        .as_str()
        .unwrap_or_default()
        .to_string()
}

#[tokio::test]
async fn a_timeout_names_the_limit_that_ran_out_and_stays_a_timeout() {
    let (url, key, _dir) = failover_gateway(
        Router::new().route("/chat/completions", post(|| async { silent().await })),
        1,
    )
    .await;
    // A streamed request that never starts.
    ask(&url, &key, true).await.1.text().await.unwrap();
    assert!(
        first_attempt_message(&url, &key)
            .await
            .contains("no response started within 1000 ms, retries included"),
    );
    assert_eq!(attempts(&url, &key).await[0].1, "timeout");
    // A buffered request that never finishes.
    ask(&url, &key, false).await.1.text().await.unwrap();
    assert!(
        first_attempt_message(&url, &key)
            .await
            .contains("response did not finish within 1000 ms, retries included"),
    );
    assert_eq!(attempts(&url, &key).await[0].1, "timeout");
}

// --- a buffered answer is bounded as a whole -----------------------------------------------------------------

#[tokio::test]
async fn a_buffered_calls_limit_includes_the_providers_retries() {
    // Every attempt takes 600 ms and fails with a 500, and the provider may retry three times: left alone that
    // is over four seconds. The limit covers the retries, so the call ends near one second as a timeout.
    let upstream = format!(
        "http://{}",
        serve(Router::new().route(
            "/chat/completions",
            post(|| async {
                tokio::time::sleep(Duration::from_millis(600)).await;
                (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "boom")
            }),
        ))
        .await
    );
    let toml = format!(
        "listen = \"127.0.0.1:0\"\n\
         [providers.p]\nbase_url = \"{upstream}\"\napi_key_env = \"K\"\nmax_retries = 3\ntimeout_secs = 1\n\
         [targets.t]\nendpoints = [{{ provider = \"p\", model = \"m\" }}]\n\
         [routes.r]\ntype = \"passthrough\"\ntargets = [\"t\"]\n"
    );
    let config = Config::from_toml(&toml, |_| Some("k".into())).unwrap();
    let url = format!(
        "http://{}",
        serve(server::router(config).await.unwrap()).await
    );
    let started = Instant::now();
    let response = reqwest::Client::new()
        .post(format!("{url}/v1/chat/completions"))
        .json(&request(false))
        .send()
        .await
        .unwrap();
    let took = started.elapsed();
    assert_eq!(response.status(), 504, "{}", response.text().await.unwrap());
    assert!(
        took >= Duration::from_millis(900) && took < Duration::from_millis(2500),
        "ended after {took:?}"
    );
}
