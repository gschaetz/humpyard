//! `humpyard ctl overrides` (spec: `cli`): list, add and remove runtime overrides against a real in-process gateway,
//! and the argument rules of the real binary.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test scaffolding

mod common;

use common::serve;
use humpyard::config::Config;
use humpyard::ctl::{Command, Failure, OverridesCommand, Settings, connect, run};
use humpyard::server;

struct Fixture {
    url: String,
    ops: String,
    reader: String,
    app: String,
    _dir: tempfile::TempDir,
}

/// A gateway with a ledger, two targets on two providers, one route, and three keys: `ops` may manage overrides,
/// `reader` may read the configuration, `app` may do neither.
async fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let (ops, ops_hash) = humpyard::auth::generate_key().unwrap();
    let (reader, reader_hash) = humpyard::auth::generate_key().unwrap();
    let (app, app_hash) = humpyard::auth::generate_key().unwrap();
    let toml = format!(
        r#"listen = "127.0.0.1:0"
[ledger]
path = "{db}"
[keys.ops]
sha256 = "{ops_hash}"
manage_overrides = true
[keys.reader]
sha256 = "{reader_hash}"
read_config = true
[keys.app]
sha256 = "{app_hash}"
[providers.pa]
base_url = "http://127.0.0.1:1"
api_key_env = "K"
[providers.pb]
base_url = "http://127.0.0.1:1"
api_key_env = "K"
[targets.fast]
endpoints = [{{ provider = "pa", model = "m" }}]
[targets.deep]
endpoints = [{{ provider = "pb", model = "m" }}]
[routes.auto]
type = "passthrough"
targets = ["fast", "deep"]
"#,
        db = dir.path().join("l.db").display()
    );
    let config = Config::from_toml(&toml, |_| Some("k".into())).unwrap();
    let url = format!(
        "http://{}",
        serve(server::router(config).await.unwrap()).await
    );
    Fixture {
        url,
        ops,
        reader,
        app,
        _dir: dir,
    }
}

async fn ctl(
    f: &Fixture,
    key: &str,
    command: OverridesCommand,
    json: bool,
) -> (Result<(), Failure>, String) {
    let settings = Settings {
        url: Some(f.url.clone()),
        key_env: "CTL_KEY".into(),
        allow_insecure_http: false,
        json,
    };
    let key = key.to_string();
    let env = move |n: &str| (n == "CTL_KEY").then(|| key.clone());
    let gateway = match connect(&settings, &env) {
        Ok(g) => g,
        Err(failure) => return (Err(failure), String::new()),
    };
    let mut out: Vec<u8> = Vec::new();
    let result = run(&gateway, Command::Overrides(command), json, &mut out).await;
    (result, String::from_utf8(out).unwrap())
}

fn disable(target: &str, duration: Option<&str>) -> OverridesCommand {
    OverridesCommand::Add {
        disable_target: Some(target.into()),
        drain_provider: None,
        pin: None,
        reason: "slow".into(),
        duration: duration.map(str::to_string),
    }
}

fn list(history: bool) -> OverridesCommand {
    OverridesCommand::List { history }
}

#[tokio::test]
async fn taking_a_target_out_for_two_hours_prints_its_id_and_expiry() {
    let f = fixture().await;
    let (result, text) = ctl(&f, &f.ops, disable("deep", Some("2h")), false).await;
    result.unwrap();
    assert!(
        text.contains("created override 1: disable_target deep"),
        "{text}"
    );
    assert!(
        text.contains("expires in 2h") || text.contains("expires in 1h59m"),
        "about two hours ahead: {text}"
    );
}

#[tokio::test]
async fn list_shows_what_is_in_force_and_remove_ends_it() {
    let f = fixture().await;
    assert_eq!(
        ctl(&f, &f.ops, list(false), false).await.1,
        "no overrides in force\n"
    );
    ctl(&f, &f.ops, disable("deep", None), false)
        .await
        .0
        .unwrap();
    let pin = OverridesCommand::Add {
        disable_target: None,
        drain_provider: None,
        pin: Some("auto=fast".into()),
        reason: "keep it local".into(),
        duration: Some("30m".into()),
    };
    ctl(&f, &f.ops, pin, false).await.0.unwrap();

    let text = ctl(&f, &f.ops, list(false), false).await.1;
    assert!(
        text.contains("disable_target") && text.contains("deep"),
        "{text}"
    );
    assert!(
        text.contains("pin_route") && text.contains("auto"),
        "{text}"
    );
    assert!(
        text.contains("ops") && text.contains("keep it local"),
        "{text}"
    );

    let (result, text) = ctl(&f, &f.ops, OverridesCommand::Remove { id: 1 }, false).await;
    result.unwrap();
    assert_eq!(text, "removed override 1\n");
    let text = ctl(&f, &f.ops, list(false), false).await.1;
    assert!(!text.contains("deep"), "{text}");
    assert!(text.contains("pin_route"), "{text}");
}

#[tokio::test]
async fn the_history_includes_what_has_ended_and_how() {
    let f = fixture().await;
    ctl(&f, &f.ops, disable("deep", None), false)
        .await
        .0
        .unwrap();
    ctl(&f, &f.ops, disable("fast", None), false)
        .await
        .0
        .unwrap();
    ctl(&f, &f.ops, OverridesCommand::Remove { id: 1 }, false)
        .await
        .0
        .unwrap();

    let active = ctl(&f, &f.ops, list(false), false).await.1;
    assert!(
        !active.contains("deep"),
        "an ended override is not in force: {active}"
    );

    let history = ctl(&f, &f.ops, list(true), false).await.1;
    assert!(history.contains("STATUS"), "{history}");
    assert!(
        history.contains("revoked") && history.contains("deep"),
        "{history}"
    );
    assert!(
        history.contains("active") && history.contains("fast"),
        "{history}"
    );
}

#[tokio::test]
async fn json_prints_the_gateways_answer_unchanged() {
    let f = fixture().await;
    let (result, text) = ctl(&f, &f.ops, disable("deep", None), true).await;
    result.unwrap();
    let created: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(created["kind"], "disable_target");
    assert_eq!(created["created_by"], "ops");
    let (_, text) = ctl(&f, &f.ops, list(false), true).await;
    let listed: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(listed["overrides"][0]["subject"], "deep");
}

#[tokio::test]
async fn a_key_that_may_not_manage_is_refused_with_exit_1() {
    let f = fixture().await;
    for key in [&f.app, &f.reader] {
        let (result, _) = ctl(&f, key, disable("deep", None), false).await;
        let failure = result.unwrap_err();
        assert_eq!(failure.exit_code(), 1, "{}", failure.message());
        assert!(failure.message().contains("403"), "{}", failure.message());
    }
    // Reading the list needs only `read_config`.
    assert!(ctl(&f, &f.reader, list(false), false).await.0.is_ok());
    assert_eq!(
        ctl(&f, &f.app, list(false), false)
            .await
            .0
            .unwrap_err()
            .exit_code(),
        1
    );
}

#[tokio::test]
async fn a_bad_duration_or_pin_fails_before_anything_is_sent() {
    let f = fixture().await;
    let (result, _) = ctl(&f, &f.ops, disable("deep", Some("soon")), false).await;
    assert_eq!(result.unwrap_err().exit_code(), 2);
    let bad_pin = OverridesCommand::Add {
        disable_target: None,
        drain_provider: None,
        pin: Some("auto".into()),
        reason: "r".into(),
        duration: None,
    };
    assert_eq!(
        ctl(&f, &f.ops, bad_pin, false)
            .await
            .0
            .unwrap_err()
            .exit_code(),
        2
    );
    assert_eq!(
        ctl(&f, &f.ops, list(false), false).await.1,
        "no overrides in force\n",
        "nothing was created"
    );
}

#[tokio::test]
async fn removing_one_that_is_not_in_force_is_a_refusal() {
    let f = fixture().await;
    let failure = ctl(&f, &f.ops, OverridesCommand::Remove { id: 42 }, false)
        .await
        .0
        .unwrap_err();
    assert_eq!(failure.exit_code(), 1, "{}", failure.message());
}

/// The real binary, so the argument rules are the ones a user meets.
fn binary(args: &[&str]) -> std::process::Output {
    std::process::Command::new(env!("CARGO_BIN_EXE_humpyard"))
        .args(["ctl", "--url", "http://127.0.0.1:1", "overrides"])
        .args(args)
        .env_remove("HUMPYARD_API_KEY")
        .output()
        .unwrap()
}

#[test]
fn add_without_a_reason_fails_before_sending_anything() {
    let out = binary(&["add", "--disable-target", "deep"]);
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("--reason"), "{err}");
    assert!(
        !err.contains("cannot reach"),
        "it must fail on the arguments, not by trying the gateway: {err}"
    );
}

#[test]
fn add_needs_exactly_one_kind() {
    let none = binary(&["add", "--reason", "r"]);
    assert!(!none.status.success());
    let two = binary(&[
        "add",
        "--reason",
        "r",
        "--disable-target",
        "a",
        "--drain-provider",
        "b",
    ]);
    assert!(!two.status.success());
    let err = String::from_utf8_lossy(&two.stderr);
    assert!(err.contains("cannot be used with"), "{err}");
}
