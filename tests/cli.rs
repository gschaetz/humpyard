#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // integration tests fail loudly on purpose
use std::process::Command;

fn run(args: &[&str], key: Option<&str>) -> std::process::Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_humpyard"));
    cmd.args(args).env_remove("CLI_TEST_KEY");
    if let Some(key) = key {
        cmd.env("CLI_TEST_KEY", key);
    }
    cmd.output().unwrap()
}

fn write_config(name: &str, body: &str) -> String {
    let path = std::env::temp_dir().join(format!("sc-{}-{name}.toml", std::process::id()));
    std::fs::write(&path, body).unwrap();
    path.to_string_lossy().into_owned()
}

const GOOD: &str = r#"listen = "127.0.0.1:0"
[providers.p]
base_url = "http://127.0.0.1:1"
api_key_env = "CLI_TEST_KEY"
[targets.m]
endpoints = [{ provider = "p", model = "m" }]
"#;

#[test]
fn check_config_accepts_valid_file() {
    let path = write_config("good", GOOD);
    let out = run(&["check-config", &path], Some("k"));
    assert!(out.status.success(), "{out:?}");
}

#[test]
fn check_config_fails_on_unknown_key() {
    let path = write_config("unknown", &format!("bogus = 1\n{GOOD}"));
    let out = run(&["check-config", &path], Some("k"));
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("bogus"));
}

#[test]
fn check_config_names_missing_env_var() {
    let path = write_config("nokey", GOOD);
    let out = run(&["check-config", &path], None);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("CLI_TEST_KEY"));
}

#[test]
fn serve_refuses_invalid_config() {
    let path = write_config("serve-bad", "nonsense");
    let out = run(&["serve", "--config", &path], Some("k"));
    assert!(!out.status.success());
}

#[test]
fn the_documented_budget_example_is_a_valid_config() {
    let out = Command::new(env!("CARGO_BIN_EXE_humpyard"))
        .args([
            "check-config",
            concat!(env!("CARGO_MANIFEST_DIR"), "/examples/budgets.toml"),
        ])
        .env("OPENROUTER_API_KEY", "k")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn the_quick_start_config_needs_only_the_one_key_the_readme_passes() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/examples/quickstart.toml");
    // The README's docker command passes only GROQ_API_KEY; the config must not need another.
    let out = Command::new(env!("CARGO_BIN_EXE_humpyard"))
        .args(["check-config", path])
        .env_remove("OPENROUTER_API_KEY")
        .env("GROQ_API_KEY", "k")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    // It runs in a container, where the default loopback address is unreachable from the host.
    let text = std::fs::read_to_string(path).unwrap();
    assert!(text.contains("listen = \"0.0.0.0:8080\""), "{text}");
    let readme =
        std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/README.md")).unwrap();
    assert!(
        readme.contains("-p 127.0.0.1:8080:8080") && readme.contains("quickstart.toml"),
        "the README quick start publishes on loopback and uses the quick-start config"
    );
}

#[test]
fn the_general_example_config_is_valid() {
    let out = Command::new(env!("CARGO_BIN_EXE_humpyard"))
        .args([
            "check-config",
            concat!(env!("CARGO_MANIFEST_DIR"), "/examples/config.toml"),
        ])
        .env("GROQ_API_KEY", "k")
        .env("OPENROUTER_API_KEY", "k")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn keygen_prints_a_key_and_the_matching_hash_and_stores_nothing() {
    use sha2::{Digest, Sha256};
    let dir = std::env::temp_dir().join(format!("sc-keygen-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_humpyard"))
        .args(["keygen", "alice"])
        .current_dir(&dir)
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    let text = String::from_utf8_lossy(&out.stdout);
    let key = text
        .split_whitespace()
        .find(|w| w.starts_with("sk-humpyard-"))
        .expect("key printed");
    assert!(text.contains("[keys.alice]"));
    let expected = format!("sha256:{}", hex::encode(Sha256::digest(key.as_bytes())));
    assert!(text.contains(&format!("sha256 = \"{expected}\"")), "{text}");
    assert_eq!(
        std::fs::read_dir(&dir).unwrap().count(),
        0,
        "keygen must not write files"
    );
}

/// Runs `humpyard keygen` with `args`, returning the exit status and the text it printed.
fn keygen(args: &[&str]) -> (bool, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_humpyard"))
        .arg("keygen")
        .args(args)
        .output()
        .unwrap();
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[test]
fn keygen_admin_prints_a_block_that_loads_and_authenticates() {
    let (ok, text, err) = keygen(&[
        "gary",
        "--admin",
        "--scopes",
        "read,overrides",
        "--expires",
        "2099-01-01T00:00:00Z",
    ]);
    assert!(ok, "{err}");
    let secret = text
        .split_whitespace()
        .find(|w| w.starts_with("sk-humpyard-"))
        .expect("secret printed");
    let block = &text[text
        .find("[admin.credentials.gary]")
        .expect("block printed")..];
    // The printed block, pasted into a config, loads and opens the admin API for exactly that secret.
    let config = humpyard::config::Config::from_toml(
        &format!(
            "listen = \"127.0.0.1:0\"\n[providers.p]\nbase_url = \"http://127.0.0.1:1\"\napi_key_env = \"PATH\"\n\
             [targets.t]\nendpoints = [{{ provider = \"p\", model = \"m\" }}]\n{block}"
        ),
        |name| std::env::var(name).ok(),
    )
    .unwrap();
    let principal = humpyard::admin::authenticate(&config.admin, Some(secret), 0).unwrap();
    assert_eq!(principal.id, "gary");
    assert!(principal.may(humpyard::config::AdminScope::Read));
    assert!(principal.may(humpyard::config::AdminScope::Overrides));
    assert!(!principal.may(humpyard::config::AdminScope::Credentials));
    assert!(principal.expires.is_some());
    assert!(humpyard::admin::authenticate(&config.admin, Some("not-it"), 0).is_err());
    // It is an admin credential, not a virtual key.
    assert!(!text.contains("[keys."), "{text}");
}

#[test]
fn keygen_admin_refuses_what_the_gateway_would_refuse() {
    // No scopes, an unknown scope, a bad id, an expiry without an offset, and admin-only flags without `--admin`.
    for args in [
        &["gary", "--admin"][..],
        &["gary", "--admin", "--scopes", "root"],
        &["bad id", "--admin", "--scopes", "read"],
        &[
            "gary",
            "--admin",
            "--scopes",
            "read",
            "--expires",
            "2099-01-01",
        ],
        &["gary", "--scopes", "read"],
        &["gary", "--expires", "2099-01-01T00:00:00Z"],
    ] {
        let (ok, text, _) = keygen(args);
        assert!(!ok, "{args:?} should be refused");
        assert!(
            !text.contains("sk-humpyard-"),
            "a refused keygen must not print a secret: {args:?}"
        );
    }
}
