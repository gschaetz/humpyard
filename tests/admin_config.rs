//! The `[admin]` section as the rest of the gateway sees it (spec: `gateway-config`, change `add-admin-api`):
//! the effective configuration shows how many credentials exist and never who or what, `check-config` shows
//! ids, scopes and expiry and never a hash, and the fingerprint follows every change to a credential.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test scaffolding

mod common;

use std::path::PathBuf;
use std::process::Command;

use common::serve;
use humpyard::config::Config;
use humpyard::server;
use serde_json::Value;

const BASE: &str = r#"listen = "127.0.0.1:0"
[providers.p]
base_url = "http://127.0.0.1:1"
api_key_env = "PATH"
[targets.t]
endpoints = [{ provider = "p", model = "m" }]
[routes.r]
type = "passthrough"
targets = ["t"]
"#;

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "humpyard-admincfg-{}-{name}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn admin_block(hash: &str, scopes: &str) -> String {
    format!(
        "[admin.credentials.gary]\nsha256 = \"{hash}\"\nscopes = {scopes}\nexpires = 2027-01-01T00:00:00Z\n"
    )
}

fn check_config(path: &std::path::Path) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_humpyard"))
        .args(["check-config", path.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}

fn fingerprint_of(output: &str) -> String {
    output
        .lines()
        .find_map(|line| line.strip_prefix("fingerprint: "))
        .expect("a fingerprint line")
        .to_string()
}

#[tokio::test]
async fn the_effective_configuration_shows_a_count_and_no_credential_detail() {
    let (admin_secret, admin_hash) = humpyard::auth::generate_key().unwrap();
    let (reader, reader_hash) = humpyard::auth::generate_key().unwrap();
    let text = format!(
        "{BASE}[keys.reader]\nsha256 = \"{reader_hash}\"\nread_config = true\n{}",
        admin_block(&admin_hash, "[\"read\", \"overrides\"]")
    );
    let config = Config::from_toml(&text, |_| Some("k".into())).unwrap();
    let url = format!(
        "http://{}",
        serve(server::router(config).await.unwrap()).await
    );
    let body = reqwest::Client::new()
        .get(format!("{url}/v1/config"))
        .bearer_auth(&reader)
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    let view: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(view["admin"]["enabled"], true);
    assert_eq!(view["admin"]["credentials"], 1);
    // A client key that may read the configuration learns neither who administers it nor how.
    let hex = admin_hash.trim_start_matches("sha256:");
    for secret in [hex, admin_secret.as_str(), "gary", "overrides"] {
        assert!(!body.contains(secret), "the view contains `{secret}`");
    }
}

#[test]
fn check_config_shows_ids_scopes_and_expiry_but_never_a_hash() {
    let (_, hash) = humpyard::auth::generate_key().unwrap();
    let dir = temp("check");
    let path = dir.join("config.toml");
    std::fs::write(
        &path,
        format!(
            "{BASE}{}",
            admin_block(&hash, "[\"read\", \"credentials\"]")
        ),
    )
    .unwrap();
    let out = check_config(&path);
    assert!(
        out.contains(
            "admin credential gary: scopes read, credentials; expires 2027-01-01T00:00:00Z"
        ),
        "{out}"
    );
    assert!(!out.contains(hash.trim_start_matches("sha256:")), "{out}");
}

#[test]
fn the_fingerprint_changes_whenever_a_credential_does() {
    let (_, first) = humpyard::auth::generate_key().unwrap();
    let (_, second) = humpyard::auth::generate_key().unwrap();
    let dir = temp("fingerprint");
    let path = dir.join("config.toml");
    let fingerprint = |hash: &str, scopes: &str| {
        std::fs::write(&path, format!("{BASE}{}", admin_block(hash, scopes))).unwrap();
        fingerprint_of(&check_config(&path))
    };
    let original = fingerprint(&first, "[\"read\"]");
    assert_eq!(original, fingerprint(&first, "[\"read\"]"), "stable");
    assert_ne!(original, fingerprint(&first, "[\"read\", \"overrides\"]"));
    assert_ne!(original, fingerprint(&second, "[\"read\"]"));
    std::fs::write(&path, BASE).unwrap();
    assert_ne!(original, fingerprint_of(&check_config(&path)));
}
