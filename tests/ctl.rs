//! `humpyard ctl` (spec: `cli`): a read-only client for a running gateway, tested against a real in-process one.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test scaffolding

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use axum::Router;
use axum::http::{StatusCode, header};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use common::{chat_completion, chat_request, serve, usage};
use humpyard::config::Config;
use humpyard::ctl::{Command, Failure, Settings, connect, run};
use humpyard::server;

struct Fixture {
    url: String,
    key: String,
    _dir: std::path::PathBuf,
}

async fn fixture() -> Fixture {
    let upstream = serve(Router::new().route(
        "/chat/completions",
        post(|| async { axum::Json(chat_completion("m", "ok", Some(usage(20, 5)))) }),
    ))
    .await;
    let dir = common::scratch().join(format!(
        "humpyard-ctl-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let (key, hash) = humpyard::auth::generate_key().unwrap();
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
sha256 = "{hash}"
"#,
        dir.join("ledger.db").display()
    );
    let config = Config::from_toml(&toml, |n| std::env::var(n).ok()).unwrap();
    let addr = serve(server::router(config).await.unwrap()).await;
    let url = format!("http://{addr}");
    // One real request, so stats and decisions have something to show.
    let r = reqwest::Client::new()
        .post(format!("{url}/v1/chat/completions"))
        .bearer_auth(&key)
        .json(&chat_request("r", false))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    Fixture {
        url,
        key,
        _dir: dir,
    }
}

fn settings(url: &str) -> Settings {
    Settings {
        url: Some(url.into()),
        key_env: "CTL_KEY".into(),
        allow_insecure_http: false,
        json: false,
    }
}

async fn ctl(
    url: &str,
    key: Option<&str>,
    command: Command,
    json: bool,
) -> (Result<(), Failure>, String) {
    let env = |n: &str| (n == "CTL_KEY").then(|| key.map(String::from)).flatten();
    let gateway = match connect(&settings(url), &env) {
        Ok(g) => g,
        Err(f) => return (Err(f), String::new()),
    };
    let mut out: Vec<u8> = Vec::new();
    let result = run(&gateway, command, json, &mut out).await;
    (result, String::from_utf8(out).unwrap())
}

fn stats() -> Command {
    Command::Stats {
        window: None,
        group_by: None,
        route: None,
        target: None,
    }
}

fn decisions() -> Command {
    Command::Decisions {
        limit: Some(5),
        route: None,
        rule: None,
        outcome: None,
        request_id: None,
    }
}

#[tokio::test]
async fn each_read_command_shows_what_the_gateway_says() {
    let f = fixture().await;
    let (r, out) = ctl(&f.url, Some(&f.key), Command::Models, false).await;
    r.unwrap();
    assert!(
        out.lines().any(|l| l == "r") && out.lines().any(|l| l == "t"),
        "{out}"
    );
    let (r, out) = ctl(&f.url, Some(&f.key), Command::Whoami, false).await;
    r.unwrap();
    assert!(out.contains("id: alice"), "{out}");
    let (r, out) = ctl(&f.url, Some(&f.key), Command::Health, false).await;
    r.unwrap();
    assert!(out.contains("config ") && out.contains("p/m"), "{out}");
    let (r, out) = ctl(&f.url, Some(&f.key), stats(), false).await;
    r.unwrap();
    assert!(out.contains("GROUP") && out.contains("REQUESTS"), "{out}");
    let (r, out) = ctl(&f.url, Some(&f.key), decisions(), false).await;
    r.unwrap();
    assert!(out.contains("REQUEST") && out.contains(" r "), "{out}");
    let explain = Command::Explain {
        model: Some("r".into()),
        prompt_tokens: Some(100),
        tools: false,
        images: false,
        subagent: false,
        body_file: None,
    };
    let (r, out) = ctl(&f.url, Some(&f.key), explain, false).await;
    r.unwrap();
    assert!(out.contains("route: r"), "{out}");
}

#[tokio::test]
async fn status_summarises_and_never_shows_the_key() {
    let f = fixture().await;
    let (r, out) = ctl(&f.url, Some(&f.key), Command::Status, false).await;
    r.unwrap();
    assert!(
        out.contains("reachable")
            && out.contains("endpoints being skipped: 0")
            && out.contains("key: alice"),
        "{out}"
    );
    assert!(!out.contains(&f.key));
}

#[tokio::test]
async fn json_output_is_the_gateways_body_unchanged() {
    let f = fixture().await;
    let direct = reqwest::Client::new()
        .get(format!("{}/v1/models", f.url))
        .bearer_auth(&f.key)
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    let (r, out) = ctl(&f.url, Some(&f.key), Command::Models, true).await;
    r.unwrap();
    assert_eq!(out.trim_end(), direct.trim_end());
}

#[tokio::test]
async fn a_wrong_key_is_a_refusal_with_exit_one_and_the_key_stays_out_of_the_message() {
    let f = fixture().await;
    let (r, out) = ctl(&f.url, Some("not-the-key"), Command::Whoami, false).await;
    let failure = r.unwrap_err();
    assert_eq!(failure.exit_code(), 1);
    assert!(failure.message().contains("401") && !failure.message().contains("not-the-key"));
    assert_eq!(out, "");
}

#[tokio::test]
async fn an_unreachable_gateway_is_exit_two() {
    let (r, _) = ctl("http://127.0.0.1:1", Some("k"), Command::Whoami, false).await;
    assert_eq!(r.unwrap_err().exit_code(), 2);
}

#[tokio::test]
async fn an_unset_key_sends_nothing() {
    let hits = Arc::new(AtomicUsize::new(0));
    let counter = hits.clone();
    let app = Router::new().route(
        "/v1/key/info",
        get(move || {
            let counter = counter.clone();
            async move {
                counter.fetch_add(1, Ordering::SeqCst);
                StatusCode::OK
            }
        }),
    );
    let addr = serve(app).await;
    let (r, _) = ctl(&format!("http://{addr}"), None, Command::Whoami, false).await;
    assert!(r.unwrap_err().message().contains("$CTL_KEY"));
    assert_eq!(hits.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn a_redirect_is_not_followed() {
    let elsewhere_hits = Arc::new(AtomicUsize::new(0));
    let counter = elsewhere_hits.clone();
    let elsewhere = serve(Router::new().route(
        "/v1/key/info",
        get(move || {
            let counter = counter.clone();
            async move {
                counter.fetch_add(1, Ordering::SeqCst);
                StatusCode::OK
            }
        }),
    ))
    .await;
    let target = format!("http://{elsewhere}/v1/key/info");
    let redirecting = serve(Router::new().route(
        "/v1/key/info",
        get(move || {
            let target = target.clone();
            async move { (StatusCode::FOUND, [(header::LOCATION, target)]).into_response() }
        }),
    ))
    .await;
    let (r, _) = ctl(
        &format!("http://{redirecting}"),
        Some("k"),
        Command::Whoami,
        false,
    )
    .await;
    let failure = r.unwrap_err();
    assert_eq!(failure.exit_code(), 2);
    assert!(
        failure.message().contains("redirect"),
        "{}",
        failure.message()
    );
    assert_eq!(
        elsewhere_hits.load(Ordering::SeqCst),
        0,
        "the key must not follow a redirect"
    );
}

#[tokio::test]
async fn plain_http_to_another_host_is_refused_before_any_request() {
    let (r, _) = ctl("http://gateway.example", Some("k"), Command::Whoami, false).await;
    assert!(r.unwrap_err().message().contains("plain http"));
}
