//! A stream that fails after it started leaves a log line that says why: the provider's message, as one bounded
//! line with anything key-shaped redacted (ADR 0028), not just that it failed.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test scaffolding

mod common;

use std::time::Duration;

use axum::Router;
use axum::response::sse::{Event, Sse};
use axum::routing::post;
use common::{chunk, logs, serve};
use futures::StreamExt;
use humpyard::config::Config;
use humpyard::server;
use serde_json::json;

/// A value shaped like a provider key, assembled so this file holds no key-looking literal.
fn key_shaped() -> String {
    format!("{}{}", "sk-", "abcdefghijklmnopqrstuvwxyz1234")
}

#[derive(Clone, Copy)]
enum Script {
    /// The provider sends an error event, with a key-shaped value in its message, in the middle of the stream.
    ErrorEvent,
}

async fn gateway(script: Script) -> String {
    logs(); // install the capture before anything can log
    let app = Router::new().route(
        "/chat/completions",
        post(move || async move {
            let events = async_stream::stream! {
                yield Ok::<_, std::io::Error>(Event::default().data(
                    chunk(json!({"role": "assistant", "content": "Hel"}), None).to_string()));
                tokio::time::sleep(Duration::from_millis(200)).await;
                match script {
                    Script::ErrorEvent => yield Ok(Event::default().data(
                        json!({"error": {"message": format!("overloaded; key {} rejected", key_shaped())}})
                            .to_string())),
                }
            };
            Sse::new(events)
        }),
    );
    let upstream = format!("http://{}", serve(app).await);
    let toml = format!(
        "listen = \"127.0.0.1:0\"\n\
         [providers.p]\nbase_url = \"{upstream}\"\napi_key_env = \"K\"\nmax_retries = 0\ntimeout_secs = 5\n\
         [targets.t]\nendpoints = [{{ provider = \"p\", model = \"m\" }}]\n\
         [routes.r]\ntype = \"passthrough\"\ntargets = [\"t\"]\n"
    );
    let config = Config::from_toml(&toml, |_| Some("k".into())).unwrap();
    format!(
        "http://{}",
        serve(server::router(config).await.unwrap()).await
    )
}

async fn drain(url: &str) {
    let response = reqwest::Client::new()
        .post(format!("{url}/v1/chat/completions"))
        .json(
            &json!({"model": "r", "stream": true, "messages": [{"role": "user", "content": "hi"}]}),
        )
        .send()
        .await
        .unwrap();
    let mut stream = response.bytes_stream();
    while stream.next().await.is_some() {}
}

/// The log lines that mention `needle`, with colour codes removed.
fn lines_with(needle: &str) -> Vec<String> {
    let plain = |s: &str| {
        let mut out = String::new();
        let mut chars = s.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '\u{1b}' {
                for c in chars.by_ref() {
                    if c == 'm' {
                        break;
                    }
                }
            } else {
                out.push(c);
            }
        }
        out
    };
    logs()
        .contents()
        .lines()
        .map(plain)
        .filter(|l| l.contains(needle))
        .collect()
}

#[tokio::test]
async fn a_provider_error_event_logs_its_message_with_keys_redacted() {
    let url = gateway(Script::ErrorEvent).await;
    drain(&url).await;
    let lines = lines_with("provider sent an error mid-stream");
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(lines[0].contains("overloaded"), "{}", lines[0]);
    assert!(lines[0].contains("[redacted]"), "{}", lines[0]);
    assert!(!lines[0].contains(&key_shaped()[..14]), "{}", lines[0]);
}
