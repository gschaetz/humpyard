//! `humpyard models check --retired`: every configured endpoint is compared with its provider's own model list.
//! What is reported, the exit codes, that one unusable provider never hides the others, that no `discover` or
//! `price_source` is needed, that nothing is written, and how the provider's list is fetched (the key goes only
//! to its own host, no redirects, bounded size and time).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test scaffolding

mod common;

use std::path::PathBuf;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::IntoResponse;
use axum::routing::get;
use common::serve;
use humpyard::discover::{ListFailure, fetch_provider_models_within};
use serde_json::{Value, json};

const KEY: &str = "provider-secret-key";

/// What a mock provider's `/models` does.
#[derive(Clone)]
enum Behavior {
    List(Vec<&'static str>),
    Status(u16),
    Body(String),
    Sleep(Duration),
    /// Nothing listens: the connection is refused.
    Closed,
}

async fn models(State(b): State<Arc<Mutex<Behavior>>>) -> axum::response::Response {
    let behavior = b.lock().unwrap().clone();
    match behavior {
        Behavior::List(ids) => {
            axum::Json(json!({"data": ids.iter().map(|id| json!({"id": id})).collect::<Vec<_>>()}))
                .into_response()
        }
        Behavior::Status(code) => StatusCode::from_u16(code).unwrap().into_response(),
        Behavior::Body(text) => text.into_response(),
        Behavior::Sleep(d) => {
            tokio::time::sleep(d).await;
            "late".into_response()
        }
        Behavior::Closed => unreachable!("a closed provider has no server"),
    }
}

async fn provider(behavior: Behavior) -> String {
    if matches!(behavior, Behavior::Closed) {
        // A port that was free a moment ago and has no listener now: the connection is refused.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener);
        return format!("http://{addr}");
    }
    let app = Router::new()
        .route("/models", get(models))
        .with_state(Arc::new(Mutex::new(behavior)));
    format!("http://{}", serve(app).await)
}

struct Rig {
    config: PathBuf,
    dir: tempfile::TempDir,
}

/// Two providers, `one` and `two`, with no `discover` and no `price_source`; each has a target on a model it
/// lists (`ok-*`) and one on a model it may have dropped (`gone-*`).
async fn rig(one: Behavior, two: Behavior) -> Rig {
    let (one_url, two_url) = (provider(one).await, provider(two).await);
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("humpyard.toml");
    std::fs::write(
        &config,
        format!(
            r#"listen = "127.0.0.1:0"
[providers.one]
base_url = "{one_url}"
api_key_env = "RETIRED_TEST_KEY"
[providers.two]
base_url = "{two_url}"
api_key_env = "RETIRED_TEST_KEY"
[targets.ok-one]
endpoints = [{{ provider = "one", model = "alpha" }}]
[targets.gone-one]
endpoints = [{{ provider = "one", model = "mimo-v2.5" }}]
[targets.ok-two]
endpoints = [{{ provider = "two", model = "beta" }}]
[targets.gone-two]
endpoints = [{{ provider = "two", model = "retired-two" }}]
[routes.r]
type = "passthrough"
targets = ["ok-one", "gone-one", "ok-two", "gone-two"]
"#
        ),
    )
    .unwrap();
    Rig { config, dir }
}

struct Out {
    code: i32,
    stdout: String,
    stderr: String,
}

async fn check(rig: &Rig, extra: &[&str]) -> Out {
    let mut args: Vec<String> = vec![
        "models".into(),
        "check".into(),
        "--retired".into(),
        "--config".into(),
        rig.config.to_string_lossy().into_owned(),
    ];
    args.extend(extra.iter().map(|a| (*a).to_string()));
    let out = tokio::task::spawn_blocking(move || {
        Command::new(env!("CARGO_BIN_EXE_humpyard"))
            .args(&args)
            .env("RETIRED_TEST_KEY", KEY)
            .output()
            .unwrap()
    })
    .await
    .unwrap();
    Out {
        code: out.status.code().unwrap(),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    }
}

fn listing(ids: &[&'static str]) -> Behavior {
    Behavior::List(ids.to_vec())
}

#[tokio::test]
async fn a_model_the_provider_stopped_listing_is_reported_and_exits_one() {
    let rig = rig(
        listing(&["alpha", "mimo-v2.6-flash"]),
        listing(&["beta", "retired-two"]),
    )
    .await;
    let out = check(&rig, &[]).await;
    assert_eq!(out.code, 1, "{}{}", out.stdout, out.stderr);
    assert!(
        out.stdout.contains("gone-one uses one/mimo-v2.5"),
        "{}",
        out.stdout
    );
    assert!(
        !out.stdout.contains("ok-one"),
        "a listed model is not reported:\n{}",
        out.stdout
    );
    assert!(!out.stdout.contains("gone-two"), "{}", out.stdout);
}

#[tokio::test]
async fn nothing_is_reported_when_every_model_is_listed() {
    let rig = rig(
        listing(&["alpha", "mimo-v2.5"]),
        listing(&["beta", "retired-two"]),
    )
    .await;
    let out = check(&rig, &[]).await;
    assert_eq!(out.code, 0, "{}{}", out.stdout, out.stderr);
    assert!(out.stdout.contains("nothing to review"), "{}", out.stdout);
}

#[tokio::test]
async fn one_unusable_provider_does_not_hide_the_others() {
    let rig = rig(listing(&["alpha"]), Behavior::Status(500)).await;
    let out = check(&rig, &[]).await;
    assert_eq!(out.code, 2, "{}{}", out.stdout, out.stderr);
    assert!(
        out.stdout.contains("gone-one uses one/mimo-v2.5"),
        "provider one is still checked:\n{}",
        out.stdout
    );
    assert!(out.stdout.contains("unusable:   two"), "{}", out.stdout);
    assert!(
        !out.stdout.contains("gone-two"),
        "an unusable provider's endpoints are not guessed at:\n{}",
        out.stdout
    );
}

#[tokio::test]
async fn a_body_that_is_not_a_model_list_is_unusable() {
    let rig = rig(
        listing(&["alpha", "mimo-v2.5"]),
        Behavior::Body("<html>nope</html>".into()),
    )
    .await;
    let out = check(&rig, &[]).await;
    assert_eq!(out.code, 2, "{}{}", out.stdout, out.stderr);
    assert!(out.stdout.contains("unusable:   two"), "{}", out.stdout);
}

#[tokio::test]
async fn no_discover_and_no_price_source_are_needed_and_no_file_is_written() {
    let rig = rig(listing(&["alpha"]), listing(&["beta"])).await;
    let before = std::fs::read_to_string(&rig.config).unwrap();
    let files_before: Vec<_> = std::fs::read_dir(rig.dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    let out = check(&rig, &[]).await;
    assert_eq!(out.code, 1, "{}{}", out.stdout, out.stderr);
    assert_eq!(std::fs::read_to_string(&rig.config).unwrap(), before);
    let mut files_after: Vec<_> = std::fs::read_dir(rig.dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    let mut files_before = files_before;
    files_before.sort();
    files_after.sort();
    assert_eq!(files_before, files_after, "the check writes nothing");
}

#[tokio::test]
async fn provider_filter_and_json_output() {
    let rig = rig(listing(&["alpha"]), listing(&["beta"])).await;
    let out = check(&rig, &["--provider", "one", "--json"]).await;
    assert_eq!(out.code, 1, "{}{}", out.stdout, out.stderr);
    let report: Value = serde_json::from_str(out.stdout.trim()).unwrap();
    assert_eq!(
        report["retired"],
        json!([{"target": "gone-one", "provider": "one", "model": "mimo-v2.5"}])
    );
    assert_eq!(
        report["checked"], 2,
        "only provider one's two endpoints were compared"
    );
    assert_eq!(report["unusable"], json!([]));

    let missing = check(&rig, &["--provider", "nope"]).await;
    assert_eq!(missing.code, 2);
    assert!(
        missing.stderr.contains("no provider named `nope`"),
        "{}",
        missing.stderr
    );
}

#[tokio::test]
async fn it_cannot_be_combined_with_the_paid_report() {
    let rig = rig(listing(&["alpha"]), listing(&["beta"])).await;
    let out = check(&rig, &["--paid"]).await;
    assert_eq!(out.code, 2, "{}{}", out.stdout, out.stderr);
}

#[tokio::test]
async fn a_provider_without_a_model_list_is_reported_and_does_not_change_the_exit_code() {
    let listed = rig(listing(&["alpha", "mimo-v2.5"]), Behavior::Status(404)).await;
    let out = check(&listed, &[]).await;
    assert_eq!(out.code, 0, "{}{}", out.stdout, out.stderr);
    assert!(
        out.stdout.contains("no list:    two has no model list"),
        "{}",
        out.stdout
    );
    assert!(out.stdout.contains("nothing to review"), "{}", out.stdout);
    assert!(
        !out.stdout.contains("gone-two"),
        "its endpoints are not compared:\n{}",
        out.stdout
    );

    // A retired endpoint on a provider that does have a list is still reported.
    let elsewhere = rig(listing(&["alpha"]), Behavior::Status(404)).await;
    let out = check(&elsewhere, &[]).await;
    assert_eq!(out.code, 1, "{}{}", out.stdout, out.stderr);
    assert!(
        out.stdout.contains("gone-one uses one/mimo-v2.5"),
        "{}",
        out.stdout
    );
}

#[tokio::test]
async fn a_provider_that_cannot_be_reached_is_reported_and_does_not_change_the_exit_code() {
    for dead in [Behavior::Closed, Behavior::Status(503)] {
        let listed = rig(listing(&["alpha", "mimo-v2.5"]), dead.clone()).await;
        let out = check(&listed, &[]).await;
        assert_eq!(out.code, 0, "{}{}", out.stdout, out.stderr);
        assert!(
            out.stdout.contains("unreachable: two could not be asked"),
            "{}",
            out.stdout
        );
        assert!(!out.stdout.contains("unusable:"), "{}", out.stdout);

        // It does not hide a retired endpoint on a provider that did answer.
        let elsewhere = rig(listing(&["alpha"]), dead).await;
        let out = check(&elsewhere, &[]).await;
        assert_eq!(out.code, 1, "{}{}", out.stdout, out.stderr);
        assert!(
            out.stdout.contains("gone-one uses one/mimo-v2.5"),
            "{}",
            out.stdout
        );
    }
}

#[tokio::test]
async fn a_rejected_key_is_still_unusable_and_exits_two() {
    let rig = rig(listing(&["alpha", "mimo-v2.5"]), Behavior::Status(401)).await;
    let out = check(&rig, &[]).await;
    assert_eq!(out.code, 2, "{}{}", out.stdout, out.stderr);
    assert!(out.stdout.contains("unusable:   two"), "{}", out.stdout);
}

#[tokio::test]
async fn the_json_report_carries_no_list_and_unreachable() {
    let rig = rig(Behavior::Status(404), Behavior::Closed).await;
    let out = check(&rig, &["--json"]).await;
    assert_eq!(out.code, 0, "{}{}", out.stdout, out.stderr);
    let report: Value = serde_json::from_str(out.stdout.trim()).unwrap();
    assert_eq!(report["no_list"][0]["provider"], "one");
    assert_eq!(report["unreachable"][0]["provider"], "two");
    assert_eq!(report["retired"], json!([]));
    assert_eq!(report["unusable"], json!([]));
    assert_eq!(report["checked"], 0);
}

// ---- how the list is fetched (task 2.1) ----

#[tokio::test]
async fn a_valid_list_is_read() {
    let url = provider(listing(&["a", "b"])).await;
    let got = fetch_provider_models_within(&url, KEY, Duration::from_secs(5))
        .await
        .unwrap();
    assert_eq!(got.into_iter().collect::<Vec<_>>(), ["a", "b"]);
}

async fn failure(behavior: Behavior) -> ListFailure {
    let url = provider(behavior).await;
    fetch_provider_models_within(&url, KEY, Duration::from_secs(5))
        .await
        .unwrap_err()
}

#[tokio::test]
async fn a_failed_list_is_told_apart_as_no_list_unreachable_or_unusable() {
    for status in [404, 405, 501] {
        assert!(
            matches!(
                failure(Behavior::Status(status)).await,
                ListFailure::NoList(_)
            ),
            "{status} means the provider has no model list"
        );
    }
    for status in [502, 503, 504] {
        assert!(
            matches!(
                failure(Behavior::Status(status)).await,
                ListFailure::Unreachable(_)
            ),
            "{status} means the upstream could not be reached"
        );
    }
    for status in [400, 401, 403, 500] {
        assert!(
            matches!(
                failure(Behavior::Status(status)).await,
                ListFailure::Unusable(_)
            ),
            "{status} is an answer the check cannot use"
        );
    }
    assert!(matches!(
        failure(Behavior::Body("{\"nope\": 1}".into())).await,
        ListFailure::Unusable(_)
    ));
    assert!(matches!(
        failure(Behavior::Body("<html></html>".into())).await,
        ListFailure::Unusable(_)
    ));
    assert!(
        matches!(failure(Behavior::Closed).await, ListFailure::Unreachable(_)),
        "a refused connection is unreachable"
    );
    let slow = provider(Behavior::Sleep(Duration::from_secs(3))).await;
    let started = std::time::Instant::now();
    let err = fetch_provider_models_within(&slow, KEY, Duration::from_millis(300))
        .await
        .unwrap_err();
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "the time limit applies: {err}"
    );
    assert!(
        matches!(err, ListFailure::Unreachable(_)),
        "a timeout is unreachable: {err}"
    );
}

#[tokio::test]
async fn an_oversized_body_is_refused() {
    let big = format!(
        "{{\"data\": [{{\"id\": \"{}\"}}]}}",
        "x".repeat(17 * 1024 * 1024)
    );
    let url = provider(Behavior::Body(big)).await;
    let err = fetch_provider_models_within(&url, KEY, Duration::from_secs(20))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("larger than"), "{err}");
}

#[tokio::test]
async fn the_key_goes_only_to_the_providers_own_host_and_redirects_are_not_followed() {
    let seen: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let other = {
        let seen = seen.clone();
        let app = Router::new().route(
            "/models",
            get(move |headers: HeaderMap| {
                let seen = seen.clone();
                async move {
                    let auth = headers
                        .get(header::AUTHORIZATION)
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or("")
                        .to_string();
                    seen.lock().unwrap().push(auth);
                    axum::Json(json!({"data": [{"id": "stolen"}]}))
                }
            }),
        );
        format!("http://{}", serve(app).await)
    };
    let redirecting = {
        let target = format!("{other}/models");
        let app = Router::new().route(
            "/models",
            get(move || {
                let target = target.clone();
                async move { (StatusCode::FOUND, [(header::LOCATION, target)]).into_response() }
            }),
        );
        format!("http://{}", serve(app).await)
    };
    let result = fetch_provider_models_within(&redirecting, KEY, Duration::from_secs(5)).await;
    assert!(result.is_err(), "a redirect is not followed");
    assert!(
        seen.lock().unwrap().is_empty(),
        "the key never reached the other host: {:?}",
        seen.lock().unwrap()
    );
}
