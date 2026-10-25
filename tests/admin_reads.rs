//! The admin API's read side (spec: `admin-api`, change `add-admin-api`): with the `read` scope an admin sees
//! every key's decisions and statistics (or one key's, by `key`), the same configuration, health and quota the
//! client endpoints show, and never a hash or a secret; a credential without `read` sees none of it.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test scaffolding

mod common;

use axum::Router;
use axum::response::IntoResponse;
use axum::routing::post;
use common::{chat_completion, chat_request, serve, usage};
use humpyard::config::Config;
use humpyard::server;
use serde_json::{Value, json};

struct Rig {
    url: String,
    alice: String,
    bob: String,
    ops: String,
    gary: String,
    sam: String,
    hashes: Vec<String>,
    secrets: Vec<String>,
    _dir: tempfile::TempDir,
}

async fn completions() -> axum::response::Response {
    axum::Json(chat_completion("m", "ok", Some(usage(10, 5)))).into_response()
}

async fn rig() -> Rig {
    let app = Router::new().route("/chat/completions", post(completions));
    let upstream = format!("http://{}", serve(app).await);
    let gen_key = || humpyard::auth::generate_key().unwrap();
    let ((alice, alice_h), (bob, bob_h), (ops, ops_h), (gary, gary_h), (sam, sam_h)) =
        (gen_key(), gen_key(), gen_key(), gen_key(), gen_key());
    let dir = tempfile::tempdir().unwrap();
    let ledger = dir.path().join("ledger.db");
    let toml = format!(
        r#"listen = "127.0.0.1:0"
[ledger]
path = "{ledger}"
[keys.alice]
sha256 = "{alice_h}"
[keys.bob]
sha256 = "{bob_h}"
[keys.ops]
sha256 = "{ops_h}"
read_config = true
[admin.credentials.gary]
sha256 = "{gary_h}"
scopes = ["read"]
expires = 2999-01-01T00:00:00Z
[admin.credentials.sam]
sha256 = "{sam_h}"
scopes = ["overrides"]
[providers.p]
base_url = "{upstream}"
api_key_env = "PATH"
max_retries = 0
[targets.t]
endpoints = [{{ provider = "p", model = "m" }}]
[routes.r]
type = "passthrough"
targets = ["t"]
"#,
        ledger = ledger.display()
    );
    let config = Config::from_toml(&toml, |n| std::env::var(n).ok()).unwrap();
    let url = format!(
        "http://{}",
        serve(server::router(config).await.unwrap()).await
    );
    let strip = |h: &str| h.trim_start_matches("sha256:").to_string();
    Rig {
        url,
        hashes: [&alice_h, &bob_h, &ops_h, &gary_h, &sam_h]
            .map(|h| strip(h))
            .to_vec(),
        secrets: vec![
            alice.clone(),
            bob.clone(),
            ops.clone(),
            gary.clone(),
            sam.clone(),
        ],
        alice,
        bob,
        ops,
        gary,
        sam,
        _dir: dir,
    }
}

impl Rig {
    async fn ask(&self, key: &str) {
        let response = reqwest::Client::new()
            .post(format!("{}/v1/chat/completions", self.url))
            .bearer_auth(key)
            .json(&chat_request("r", false))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
    }

    async fn get(&self, path: &str, key: &str) -> (u16, String) {
        let response = reqwest::Client::new()
            .get(format!("{}{path}", self.url))
            .bearer_auth(key)
            .send()
            .await
            .unwrap();
        (response.status().as_u16(), response.text().await.unwrap())
    }

    async fn json(&self, path: &str, key: &str) -> Value {
        let (status, body) = self.get(path, key).await;
        assert_eq!(status, 200, "{path}: {body}");
        serde_json::from_str(&body).unwrap()
    }

    /// Alice makes two requests and Bob one, and the decisions have been written.
    async fn traffic(&self) {
        self.ask(&self.alice).await;
        self.ask(&self.alice).await;
        self.ask(&self.bob).await;
        for _ in 0..100 {
            if self.json("/admin/v1/decisions", &self.gary).await["data"]
                .as_array()
                .unwrap()
                .len()
                == 3
            {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(30)).await;
        }
        panic!("the decisions were not written");
    }
}

fn keys_of(decisions: &Value) -> Vec<String> {
    let mut keys: Vec<String> = decisions["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["key"].as_str().unwrap().to_string())
        .collect();
    keys.sort();
    keys
}

#[tokio::test]
async fn an_admin_sees_every_keys_decisions_and_can_narrow_to_one() {
    let rig = rig().await;
    rig.traffic().await;
    let all = rig.json("/admin/v1/decisions", &rig.gary).await;
    assert_eq!(keys_of(&all), ["alice", "alice", "bob"]);
    let alice = rig.json("/admin/v1/decisions?key=alice", &rig.gary).await;
    assert_eq!(keys_of(&alice), ["alice", "alice"]);
    let bob = rig.json("/admin/v1/decisions?key=bob", &rig.gary).await;
    assert_eq!(keys_of(&bob), ["bob"]);
    let nobody = rig.json("/admin/v1/decisions?key=nobody", &rig.gary).await;
    assert_eq!(nobody["data"].as_array().unwrap().len(), 0);
    // The ordinary filters still apply on top.
    let limited = rig.json("/admin/v1/decisions?limit=1", &rig.gary).await;
    assert_eq!(limited["data"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn the_client_endpoint_still_shows_a_key_only_its_own() {
    let rig = rig().await;
    rig.traffic().await;
    let own = rig.json("/v1/decisions", &rig.alice).await;
    assert_eq!(keys_of(&own), ["alice", "alice"]);
    // A key cannot widen itself with the admin filter, or by naming another key.
    let (status, body) = rig.get("/v1/decisions?key=bob", &rig.alice).await;
    assert_eq!(status, 400, "{body}");
}

#[tokio::test]
async fn statistics_cover_every_key_or_the_one_named() {
    let rig = rig().await;
    rig.traffic().await;
    let total = |stats: &Value| -> u64 {
        stats["data"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| row["requests"].as_u64().unwrap())
            .sum()
    };
    let all = rig.json("/admin/v1/stats?group_by=route", &rig.gary).await;
    assert_eq!(total(&all), 3, "{all}");
    let alice = rig
        .json("/admin/v1/stats?group_by=route&key=alice", &rig.gary)
        .await;
    assert_eq!(total(&alice), 2, "{alice}");
    let mine = rig.json("/v1/stats?group_by=route", &rig.bob).await;
    assert_eq!(total(&mine), 1);
}

#[tokio::test]
async fn health_quota_and_config_are_the_clients_views_with_the_admin_section_in_full() {
    let rig = rig().await;
    let client_health = rig.json("/v1/health", &rig.alice).await;
    let admin_health = rig.json("/admin/v1/health", &rig.gary).await;
    assert_eq!(admin_health["config"], client_health["config"]);
    assert_eq!(admin_health["targets"][0]["target"], "t");
    assert_eq!(
        rig.json("/admin/v1/quota", &rig.gary).await,
        rig.json("/v1/quota", &rig.alice).await
    );
    let admin_config = rig.json("/admin/v1/config", &rig.gary).await;
    let client_config = rig.json("/v1/config", &rig.ops).await;
    for section in ["targets", "routes", "providers", "config"] {
        assert_eq!(admin_config[section], client_config[section], "{section}");
    }
    // The client endpoint counts the credentials; the admin one names them.
    assert_eq!(client_config["admin"]["credentials"], 2);
    assert_eq!(
        admin_config["admin"]["credentials"]["gary"],
        json!({"id": "gary", "kind": "credential", "scopes": ["read"], "expires": "2999-01-01T00:00:00Z"})
    );
    assert_eq!(
        admin_config["admin"]["credentials"]["sam"]["scopes"],
        json!(["overrides"])
    );
}

#[tokio::test]
async fn credentials_are_listed_without_hashes() {
    let rig = rig().await;
    let list = rig.json("/admin/v1/credentials", &rig.gary).await;
    assert_eq!(list["object"], "list");
    let ids: Vec<&str> = list["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["gary", "sam"]);
    assert_eq!(list["data"][0]["expires"], "2999-01-01T00:00:00Z");
    assert_eq!(list["data"][1]["expires"], Value::Null);
    // Exactly these fields, and nothing that could be a secret or a hash.
    let mut fields: Vec<&str> = list["data"][0]
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    fields.sort_unstable();
    assert_eq!(fields, ["expires", "id", "kind", "revoked", "scopes"]);
    assert_eq!(list["data"][0]["revoked"], false);
}

#[tokio::test]
async fn no_admin_read_contains_a_secret_or_a_hash() {
    let rig = rig().await;
    rig.traffic().await;
    for path in [
        "/admin/v1/config",
        "/admin/v1/health",
        "/admin/v1/quota",
        "/admin/v1/decisions",
        "/admin/v1/stats",
        "/admin/v1/credentials",
        "/admin/v1/whoami",
    ] {
        let (status, body) = rig.get(path, &rig.gary).await;
        assert_eq!(status, 200, "{path}");
        for hidden in rig.hashes.iter().chain(&rig.secrets) {
            assert!(
                !body.contains(hidden.as_str()),
                "{path} contains a secret or hash"
            );
        }
    }
}

#[tokio::test]
async fn a_credential_without_read_sees_none_of_it_and_a_client_key_is_refused() {
    let rig = rig().await;
    for path in [
        "/admin/v1/config",
        "/admin/v1/health",
        "/admin/v1/quota",
        "/admin/v1/decisions",
        "/admin/v1/stats",
        "/admin/v1/credentials",
    ] {
        let (status, body) = rig.get(path, &rig.sam).await;
        assert_eq!(status, 403, "{path}");
        let body: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(body["error"]["code"], "admin_scope_required");
        assert_eq!(body["error"]["scope"], "read");
        let (status, _) = rig.get(path, &rig.ops).await;
        assert_eq!(status, 401, "{path}: the strongest client key");
    }
    // Whoever it is, `whoami` needs no scope.
    assert_eq!(rig.json("/admin/v1/whoami", &rig.sam).await["id"], "sam");
}
