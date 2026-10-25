//! `/admin/v1`: the admin API (ADR 0027, change `add-admin-api`). Everything here is authenticated by
//! [`crate::admin`], never by a client key, and the namespace does not exist (404) unless the configuration
//! defines an admin credential.

use std::sync::Arc;

use std::collections::{BTreeMap, HashMap};

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::MethodRouter;
use axum::{Json, Router};
use serde_json::json;

use super::AppState;
use crate::admin::{Principal, authenticate};
use crate::auth::presented_key;
use crate::config::{AdminConfig, AdminScope};
use crate::overrides::OverrideError;

type Register = fn(MethodRouter<Arc<AppState>>) -> MethodRouter<Arc<AppState>>;

/// One operation of the admin API: the single source for the router, for the scope each handler demands of a
/// caller, and for the drift test against `docs/openapi/admin-v1.json`.
// `scope` and `codes` are read by the drift tests; the router itself only needs the registration.
#[cfg_attr(not(test), allow(dead_code))]
pub(super) struct Route {
    pub(super) method: &'static str,
    pub(super) path: &'static str,
    /// The scope a caller must hold; `None` means any valid credential.
    pub(super) scope: Option<AdminScope>,
    /// The error codes this operation can answer with, beyond the ones every operation shares.
    pub(super) codes: &'static [&'static str],
    register: Register,
}

const OVERRIDES_CODES: &[&str] = &["invalid_override", "overrides_need_ledger", "ledger_error"];

/// Every `/admin/v1` operation. Adding a route here without describing it in the `OpenAPI` document, or the
/// reverse, fails `the_openapi_document_describes_exactly_the_routes`.
pub(super) const ROUTES: &[Route] = &[
    Route {
        method: "GET",
        path: "/admin/v1/whoami",
        scope: None,
        codes: &[],
        register: |m| m.get(whoami),
    },
    Route {
        method: "GET",
        path: "/admin/v1/openapi.json",
        scope: None,
        codes: &[],
        register: |m| m.get(openapi),
    },
    Route {
        method: "GET",
        path: "/admin/v1/config",
        scope: Some(AdminScope::Read),
        codes: &[],
        register: |m| m.get(config),
    },
    Route {
        method: "GET",
        path: "/admin/v1/health",
        scope: Some(AdminScope::Read),
        codes: &[],
        register: |m| m.get(health),
    },
    Route {
        method: "GET",
        path: "/admin/v1/quota",
        scope: Some(AdminScope::Read),
        codes: &[],
        register: |m| m.get(quota),
    },
    Route {
        method: "GET",
        path: "/admin/v1/decisions",
        scope: Some(AdminScope::Read),
        codes: &[],
        register: |m| m.get(decisions),
    },
    Route {
        method: "GET",
        path: "/admin/v1/stats",
        scope: Some(AdminScope::Read),
        codes: &[],
        register: |m| m.get(stats),
    },
    Route {
        method: "GET",
        path: "/admin/v1/credentials",
        scope: Some(AdminScope::Read),
        codes: &[],
        register: |m| m.get(credentials),
    },
    Route {
        method: "POST",
        path: "/admin/v1/credentials/{id}/revoke",
        scope: Some(AdminScope::Credentials),
        codes: &[
            "invalid_revocation",
            "overrides_need_ledger",
            "already_revoked",
            "ledger_error",
        ],
        register: |m| m.post(revoke_credential),
    },
    Route {
        method: "DELETE",
        path: "/admin/v1/credentials/{id}/revoke",
        scope: Some(AdminScope::Credentials),
        codes: &["revocation_not_found"],
        register: |m| m.delete(end_revocation),
    },
    Route {
        method: "GET",
        path: "/admin/v1/overrides",
        scope: Some(AdminScope::Read),
        codes: &[],
        register: |m| m.get(list_overrides),
    },
    Route {
        method: "POST",
        path: "/admin/v1/overrides",
        scope: Some(AdminScope::Overrides),
        codes: OVERRIDES_CODES,
        register: |m| m.post(create_override),
    },
    Route {
        method: "DELETE",
        path: "/admin/v1/overrides/{id}",
        scope: Some(AdminScope::Overrides),
        codes: &["override_not_found", "ledger_error"],
        register: |m| m.delete(remove_override),
    },
];

#[cfg_attr(not(test), allow(dead_code))]
impl Route {
    /// Every error code this operation can answer with: the shared authentication codes, the scope code when it
    /// needs a scope, and its own.
    pub(super) fn error_codes(&self) -> Vec<&'static str> {
        let mut codes = vec!["admin_credential_invalid"];
        if self.scope.is_some() {
            codes.push("admin_scope_required");
        }
        codes.extend_from_slice(self.codes);
        codes
    }
}

/// The admin routes, built from [`ROUTES`]. They carry no client authentication of any kind.
pub(super) fn routes() -> Router<Arc<AppState>> {
    let mut by_path: BTreeMap<&str, MethodRouter<Arc<AppState>>> = BTreeMap::new();
    for route in ROUTES {
        let method_router = by_path.remove(route.path).unwrap_or_default();
        by_path.insert(route.path, (route.register)(method_router));
    }
    by_path
        .into_iter()
        .fold(Router::new(), |router, (path, methods)| {
            router.route(path, methods)
        })
}

/// The published contract, embedded so the gateway serves exactly what the repository documents.
const OPENAPI: &str = include_str!("../../docs/openapi/admin-v1.json");

/// The `OpenAPI` document of the admin API, for any valid credential.
async fn openapi(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    match authorize(&state, &headers, None) {
        Ok(_) => ([(header::CONTENT_TYPE, "application/json")], OPENAPI).into_response(),
        Err(denied) => denied.into_response(),
    }
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
    // A revoked credential is refused exactly like an unknown one, on its very next request.
    let revoked = state
        .overrides
        .as_ref()
        .is_some_and(|store| store.is_revoked(&principal.id, state.clock.now_ms()));
    if revoked {
        state.metrics.observe_admin_request("unauthorized");
        return Err(Denied::Invalid);
    }
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
    let now_ms = state.clock.now_ms();
    let data: Vec<_> = snap
        .config
        .admin
        .credentials
        .iter()
        .map(|(id, credential)| {
            let mut row = credential_json(id, credential);
            row["revoked"] = json!(
                state
                    .overrides
                    .as_ref()
                    .is_some_and(|store| store.is_revoked(id, now_ms))
            );
            row
        })
        .collect();
    Json(json!({"object": "list", "data": data})).into_response()
}

fn fail(status: StatusCode, kind: &str, code: &str, message: impl Into<String>) -> Response {
    (
        status,
        Json(json!({"error": {"message": message.into(), "type": kind, "code": code}})),
    )
        .into_response()
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct RevokeBody {
    reason: String,
    expires_in_secs: Option<u64>,
}

/// Revokes another credential (or the caller's own) for a bounded time. It only removes access, and it ends by
/// itself; editing the file and reloading is how a credential is removed for good.
async fn revoke_credential(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(body): Json<RevokeBody>,
) -> Response {
    let principal = match authorize(&state, &headers, Some(AdminScope::Credentials)) {
        Ok(principal) => principal,
        Err(denied) => return denied.into_response(),
    };
    let snap = state.snapshot();
    if !snap.config.admin.credentials.contains_key(&id) {
        return fail(
            StatusCode::BAD_REQUEST,
            "invalid_request_error",
            "invalid_revocation",
            format!("`{id}` is not an admin credential in the loaded configuration"),
        );
    }
    let Some(store) = state.overrides.clone() else {
        return fail(
            StatusCode::CONFLICT,
            "invalid_request_error",
            "overrides_need_ledger",
            "revocations need a `[ledger]`: without one nothing would survive a restart",
        );
    };
    let now_ms = state.clock.now_ms();
    if store.is_revoked(&id, now_ms) {
        return fail(
            StatusCode::CONFLICT,
            "invalid_request_error",
            "already_revoked",
            format!("`{id}` is already revoked; end that revocation first to change it"),
        );
    }
    let by = principal.audit_name();
    match store
        .revoke_principal(
            &id,
            &body.reason,
            body.expires_in_secs,
            &by,
            snap.config.overrides,
            now_ms,
        )
        .await
    {
        Ok(revocation) => {
            tracing::warn!(
                credential = %id,
                by = %by,
                expires_ms = revocation.expires_ms,
                "admin credential revoked"
            );
            (
                StatusCode::CREATED,
                Json(json!({
                    "credential": revocation.principal,
                    "revoked": true,
                    "created_by": revocation.created_by,
                    "expires_ms": revocation.expires_ms,
                    "expires_in_secs": (revocation.expires_ms.saturating_sub(now_ms) / 1000).max(0),
                })),
            )
                .into_response()
        }
        Err(OverrideError::Ledger(error)) => {
            tracing::error!(%error, "could not store a revocation");
            fail(
                StatusCode::INTERNAL_SERVER_ERROR,
                "api_error",
                "ledger_error",
                "the revocation could not be stored",
            )
        }
        Err(error) => fail(
            StatusCode::BAD_REQUEST,
            "invalid_request_error",
            "invalid_revocation",
            error.to_string(),
        ),
    }
}

/// Ends the revocation of a credential before it expires.
async fn end_revocation(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    let principal = match authorize(&state, &headers, Some(AdminScope::Credentials)) {
        Ok(principal) => principal,
        Err(denied) => return denied.into_response(),
    };
    let by = principal.audit_name();
    let ended = match &state.overrides {
        Some(store) => store
            .end_revocations(&id, &by, state.clock.now_ms())
            .await
            .unwrap_or_else(|error| {
                tracing::error!(%error, "could not end a revocation");
                false
            }),
        None => false,
    };
    if ended {
        tracing::warn!(credential = %id, by = %by, "admin credential revocation ended");
        Json(json!({"credential": id, "revoked": false})).into_response()
    } else {
        fail(
            StatusCode::NOT_FOUND,
            "not_found_error",
            "revocation_not_found",
            format!("`{id}` has no revocation in force"),
        )
    }
}

/// The runtime overrides in force (`?history=true` adds those of the last 30 days), as `/v1/overrides` lists
/// them. Credential revocations are not among them.
async fn list_overrides(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    if let Err(denied) = read(&state, &headers) {
        return denied.into_response();
    }
    super::overrides::list_core(&state, &state.snapshot(), &params).await
}

/// Creates an override under the rules of `POST /v1/overrides`, recorded as made by `admin:<id>`.
async fn create_override(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<super::overrides::CreateBody>,
) -> Response {
    let principal = match authorize(&state, &headers, Some(AdminScope::Overrides)) {
        Ok(principal) => principal,
        Err(denied) => return denied.into_response(),
    };
    super::overrides::create_core(&state, &state.snapshot(), body, &principal.audit_name()).await
}

/// Removes an override in force, recorded as removed by `admin:<id>`.
async fn remove_override(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> Response {
    let principal = match authorize(&state, &headers, Some(AdminScope::Overrides)) {
        Ok(principal) => principal,
        Err(denied) => return denied.into_response(),
    };
    super::overrides::remove_core(&state, id, &principal.audit_name()).await
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
    use axum::Router;

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

    /// The admin API as a router with a credential of each kind of scope, and their secrets.
    async fn served() -> (Router, [String; 3]) {
        let keys: Vec<_> = (0..3)
            .map(|_| crate::auth::generate_key().unwrap())
            .collect();
        let text = format!(
            r#"listen = "127.0.0.1:0"
[providers.p]
base_url = "http://127.0.0.1:1"
api_key_env = "PATH"
[targets.t]
endpoints = [{{ provider = "p", model = "m" }}]
[admin.credentials.reader]
sha256 = "{}"
scopes = ["read"]
[admin.credentials.operator]
sha256 = "{}"
scopes = ["overrides"]
[admin.credentials.custodian]
sha256 = "{}"
scopes = ["credentials"]
"#,
            keys[0].1, keys[1].1, keys[2].1
        );
        let config = Config::from_toml(&text, |name| std::env::var(name).ok()).unwrap();
        let built = build(config, Options::default(), "test".into())
            .await
            .unwrap();
        let [a, b, c] = [&keys[0].0, &keys[1].0, &keys[2].0].map(Clone::clone);
        (routes().with_state(built.state), [a, b, c])
    }

    async fn call(
        app: &Router,
        route: &Route,
        secret: Option<&str>,
    ) -> (StatusCode, serde_json::Value) {
        use tower::ServiceExt;
        let uri = route.path.replace("{id}", "1");
        let body = match (route.method, route.path) {
            ("POST", "/admin/v1/overrides") => {
                json!({"kind": "disable_target", "subject": "t", "reason": "x"})
            }
            ("POST", _) => json!({"reason": "x"}),
            _ => json!(null),
        };
        let mut request = axum::http::Request::builder()
            .method(route.method)
            .uri(uri)
            .header(header::CONTENT_TYPE, "application/json");
        if let Some(secret) = secret {
            request = request.header(header::AUTHORIZATION, format!("Bearer {secret}"));
        }
        let request = request
            .body(axum::body::Body::from(body.to_string()))
            .unwrap();
        let response = app.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
            .await
            .unwrap();
        (status, serde_json::from_slice(&bytes).unwrap_or_default())
    }

    #[test]
    fn the_openapi_document_describes_exactly_the_routes() {
        let doc: serde_json::Value = serde_json::from_str(OPENAPI).unwrap();
        let mut described = std::collections::BTreeSet::new();
        for (path, item) in doc["paths"].as_object().unwrap() {
            for (method, operation) in item.as_object().unwrap() {
                let scope = operation["security"][0]["adminBearer"][0]
                    .as_str()
                    .map(str::to_owned);
                let mut codes: Vec<String> = operation["x-error-codes"]
                    .as_array()
                    .unwrap_or_else(|| panic!("{method} {path} lists no x-error-codes"))
                    .iter()
                    .map(|c| c.as_str().unwrap().to_owned())
                    .collect();
                codes.sort();
                // Every `{name}` in the path is declared as a parameter.
                for name in path
                    .split('/')
                    .filter_map(|s| s.strip_prefix('{')?.strip_suffix('}'))
                {
                    let declared = operation["parameters"].as_array().is_some_and(|ps| {
                        ps.iter().any(|p| p["name"] == name && p["in"] == "path")
                    });
                    assert!(declared, "{method} {path} does not declare `{name}`");
                }
                described.insert((method.to_uppercase(), path.clone(), scope, codes));
            }
        }
        let served: std::collections::BTreeSet<_> = ROUTES
            .iter()
            .map(|r| {
                let mut codes: Vec<String> =
                    r.error_codes().iter().map(|c| (*c).to_owned()).collect();
                codes.sort();
                (
                    r.method.to_owned(),
                    r.path.to_owned(),
                    r.scope.map(|s| s.as_str().to_owned()),
                    codes,
                )
            })
            .collect();
        let only_served: Vec<_> = served.difference(&described).collect();
        let only_described: Vec<_> = described.difference(&served).collect();
        assert!(
            only_served.is_empty() && only_described.is_empty(),
            "served but not described (or described differently): {only_served:#?}\n\
             described but not served: {only_described:#?}"
        );
    }

    #[tokio::test]
    async fn every_route_demands_the_scope_the_table_says() {
        let (app, [reader, operator, custodian]) = served().await;
        for route in ROUTES {
            let (status, body) = call(&app, route, None).await;
            assert_eq!(
                status,
                StatusCode::UNAUTHORIZED,
                "{} {}",
                route.method,
                route.path
            );
            assert_eq!(body["error"]["code"], "admin_credential_invalid");
            // A credential holding a different scope is refused, naming the one the table lists.
            if let Some(scope) = route.scope {
                let other = match scope {
                    AdminScope::Read => &operator,
                    AdminScope::Overrides | AdminScope::Credentials => &reader,
                };
                let (status, body) = call(&app, route, Some(other)).await;
                assert_eq!(
                    status,
                    StatusCode::FORBIDDEN,
                    "{} {}",
                    route.method,
                    route.path
                );
                assert_eq!(
                    body["error"]["scope"],
                    scope.as_str(),
                    "{} {}",
                    route.method,
                    route.path
                );
            } else {
                // No scope listed: any valid credential gets through.
                for secret in [&reader, &operator, &custodian] {
                    let (status, _) = call(&app, route, Some(secret)).await;
                    assert_eq!(status, StatusCode::OK, "{} {}", route.method, route.path);
                }
            }
        }
    }

    #[tokio::test]
    async fn the_served_document_is_the_embedded_one() {
        let (app, [reader, ..]) = served().await;
        let route = ROUTES
            .iter()
            .find(|r| r.path.ends_with("openapi.json"))
            .unwrap();
        let (status, body) = call(&app, route, Some(&reader)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            body,
            serde_json::from_str::<serde_json::Value>(OPENAPI).unwrap()
        );
    }
}
