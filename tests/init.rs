//! `humpyard init` (spec: `cli`): live checks, safe output, a proven setup, and the exposure rule.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test scaffolding

mod common;

use axum::Router;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use common::{chat_completion, serve, usage};
use humpyard::init::{InitOptions, run};

const SECRET: &str = "placeholder-value-that-must-not-appear";

/// A provider that lists `m` and answers chat for `m`; `broken` makes chat answer 404.
async fn provider(broken: bool) -> String {
    let app = Router::new()
        .route(
            "/models",
            get(|| async { axum::Json(serde_json::json!({"data": [{"id": "m"}]})) }),
        )
        .route(
            "/chat/completions",
            post(move || async move {
                if broken {
                    StatusCode::NOT_FOUND.into_response()
                } else {
                    axum::Json(chat_completion("m", "hello there", Some(usage(5, 3))))
                        .into_response()
                }
            }),
        );
    format!("http://{}", serve(app).await)
}

fn options(dir: &std::path::Path, base: &str, model: &str, listen: &str) -> InitOptions {
    InitOptions {
        providers: vec![],
        custom: vec![format!("mock,{base},MOCK_KEY,{model}")],
        out: dir.join("config.toml"),
        listen: listen.into(),
        no_proof: false,
    }
}

async fn init(options: InitOptions, key: Option<&str>) -> (Result<(), String>, String) {
    let env = |name: &str| {
        (name == "MOCK_KEY")
            .then(|| key.map(String::from))
            .flatten()
    };
    let mut out: Vec<u8> = Vec::new();
    let result = run(options, &env, None, &mut out, &mut std::io::sink()).await;
    (result, String::from_utf8(out).unwrap())
}

fn temp() -> std::path::PathBuf {
    let dir = common::scratch().join(format!(
        "humpyard-init-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[tokio::test]
async fn a_working_setup_is_written_proven_and_never_shows_the_key() {
    let dir = temp().join("works");
    std::fs::create_dir_all(&dir).unwrap();
    let base = provider(false).await;
    let (result, out) = init(options(&dir, &base, "m", "127.0.0.1:8080"), Some(SECRET)).await;
    result.unwrap();
    let file = std::fs::read_to_string(dir.join("config.toml")).unwrap();
    assert!(file.contains("api_key_env = \"MOCK_KEY\""), "{file}");
    assert!(file.contains("[targets.fast]"));
    assert!(out.contains("Reply: hello there"), "{out}");
    assert!(out.contains("humpyard serve"), "{out}");
    assert!(!out.contains(SECRET) && !file.contains(SECRET));
    assert!(!file.contains("[keys."), "a loopback setup has no keys");
}

#[tokio::test]
async fn an_existing_file_is_never_overwritten() {
    let dir = temp().join("exists");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("config.toml"), "keep me").unwrap();
    let base = provider(false).await;
    let (result, _) = init(options(&dir, &base, "m", "127.0.0.1:8080"), Some(SECRET)).await;
    assert!(result.unwrap_err().contains("already exists"));
    assert_eq!(
        std::fs::read_to_string(dir.join("config.toml")).unwrap(),
        "keep me"
    );
}

#[tokio::test]
async fn a_model_the_provider_no_longer_lists_is_not_written() {
    let dir = temp().join("retired");
    std::fs::create_dir_all(&dir).unwrap();
    let base = provider(false).await;
    let (result, out) = init(
        options(&dir, &base, "retired-model", "127.0.0.1:8080"),
        Some(SECRET),
    )
    .await;
    assert!(result.unwrap_err().contains("nothing was written"));
    assert!(
        out.contains("retired-model is not in its model list"),
        "{out}"
    );
    assert!(!dir.join("config.toml").exists());
}

#[tokio::test]
async fn a_model_that_does_not_answer_is_left_out() {
    let dir = temp().join("silent");
    std::fs::create_dir_all(&dir).unwrap();
    let base = provider(true).await;
    let (result, out) = init(options(&dir, &base, "m", "127.0.0.1:8080"), Some(SECRET)).await;
    assert!(result.is_err());
    assert!(out.contains("left out, answered 404"), "{out}");
    assert!(!dir.join("config.toml").exists());
}

#[tokio::test]
async fn a_missing_key_is_written_unchecked_with_the_variable_named() {
    let dir = temp().join("nokey");
    std::fs::create_dir_all(&dir).unwrap();
    let base = provider(false).await;
    let (result, out) = init(options(&dir, &base, "m", "127.0.0.1:8080"), None).await;
    result.unwrap();
    assert!(out.contains("$MOCK_KEY is not set"), "{out}");
    assert!(out.contains("No test request was sent"), "{out}");
    assert!(dir.join("config.toml").exists());
}

#[tokio::test]
async fn a_non_loopback_listener_gets_a_budgeted_key_shown_once() {
    let dir = temp().join("exposed");
    std::fs::create_dir_all(&dir).unwrap();
    let base = provider(false).await;
    let (result, out) = init(options(&dir, &base, "m", "0.0.0.0:8080"), Some(SECRET)).await;
    result.unwrap();
    let file = std::fs::read_to_string(dir.join("config.toml")).unwrap();
    assert!(
        file.contains("[keys.default]") && file.contains("daily_tokens"),
        "{file}"
    );
    assert!(out.contains("shown once"), "{out}");
    let shown = out
        .lines()
        .find(|l| l.starts_with("Key (shown once"))
        .unwrap();
    let key = shown.rsplit(": ").next().unwrap();
    assert!(
        key.starts_with("sk-humpyard-") && !file.contains(key),
        "the file holds the hash, not the key"
    );
    assert!(!out.contains(SECRET));
}

#[tokio::test]
async fn no_provider_and_no_terminal_explains_what_to_pass() {
    let dir = temp().join("none");
    std::fs::create_dir_all(&dir).unwrap();
    let mut o = options(&dir, "http://127.0.0.1:1", "m", "127.0.0.1:8080");
    o.custom.clear();
    let (result, _) = init(o, None).await;
    assert!(result.unwrap_err().contains("--provider"));
}

#[tokio::test]
async fn out_dash_prints_valid_config_to_stdout_and_progress_to_stderr() {
    let base = provider(false).await;
    let options = InitOptions {
        providers: vec![],
        custom: vec![format!("mock,{base},MOCK_KEY,m")],
        out: std::path::PathBuf::from("-"),
        listen: "127.0.0.1:8080".into(),
        no_proof: false,
    };
    let env = |name: &str| {
        (name == "MOCK_KEY")
            .then(|| Some(SECRET.to_string()))
            .flatten()
    };
    let mut progress: Vec<u8> = Vec::new();
    let mut config: Vec<u8> = Vec::new();
    let result = run(options, &env, None, &mut progress, &mut config).await;
    result.unwrap();

    let config_str = String::from_utf8(config).unwrap();
    let progress_str = String::from_utf8(progress).unwrap();

    // The config itself must be valid and must not contain progress or status text
    assert!(
        config_str.contains("api_key_env = \"MOCK_KEY\""),
        "{config_str}"
    );
    assert!(config_str.contains("[targets.fast]"), "{config_str}");
    assert!(
        !config_str.contains("Checking providers:"),
        "config must not contain progress text"
    );
    assert!(
        !config_str.contains("Reply: hello there"),
        "config must not contain test request reply"
    );
    assert!(!config_str.contains(SECRET));

    // Must parse as valid humpyard configuration (same check as `humpyard check-config`)
    let loaded = humpyard::config::Config::from_toml(&config_str, |name| {
        (name == "MOCK_KEY")
            .then(|| Some("test-key".into()))
            .flatten()
    });
    assert!(loaded.is_ok(), "config must pass config parser: {loaded:?}");

    // Progress text must have gone to the progress stream (stderr in the CLI)
    assert!(
        progress_str.contains("Checking providers:"),
        "{progress_str}"
    );
    assert!(
        progress_str.contains("Reply: hello there"),
        "{progress_str}"
    );
    assert!(!progress_str.contains(SECRET));
}

#[tokio::test]
async fn out_dash_with_exposed_listener_shows_key_only_in_progress_and_hash_in_config() {
    let base = provider(false).await;
    let options = InitOptions {
        providers: vec![],
        custom: vec![format!("mock,{base},MOCK_KEY,m")],
        out: std::path::PathBuf::from("-"),
        listen: "0.0.0.0:8080".into(),
        no_proof: false,
    };
    let env = |name: &str| {
        (name == "MOCK_KEY")
            .then(|| Some(SECRET.to_string()))
            .flatten()
    };
    let mut progress: Vec<u8> = Vec::new();
    let mut config: Vec<u8> = Vec::new();
    let result = run(options, &env, None, &mut progress, &mut config).await;
    result.unwrap();

    let config_str = String::from_utf8(config).unwrap();
    let progress_str = String::from_utf8(progress).unwrap();

    // 1. One-time key must appear only in progress output (stderr) and never in config (stdout)
    assert!(
        progress_str.contains("Key (shown once"),
        "progress must show the one-time key: {progress_str}"
    );
    let shown = progress_str
        .lines()
        .find(|l| l.starts_with("Key (shown once"))
        .expect("one-time key line must be present");
    let key = shown
        .rsplit(": ")
        .next()
        .expect("key value must follow colon");
    assert!(
        key.starts_with("sk-humpyard-"),
        "key must have expected prefix: {key}"
    );
    assert!(
        !config_str.contains(key),
        "plain key must NEVER appear in config output"
    );
    assert!(
        !config_str.contains("Key (shown once"),
        "one-time key banner must not appear in config"
    );

    // 2. Config must contain the key's sha256 hash and key section, not the key itself
    assert!(
        config_str.contains("[keys.default]") && config_str.contains("sha256 = \""),
        "config must contain the key's sha256 hash: {config_str}"
    );

    // 3. TOML config must parse as valid humpyard configuration
    let loaded = humpyard::config::Config::from_toml(&config_str, |name| {
        (name == "MOCK_KEY")
            .then(|| Some("test-key".into()))
            .flatten()
    });
    assert!(loaded.is_ok(), "config must pass config parser: {loaded:?}");

    // Secret provider key must not leak anywhere
    assert!(!config_str.contains(SECRET));
    assert!(!progress_str.contains(SECRET));
}
