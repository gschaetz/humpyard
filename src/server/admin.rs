//! `/admin/v1`: the admin API (ADR 0027, change `add-admin-api`). Everything here is authenticated by
//! [`crate::admin`], never by a client key, and the namespace does not exist (404) unless the configuration
//! defines an admin credential.

use std::sync::Arc;

use std::collections::HashMap;

use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use serde_json::json;

use super::AppState;
use crate::admin::{Principal, authenticate};
use crate::auth::presented_key;
use crate::config::{AdminConfig, AdminScope};

/// The admin routes. They carry no client authentication of any kind.
pub(super) fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/admin/v1/whoami", get(whoami))
        .route("/admin/v1/config", get(config))
        .route("/admin/v1/health", get(health))
        .route("/admin/v1/quota", get(quota))
        .route("/admin/v1/decisions", get(decisions))
        .route("/admin/v1/stats", get(stats))
        .route("/admin/v1/credentials", get(credentials))
}

/// Adds the admin routes to the main router, unless `[admin] listen` gives them their own address, in which
/// case they are never reachable on the public port (ADR 0027). Without a credential they answer 404.
pub(super) fn mount(app: Router<Arc<AppState>>, state: &AppState) -> Router<Arc<AppState>> {
    if state.snapshot().config.admin.listen.is_none() {
        app.merge(routes())
    } else {
        app
    }
}

/// Why the admin API turned a request away.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Denied {
    /// No admin credential is configured: the namespace does not exist.
    Absent,
    /// Missing, unknown or expired. One shape for every reason, so the answer says nothing about which
    /// credentials exist.
    Invalid,
    /// A valid credential that lacks the scope.
    Scope(AdminScope),
}

impl IntoResponse for Denied {
    fn into_response(self) -> Response {
        match self {
            Self::Absent => StatusCode::NOT_FOUND.into_response(),
            Self::Invalid => (
                StatusCode::UNAUTHORIZED,
                [(header::WWW_AUTHENTICATE, "Bearer")],
                Json(json!({"error": {
                    "message": "a valid admin credential is required",
                    "type": "authentication_error",
                    "code": "admin_credential_invalid",
                }})),
            )
                .into_response(),
            Self::Scope(scope) => (
                StatusCode::FORBIDDEN,
                Json(json!({"error": {
                    "message": format!("this credential needs the `{}` scope", scope.as_str()),
                    "type": "permission_error",
                    "code": "admin_scope_required",
                    "scope": scope.as_str(),
                }})),
            )
                .into_response(),
        }
    }
}

/// Authenticates the caller as an admin principal and, when `scope` is named, checks that it holds it. The
/// admin API being off is a plain 404, as if the namespace did not exist; each outcome is counted.
pub(super) fn authorize(
    state: &AppState,
    headers: &HeaderMap,
    scope: Option<AdminScope>,
) -> Result<Principal, Denied> {
    let snap = state.snapshot();
    if !snap.config.admin.enabled() {
        return Err(Denied::Absent);
    }
    let principal = authenticate(
        &snap.config.admin,
        presented_key(headers),
        state.clock.now_ms(),
    )
    .map_err(|_| {
        state.metrics.observe_admin_request("unauthorized");
        Denied::Invalid
    })?;
    if let Some(scope) = scope
        && !principal.may(scope)
    {
        state.metrics.observe_admin_request("forbidden");
        return Err(Denied::Scope(scope));
    }
    state.metrics.observe_admin_request("ok");
    Ok(principal)
}

/// Authorizes a read: any valid credential with the `read` scope.
fn read(state: &AppState, headers: &HeaderMap) -> Result<Principal, Denied> {
    authorize(state, headers, Some(AdminScope::Read))
}

/// A credential as the admin API shows it: who it is and what it may do, never a hash.
fn credential_json(id: &str, credential: &crate::config::AdminCredential) -> serde_json::Value {
    json!({
        "id": id,
        "kind": "credential",
        "scopes": credential.scopes.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
        "expires": credential.expires_utc(),
    })
}

/// The admin section of the configuration in full, for administrators; client keys that may read the
/// configuration see only a count (ADR 0027).
fn admin_detail(admin: &AdminConfig) -> serde_json::Value {
    let credentials: serde_json::Map<String, serde_json::Value> = admin
        .credentials
        .iter()
        .map(|(id, credential)| (id.clone(), credential_json(id, credential)))
        .collect();
    json!({
        "enabled": admin.enabled(),
        "listen": admin.listen.map(|addr| addr.to_string()),
        "credentials": credentials,
    })
}

/// The effective configuration, with the admin credentials listed by id, scopes and expiry.
async fn config(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    if let Err(denied) = read(&state, &headers) {
        return denied.into_response();
    }
    let snap = state.snapshot();
    let mut view = super::config_view::config_value(&state, &snap).await;
    view["admin"] = admin_detail(&snap.config.admin);
    Json(view).into_response()
}

async fn health(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    match read(&state, &headers) {
        Ok(_) => super::health_report(&state, &state.snapshot()),
        Err(denied) => denied.into_response(),
    }
}

async fn quota(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    match read(&state, &headers) {
        Ok(_) => super::quota::quota_report(&state, &state.snapshot()),
        Err(denied) => denied.into_response(),
    }
}

/// Decisions of every key, or of the one named by `key`.
async fn decisions(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(mut params): Query<HashMap<String, String>>,
) -> Response {
    if let Err(denied) = read(&state, &headers) {
        return denied.into_response();
    }
    let key = params.remove("key");
    super::decisions::decisions_view(&state, &params, key).await
}

/// Statistics of every key, or of the one named by `key`.
async fn stats(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(mut params): Query<HashMap<String, String>>,
) -> Response {
    if let Err(denied) = read(&state, &headers) {
        return denied.into_response();
    }
    let key = params.remove("key");
    super::stats::stats_view(&state, &state.snapshot(), &params, key).await
}

/// Every admin credential: id, scopes and expiry, never a hash.
async fn credentials(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    if let Err(denied) = read(&state, &headers) {
        return denied.into_response();
    }
    let snap = state.snapshot();
    let data: Vec<_> = snap
        .config
        .admin
        .credentials
        .iter()
        .map(|(id, credential)| credential_json(id, credential))
        .collect();
    Json(json!({"object": "list", "data": data})).into_response()
}

/// Who the caller is: their id, scopes and expiry. Any valid credential may ask.
async fn whoami(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    let principal = match authorize(&state, &headers, None) {
        Ok(principal) => principal,
        Err(denied) => return denied.into_response(),
    };
    Json(json!({
        "id": principal.id,
        "kind": "credential",
        "scopes": principal.scopes.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
        "expires": principal.expires,
    }))
    .into_response()
}

#[cfg(test)]
mod tests {
    use super::super::{Options, build};
    use super::*;
    use crate::config::Config;

    fn bearer(secret: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            format!("Bearer {secret}").parse().unwrap(),
        );
        headers
    }

    /// A gateway whose credentials are `reader` (read) and `operator` (overrides), with their secrets.
    async fn gateway() -> (Arc<AppState>, String, String) {
        let (reader, reader_hash) = crate::auth::generate_key().unwrap();
        let (operator, operator_hash) = crate::auth::generate_key().unwrap();
        let text = format!(
            r#"listen = "127.0.0.1:0"
[providers.p]
base_url = "http://127.0.0.1:1"
api_key_env = "PATH"
[targets.t]
endpoints = [{{ provider = "p", model = "m" }}]
[admin.credentials.reader]
sha256 = "{reader_hash}"
scopes = ["read"]
[admin.credentials.operator]
sha256 = "{operator_hash}"
scopes = ["overrides"]
"#
        );
        let config = Config::from_toml(&text, |name| std::env::var(name).ok()).unwrap();
        let built = build(config, Options::default(), "test".into())
            .await
            .unwrap();
        (built.state, reader, operator)
    }

    async fn body_of(response: Response) -> serde_json::Value {
        let bytes = axum::body::to_bytes(response.into_body(), 4096)
            .await
            .unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    #[tokio::test]
    async fn a_credential_without_the_scope_is_refused_naming_it() {
        let (state, reader, operator) = gateway().await;
        let refusal = authorize(&state, &bearer(&reader), Some(AdminScope::Overrides)).unwrap_err();
        assert_eq!(refusal, Denied::Scope(AdminScope::Overrides));
        let response = refusal.into_response();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        let body = body_of(response).await;
        assert_eq!(body["error"]["code"], "admin_scope_required");
        assert_eq!(body["error"]["scope"], "overrides");
        // The same credential holds the scope it was given, and the operator holds its own.
        assert_eq!(
            authorize(&state, &bearer(&reader), Some(AdminScope::Read))
                .unwrap()
                .id,
            "reader"
        );
        assert_eq!(
            authorize(&state, &bearer(&operator), Some(AdminScope::Overrides))
                .unwrap()
                .id,
            "operator"
        );
        let refusal = authorize(&state, &bearer(&operator), Some(AdminScope::Read)).unwrap_err();
        assert_eq!(refusal, Denied::Scope(AdminScope::Read));
        assert_eq!(
            body_of(refusal.into_response()).await["error"]["scope"],
            "read"
        );
    }

    #[tokio::test]
    async fn no_scope_asked_means_any_valid_credential() {
        let (state, reader, _) = gateway().await;
        assert!(authorize(&state, &bearer(&reader), None).is_ok());
        let refusal = authorize(&state, &HeaderMap::new(), None).unwrap_err();
        assert_eq!(refusal, Denied::Invalid);
        assert_eq!(refusal.into_response().status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn each_outcome_is_counted() {
        let (state, reader, _) = gateway().await;
        let _ = authorize(&state, &bearer(&reader), Some(AdminScope::Read));
        let _ = authorize(&state, &bearer(&reader), Some(AdminScope::Credentials));
        let _ = authorize(&state, &bearer("not-a-real-credential"), None);
        let text = state.metrics.render(Vec::new());
        for result in ["ok", "forbidden", "unauthorized"] {
            assert!(
                text.contains(&format!(
                    "humpyard_admin_requests_total{{result=\"{result}\"}} 1"
                )),
                "{result}: {text}"
            );
        }
    }
}
