//! Revoking a client key without a deploy (spec: `admin-api`, `client-auth`; change `add-actors`): an admin with the
//! `credentials` scope retires a leaked key at once, the key is refused exactly like one the gateway never knew,
//! and the revocation is an audited, expiring record that is not an override and cannot be undone by anything but the
//! admin API.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test scaffolding

mod common;

use std::time::Duration;

use axum::Router;
use axum::response::IntoResponse;
use axum::routing::post;
use common::{chat_completion, chat_request, serve, usage};
use humpyard::config::Config;
use humpyard::server;
use serde_json::{Value, json};

struct Rig {
    toml: String,
    url: String,
    ledger: std::path::PathBuf,
    leaked: String,
    leaked_hash: String,
    steady: String,
    ops: String,
    gary: String,
    sam: String,
    robin: String,
    metrics: String,
    _dir: tempfile::TempDir,
}

async fn completions() -> axum::response::Response {
    axum::Json(chat_completion("m", "ok", Some(usage(10, 5)))).into_response()
}

/// `with_ledger` false gives a gateway that cannot store a revocation.
async fn rig(with_ledger: bool) -> Rig {
    let upstream = format!(
        "http://{}",
        serve(Router::new().route("/chat/completions", post(completions))).await
    );
    let gen_key = || humpyard::auth::generate_key().unwrap();
    let (
        (leaked, leaked_h),
        (steady, steady_h),
        (ops, ops_h),
        (gary, gary_h),
        (sam, sam_h),
        (robin, robin_h),
        (metrics, metrics_h),
    ) = (
        gen_key(),
        gen_key(),
        gen_key(),
        gen_key(),
        gen_key(),
        gen_key(),
        gen_key(),
    );
    let dir = tempfile::tempdir().unwrap();
    let ledger = dir.path().join("ledger.db");
    let ledger_section = if with_ledger {
        format!("[ledger]\npath = \"{}\"\n", ledger.display())
    } else {
        String::new()
    };
    let toml = format!(
        r#"listen = "127.0.0.1:0"
{ledger_section}
[metrics]
enabled = true
bearer_sha256 = "{metrics_h}"
[keys.leaked]
sha256 = "{leaked_h}"
[keys.steady]
sha256 = "{steady_h}"
[keys.ops]
sha256 = "{ops_h}"
read_config = true
manage_overrides = true
[admin.credentials.gary]
sha256 = "{gary_h}"
scopes = ["read", "overrides", "credentials"]
[admin.credentials.sam]
sha256 = "{sam_h}"
scopes = ["read"]
[admin.credentials.robin]
sha256 = "{robin_h}"
scopes = ["read", "overrides"]
[providers.p]
base_url = "{upstream}"
api_key_env = "PATH"
max_retries = 0
[targets.a]
endpoints = [{{ provider = "p", model = "m" }}]
[routes.r]
type = "passthrough"
targets = ["a"]
"#
    );
    let config = Config::from_toml(&toml, |n| std::env::var(n).ok()).unwrap();
    let url = format!(
        "http://{}",
        serve(server::router(config).await.unwrap()).await
    );
    Rig {
        toml,
        url,
        ledger,
        leaked,
        leaked_hash: leaked_h,
        steady,
        ops,
        gary,
        sam,
        robin,
        metrics,
        _dir: dir,
    }
}

impl Rig {
    async fn call(
        &self,
        method: reqwest::Method,
        path: &str,
        key: &str,
        body: Option<Value>,
    ) -> (u16, Value) {
        let mut request = reqwest::Client::new()
            .request(method, format!("{}{path}", self.url))
            .bearer_auth(key);
        if let Some(body) = body {
            request = request.json(&body);
        }
        let response = request.send().await.unwrap();
        let status = response.status().as_u16();
        (status, response.json().await.unwrap_or(Value::Null))
    }

    async fn get(&self, path: &str, key: &str) -> (u16, Value) {
        self.call(reqwest::Method::GET, path, key, None).await
    }

    async fn post(&self, path: &str, key: &str, body: Value) -> (u16, Value) {
        self.call(reqwest::Method::POST, path, key, Some(body))
            .await
    }

    async fn delete(&self, path: &str, key: &str) -> (u16, Value) {
        self.call(reqwest::Method::DELETE, path, key, None).await
    }

    async fn revoke(&self, by: &str, key: &str, secs: u64) -> (u16, Value) {
        self.post(
            &format!("/admin/v1/keys/{key}/revoke"),
            by,
            json!({"reason": "leaked in a screenshot", "expires_in_secs": secs}),
        )
        .await
    }

    /// The status and raw body of an inference request with `key`.
    async fn infer(&self, key: &str) -> (u16, String) {
        let response = reqwest::Client::new()
            .post(format!("{}/v1/chat/completions", self.url))
            .bearer_auth(key)
            .json(&chat_request("r", false))
            .send()
            .await
            .unwrap();
        (response.status().as_u16(), response.text().await.unwrap())
    }

    async fn metrics(&self) -> String {
        reqwest::Client::new()
            .get(format!("{}/metrics", self.url))
            .bearer_auth(&self.metrics)
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap()
    }

    /// Every row of the overrides table: `id`, `kind`, `subject`, `created_by` and `revoked_ms`.
    async fn rows(&self) -> Vec<(i64, String, String, String, Option<i64>)> {
        let pool = sqlx::SqlitePool::connect(&format!("sqlite://{}", self.ledger.display()))
            .await
            .unwrap();
        sqlx::query_as(
            "SELECT id, kind, subject, created_by, revoked_ms FROM overrides ORDER BY id",
        )
        .fetch_all(&pool)
        .await
        .unwrap()
    }
}

#[tokio::test]
async fn revoking_a_leaked_key_refuses_its_next_request_and_leaves_the_others_alone() {
    let rig = rig(true).await;
    assert_eq!(rig.infer(&rig.leaked).await.0, 200);
    let (status, revoked) = rig.revoke(&rig.gary, "leaked", 600).await;
    assert_eq!(status, 201, "{revoked}");
    assert_eq!(revoked["key"], "leaked");
    assert_eq!(revoked["revoked"], true);
    assert_eq!(revoked["created_by"], "admin:gary");
    assert_eq!(rig.infer(&rig.leaked).await.0, 401);
    assert_eq!(rig.infer(&rig.steady).await.0, 200);
    // The audit row names the revoker and the key, and only the key's id.
    let rows = rig.rows().await;
    assert_eq!(rows.len(), 1);
    assert_eq!(
        (rows[0].1.as_str(), rows[0].2.as_str(), rows[0].3.as_str()),
        ("revoke_client_key", "leaked", "admin:gary")
    );
}

#[tokio::test]
async fn a_revoked_key_is_refused_byte_for_byte_like_a_key_the_gateway_never_knew() {
    let rig = rig(true).await;
    rig.revoke(&rig.gary, "leaked", 600).await;
    let (_, never_known) = humpyard::auth::generate_key().unwrap();
    let revoked = rig.infer(&rig.leaked).await;
    let unknown = rig.infer(&never_known).await;
    assert_eq!(revoked, unknown);
    assert_eq!(revoked.0, 401);
    // The same on a client read, and the answer never says "revoked".
    let (a, ab) = rig.get("/v1/key/info", &rig.leaked).await;
    let (b, bb) = rig.get("/v1/key/info", &never_known).await;
    assert_eq!((a, &ab), (b, &bb));
    assert!(
        !revoked.1.to_lowercase().contains("revoked"),
        "{}",
        revoked.1
    );
}

#[tokio::test]
async fn a_revocation_ends_when_removed_or_when_it_expires() {
    let rig = rig(true).await;
    rig.revoke(&rig.gary, "leaked", 600).await;
    assert_eq!(rig.infer(&rig.leaked).await.0, 401);
    let (status, ended) = rig.delete("/admin/v1/keys/leaked/revoke", &rig.gary).await;
    assert_eq!((status, ended["revoked"].clone()), (200, json!(false)));
    assert_eq!(rig.infer(&rig.leaked).await.0, 200);
    // Nothing left to end.
    assert_eq!(
        rig.delete("/admin/v1/keys/leaked/revoke", &rig.gary)
            .await
            .0,
        404
    );
    // And a short one ends by itself.
    rig.revoke(&rig.gary, "leaked", 1).await;
    assert_eq!(rig.infer(&rig.leaked).await.0, 401);
    tokio::time::sleep(Duration::from_millis(1300)).await;
    assert_eq!(rig.infer(&rig.leaked).await.0, 200);
}

#[tokio::test]
async fn a_revocation_names_a_key_of_the_configuration_and_cannot_be_made_twice() {
    let rig = rig(true).await;
    let (status, refused) = rig.revoke(&rig.gary, "nobody", 600).await;
    assert_eq!(status, 400, "{refused}");
    assert_eq!(refused["error"]["code"], "invalid_revocation");
    // An admin credential is not a client key.
    assert_eq!(rig.revoke(&rig.gary, "gary", 600).await.0, 400);
    assert_eq!(rig.revoke(&rig.gary, "leaked", 600).await.0, 201);
    let (status, again) = rig.revoke(&rig.gary, "leaked", 600).await;
    assert_eq!(status, 409, "{again}");
    assert_eq!(again["error"]["code"], "already_revoked");
    // An expiry past the overrides maximum, or none, is refused and nothing is stored.
    let (status, _) = rig.revoke(&rig.gary, "steady", 99_999_999).await;
    assert_eq!(status, 400);
    assert_eq!(rig.infer(&rig.steady).await.0, 200);
}

#[tokio::test]
async fn only_the_credentials_scope_may_revoke_a_key_and_a_client_key_never_can() {
    let rig = rig(true).await;
    for (who, expected) in [(&rig.robin, 403), (&rig.sam, 403)] {
        assert_eq!(rig.revoke(who, "leaked", 600).await.0, expected);
        assert_eq!(
            rig.delete("/admin/v1/keys/leaked/revoke", who).await.0,
            expected
        );
    }
    // A client key, the key itself included, gets the same 401 as for any admin endpoint.
    for key in [&rig.leaked, &rig.ops] {
        let (status, _) = rig.revoke(key, "leaked", 600).await;
        assert_eq!(status, 401);
    }
    assert_eq!(rig.infer(&rig.leaked).await.0, 200, "nothing was revoked");
}

#[tokio::test]
async fn without_a_ledger_a_key_cannot_be_revoked() {
    let rig = rig(false).await;
    let (status, body) = rig.revoke(&rig.gary, "leaked", 600).await;
    assert_eq!(status, 409, "{body}");
    assert_eq!(body["error"]["code"], "overrides_need_ledger");
}

#[tokio::test]
async fn a_key_revocation_is_not_an_override_and_nothing_but_the_admin_api_can_end_it() {
    let rig = rig(true).await;
    rig.revoke(&rig.gary, "leaked", 600).await;
    let id = rig.rows().await[0].0;
    // Not listed as an override, by an admin or by a key that manages overrides.
    let (_, listed) = rig
        .get("/admin/v1/overrides?history=true", &rig.robin)
        .await;
    assert_eq!(listed["overrides"].as_array().unwrap().len(), 0, "{listed}");
    let (_, client) = rig.get("/v1/overrides", &rig.ops).await;
    assert_eq!(client["overrides"].as_array().unwrap().len(), 0, "{client}");
    // Not counted with the overrides, nor with the admin revocations.
    let text = rig.metrics().await;
    assert!(
        text.contains("humpyard_admin_revocations_active 0"),
        "{text}"
    );
    for kind in ["disable_target", "drain_provider", "pin_route"] {
        assert!(
            text.contains(&format!("humpyard_overrides_active{{kind=\"{kind}\"}} 0")),
            "{text}"
        );
    }
    // The override endpoints cannot end it, so a key that manages overrides cannot restore a revoked key.
    assert_eq!(
        rig.delete(&format!("/v1/overrides/{id}"), &rig.ops).await.0,
        404
    );
    assert_eq!(
        rig.delete(&format!("/admin/v1/overrides/{id}"), &rig.robin)
            .await
            .0,
        404
    );
    assert_eq!(rig.rows().await[0].4, None, "and the row was never ended");
    assert_eq!(rig.infer(&rig.leaked).await.0, 401);
    // Routing is untouched: the other keys still reach the target.
    assert_eq!(rig.infer(&rig.steady).await.0, 200);
}

#[tokio::test]
async fn the_effective_configuration_marks_a_revoked_key_and_shows_no_secret() {
    let rig = rig(true).await;
    let (_, before) = rig.get("/admin/v1/config", &rig.sam).await;
    assert_eq!(before["keys"]["leaked"]["revoked"], false);
    rig.revoke(&rig.gary, "leaked", 600).await;
    let (status, view) = rig.get("/admin/v1/config", &rig.sam).await;
    assert_eq!(status, 200);
    assert_eq!(view["keys"]["leaked"]["revoked"], true);
    assert!(view["keys"]["leaked"]["revoked_until_ms"].as_i64().unwrap() > 0);
    assert_eq!(view["keys"]["steady"]["revoked"], false);
    assert_eq!(view["keys"]["steady"]["revoked_until_ms"], Value::Null);
    let text = view.to_string();
    assert!(
        !text.contains(&rig.leaked_hash) && !text.contains(&rig.leaked),
        "{text}"
    );
}

#[tokio::test]
async fn a_revocation_survives_a_restart() {
    let rig = rig(true).await;
    rig.revoke(&rig.gary, "leaked", 600).await;
    // A second gateway on the same ledger and configuration is what a restart brings up.
    let config = Config::from_toml(&rig.toml, |n| std::env::var(n).ok()).unwrap();
    let restarted = format!(
        "http://{}",
        serve(server::router(config).await.unwrap()).await
    );
    let refused = reqwest::Client::new()
        .post(format!("{restarted}/v1/chat/completions"))
        .bearer_auth(&rig.leaked)
        .json(&chat_request("r", false))
        .send()
        .await
        .unwrap();
    assert_eq!(refused.status().as_u16(), 401);
    let served = reqwest::Client::new()
        .post(format!("{restarted}/v1/chat/completions"))
        .bearer_auth(&rig.steady)
        .json(&chat_request("r", false))
        .send()
        .await
        .unwrap();
    assert_eq!(served.status().as_u16(), 200);
}
