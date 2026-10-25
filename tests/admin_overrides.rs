//! Overrides and revocation through the admin API (spec: `admin-api`, change `add-admin-api`): an admin manages
//! overrides under the same rules as a client key, recorded as `admin:<id>`; an admin with the `credentials`
//! scope revokes another credential without a deploy; and a revocation is invisible to everything that plans a
//! request or that a client key can see, and cannot be undone by anything but the admin API.
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
    url: String,
    ledger: std::path::PathBuf,
    ops: String,
    gary: String,
    sam: String,
    robin: String,
    tmp: String,
    metrics: String,
    _dir: tempfile::TempDir,
}

async fn completions() -> axum::response::Response {
    axum::Json(chat_completion("m", "ok", Some(usage(10, 5)))).into_response()
}

/// `with_ledger` false gives a gateway that cannot store an override or a revocation.
async fn rig(with_ledger: bool) -> Rig {
    let upstream = format!(
        "http://{}",
        serve(Router::new().route("/chat/completions", post(completions))).await
    );
    let gen_key = || humpyard::auth::generate_key().unwrap();
    let (
        (ops, ops_h),
        (gary, gary_h),
        (sam, sam_h),
        (robin, robin_h),
        (tmp, tmp_h),
        (metrics, metrics_h),
    ) = (
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
scopes = ["overrides"]
[admin.credentials.tmp]
sha256 = "{tmp_h}"
scopes = ["credentials"]
[providers.p]
base_url = "{upstream}"
api_key_env = "PATH"
max_retries = 0
[targets.a]
endpoints = [{{ provider = "p", model = "m" }}]
[targets.b]
endpoints = [{{ provider = "p", model = "m" }}]
[routes.r]
type = "passthrough"
targets = ["a", "b"]
"#
    );
    let config = Config::from_toml(&toml, |n| std::env::var(n).ok()).unwrap();
    let url = format!(
        "http://{}",
        serve(server::router(config).await.unwrap()).await
    );
    Rig {
        url,
        ledger,
        ops,
        gary,
        sam,
        robin,
        tmp,
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

    async fn revoke(&self, by: &str, who: &str, secs: u64) -> (u16, Value) {
        self.post(
            &format!("/admin/v1/credentials/{who}/revoke"),
            by,
            json!({"reason": "left the team", "expires_in_secs": secs}),
        )
        .await
    }

    async fn whoami(&self, key: &str) -> u16 {
        self.get("/admin/v1/whoami", key).await.0
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

fn disable(target: &str) -> Value {
    json!({"kind": "disable_target", "subject": target, "reason": "testing", "expires_in_secs": 600})
}

#[tokio::test]
async fn an_admin_manages_overrides_and_is_recorded_as_the_creator() {
    let rig = rig(true).await;
    let (status, created) = rig
        .post("/admin/v1/overrides", &rig.gary, disable("a"))
        .await;
    assert_eq!(status, 201, "{created}");
    assert_eq!(created["created_by"], "admin:gary");
    let id = created["id"].as_i64().unwrap();
    // It is the same override a client key that manages overrides sees, and lists.
    let (_, listed) = rig.get("/admin/v1/overrides", &rig.sam).await;
    assert_eq!(listed["overrides"][0]["subject"], "a");
    let (_, client) = rig.get("/v1/overrides", &rig.ops).await;
    assert_eq!(client["overrides"][0]["created_by"], "admin:gary");
    // And an admin removes it.
    let (status, removed) = rig
        .delete(&format!("/admin/v1/overrides/{id}"), &rig.gary)
        .await;
    assert_eq!((status, removed["removed"].clone()), (200, json!(true)));
    assert_eq!(
        rig.get("/v1/overrides", &rig.ops).await.1["overrides"],
        json!([])
    );
    // The audit rows name the admin for the creation.
    let rows = rig.rows().await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].3, "admin:gary");
}

#[tokio::test]
async fn the_rules_are_the_client_endpoints_rules() {
    let rig = rig(true).await;
    for body in [
        disable("nope"),
        json!({"kind": "drain_provider", "subject": "ghost", "reason": "x"}),
        json!({"kind": "pin_route", "subject": "a", "route": "r", "reason": ""}),
        json!({"kind": "disable_target", "subject": "a", "reason": "x", "expires_in_secs": 999_999_999}),
    ] {
        let admin = rig
            .post("/admin/v1/overrides", &rig.gary, body.clone())
            .await;
        let client = rig.post("/v1/overrides", &rig.ops, body.clone()).await;
        assert_eq!(admin.0, client.0, "{body}");
        assert_eq!(
            admin.1["error"]["code"], client.1["error"]["code"],
            "{body}"
        );
        assert_eq!(admin.0, 400, "{body}");
    }
    assert_eq!(rig.rows().await.len(), 0, "nothing was created");
}

#[tokio::test]
async fn managing_overrides_needs_the_overrides_scope_and_listing_needs_read() {
    let rig = rig(true).await;
    let (status, body) = rig
        .post("/admin/v1/overrides", &rig.sam, disable("a"))
        .await;
    assert_eq!(
        (status, body["error"]["scope"].clone()),
        (403, json!("overrides"))
    );
    let (status, body) = rig.delete("/admin/v1/overrides/1", &rig.sam).await;
    assert_eq!(
        (status, body["error"]["scope"].clone()),
        (403, json!("overrides"))
    );
    let (status, body) = rig.get("/admin/v1/overrides", &rig.robin).await;
    assert_eq!(
        (status, body["error"]["scope"].clone()),
        (403, json!("read"))
    );
    assert_eq!(
        rig.post("/admin/v1/overrides", &rig.ops, disable("a"))
            .await
            .0,
        401
    );
}

#[tokio::test]
async fn revoking_a_credential_takes_effect_on_its_next_request_without_a_reload() {
    let rig = rig(true).await;
    assert_eq!(rig.whoami(&rig.sam).await, 200);
    let (status, revoked) = rig.revoke(&rig.gary, "sam", 600).await;
    assert_eq!(status, 201, "{revoked}");
    assert_eq!(revoked["created_by"], "admin:gary");
    assert_eq!(rig.whoami(&rig.sam).await, 401);
    // Every endpoint refuses it, with the same answer as an unknown secret.
    let (status, body) = rig.get("/admin/v1/config", &rig.sam).await;
    assert_eq!(
        (status, body["error"]["code"].clone()),
        (401, json!("admin_credential_invalid"))
    );
    // The others are unaffected, and the listing says who is revoked.
    assert_eq!(rig.whoami(&rig.gary).await, 200);
    let (_, list) = rig.get("/admin/v1/credentials", &rig.gary).await;
    let revoked_of = |id: &str| {
        list["data"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["id"] == id)
            .unwrap()["revoked"]
            .clone()
    };
    assert_eq!(
        (revoked_of("sam"), revoked_of("gary")),
        (json!(true), json!(false))
    );
    // The audit row names the revoker and the revoked.
    let rows = rig.rows().await;
    assert_eq!(rows.len(), 1);
    assert_eq!(
        (rows[0].1.as_str(), rows[0].2.as_str(), rows[0].3.as_str()),
        ("revoke_admin_credential", "sam", "admin:gary")
    );
}

#[tokio::test]
async fn a_revocation_ends_when_removed_or_when_it_expires() {
    let rig = rig(true).await;
    rig.revoke(&rig.gary, "sam", 600).await;
    assert_eq!(rig.whoami(&rig.sam).await, 401);
    let (status, ended) = rig
        .delete("/admin/v1/credentials/sam/revoke", &rig.gary)
        .await;
    assert_eq!((status, ended["revoked"].clone()), (200, json!(false)));
    assert_eq!(rig.whoami(&rig.sam).await, 200);
    // Nothing left to end.
    assert_eq!(
        rig.delete("/admin/v1/credentials/sam/revoke", &rig.gary)
            .await
            .0,
        404
    );
    // And a short one ends by itself.
    rig.revoke(&rig.gary, "sam", 1).await;
    assert_eq!(rig.whoami(&rig.sam).await, 401);
    for _ in 0..80 {
        if rig.whoami(&rig.sam).await == 200 {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("the revocation did not end");
}

#[tokio::test]
async fn a_revocation_is_refused_for_anything_that_is_not_an_admin_credential_or_is_malformed() {
    let rig = rig(true).await;
    for (who, body) in [
        ("nobody", json!({"reason": "x"})),
        ("ops", json!({"reason": "x"})),
        ("sam", json!({"reason": "   "})),
        ("sam", json!({"reason": "x", "expires_in_secs": 0})),
        (
            "sam",
            json!({"reason": "x", "expires_in_secs": 999_999_999}),
        ),
    ] {
        let (status, response) = rig
            .post(
                &format!("/admin/v1/credentials/{who}/revoke"),
                &rig.gary,
                body.clone(),
            )
            .await;
        assert_eq!(status, 400, "{who} {body}: {response}");
        assert_eq!(response["error"]["code"], "invalid_revocation");
    }
    assert_eq!(rig.rows().await.len(), 0, "nothing changed");
    assert_eq!(rig.whoami(&rig.sam).await, 200);
    // Revoking twice is refused, not stacked.
    assert_eq!(rig.revoke(&rig.gary, "sam", 600).await.0, 201);
    let (status, again) = rig.revoke(&rig.gary, "sam", 600).await;
    assert_eq!(
        (status, again["error"]["code"].clone()),
        (409, json!("already_revoked"))
    );
}

#[tokio::test]
async fn revoking_needs_the_credentials_scope() {
    let rig = rig(true).await;
    for key in [&rig.sam, &rig.robin] {
        let (status, body) = rig.revoke(key, "tmp", 600).await;
        assert_eq!(
            (status, body["error"]["scope"].clone()),
            (403, json!("credentials"))
        );
        assert_eq!(
            rig.delete("/admin/v1/credentials/tmp/revoke", key).await.0,
            403
        );
    }
    assert_eq!(
        rig.revoke(&rig.ops, "tmp", 600).await.0,
        401,
        "a client key never revokes"
    );
    assert_eq!(rig.whoami(&rig.tmp).await, 200);
}

#[tokio::test]
async fn an_admin_can_revoke_themselves_and_a_colleague_can_restore_them() {
    let rig = rig(true).await;
    assert_eq!(rig.revoke(&rig.tmp, "tmp", 600).await.0, 201);
    assert_eq!(rig.whoami(&rig.tmp).await, 401);
    assert_eq!(
        rig.delete("/admin/v1/credentials/tmp/revoke", &rig.gary)
            .await
            .0,
        200
    );
    assert_eq!(rig.whoami(&rig.tmp).await, 200);
}

#[tokio::test]
async fn without_a_ledger_nothing_can_be_revoked() {
    let rig = rig(false).await;
    let (status, body) = rig.revoke(&rig.gary, "sam", 600).await;
    assert_eq!(
        (status, body["error"]["code"].clone()),
        (409, json!("overrides_need_ledger"))
    );
    assert_eq!(rig.whoami(&rig.sam).await, 200);
    let (status, _) = rig
        .post("/admin/v1/overrides", &rig.gary, disable("a"))
        .await;
    assert_eq!(status, 409);
}

#[tokio::test]
async fn a_revocation_is_invisible_to_clients_and_to_planning_and_cannot_be_removed_by_them() {
    let rig = rig(true).await;
    rig.revoke(&rig.gary, "sam", 600).await;
    let id = rig.rows().await[0].0;
    // Client views of overrides never list it, in force or in history.
    assert_eq!(
        rig.get("/v1/overrides", &rig.ops).await.1["overrides"],
        json!([])
    );
    assert_eq!(
        rig.get("/v1/overrides?history=true", &rig.ops).await.1["overrides"],
        json!([])
    );
    assert_eq!(
        rig.get("/admin/v1/overrides", &rig.gary).await.1["overrides"],
        json!([])
    );
    // A key that manages overrides cannot end it, by id, through the client or the admin override endpoint.
    assert_eq!(
        rig.delete(&format!("/v1/overrides/{id}"), &rig.ops).await.0,
        404
    );
    assert_eq!(
        rig.delete(&format!("/admin/v1/overrides/{id}"), &rig.gary)
            .await
            .0,
        404
    );
    assert_eq!(rig.whoami(&rig.sam).await, 401, "still revoked");
    assert_eq!(rig.rows().await[0].4, None, "and the row was never ended");
    // It is not counted with the overrides, and has its own gauge.
    let text = rig.metrics().await;
    for kind in ["disable_target", "drain_provider", "pin_route"] {
        assert!(
            text.contains(&format!("humpyard_overrides_active{{kind=\"{kind}\"}} 0")),
            "{text}"
        );
    }
    assert!(
        text.contains("humpyard_admin_revocations_active 1"),
        "{text}"
    );
    assert!(!text.contains("revoke_admin_credential"), "{text}");
    // Planning is untouched: both targets still serve, and the decision records no override.
    let (_, config) = rig.get("/admin/v1/config", &rig.gary).await;
    assert_eq!(config["targets"]["a"]["source"], "file");
    let response = reqwest::Client::new()
        .post(format!("{}/v1/chat/completions", rig.url))
        .bearer_auth(&rig.ops)
        .json(&chat_request("r", false))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["x-humpyard-target"], "a");
}

#[tokio::test]
async fn the_gauge_counts_the_revocations_in_force() {
    let rig = rig(true).await;
    assert!(
        rig.metrics()
            .await
            .contains("humpyard_admin_revocations_active 0")
    );
    rig.revoke(&rig.gary, "sam", 600).await;
    rig.revoke(&rig.gary, "robin", 600).await;
    assert!(
        rig.metrics()
            .await
            .contains("humpyard_admin_revocations_active 2")
    );
    rig.delete("/admin/v1/credentials/sam/revoke", &rig.gary)
        .await;
    assert!(
        rig.metrics()
            .await
            .contains("humpyard_admin_revocations_active 1")
    );
}
