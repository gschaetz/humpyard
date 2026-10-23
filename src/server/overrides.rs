//! `POST`, `GET` and `DELETE /v1/overrides`: expiring runtime overrides (spec: `gateway-api`).
//!
//! An override only removes candidates, always expires and is audited (ADR 0025). Writing needs a key with
//! `manage_overrides`; listing needs that or `read_config`; a gateway with no keys is open, and so is this.
//! Subjects are checked against the configuration being served, so an override can never name something that
//! does not exist, and a pin can never add reach.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::{Value, json};
use switchyard_protocol::WireFormat;

use super::{AppState, Snapshot, authenticate, class_allows};
use crate::auth::KeyRecord;
use crate::overrides::{Create, Kind, Override, OverrideError, OverrideStore};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CreateBody {
    kind: String,
    subject: String,
    route: Option<String>,
    reason: String,
    expires_in_secs: Option<u64>,
}

fn refuse(status: StatusCode, kind: &str, code: &str, message: impl Into<String>) -> Response {
    (
        status,
        Json(json!({"error": {"message": message.into(), "type": kind, "code": code}})),
    )
        .into_response()
}

fn not_manageable() -> Response {
    refuse(
        StatusCode::FORBIDDEN,
        "permission_error",
        "overrides_not_manageable",
        "this key may not manage overrides (set `manage_overrides = true` on it)",
    )
}

/// `None` is an open gateway (no keys configured), which may do anything.
fn may_manage(caller: Option<&KeyRecord>) -> bool {
    caller.is_none_or(|key| key.manage_overrides)
}

fn may_list(caller: Option<&KeyRecord>) -> bool {
    caller.is_none_or(|key| key.manage_overrides || key.read_config)
}

fn override_json(o: &Override, stale: bool) -> Value {
    json!({
        "id": o.id,
        "kind": o.kind.as_str(),
        "subject": o.subject,
        "route": o.route,
        "reason": o.reason,
        "created_ms": o.created_ms,
        "expires_ms": o.expires_ms,
        "created_by": o.created_by,
        "stale": stale,
    })
}

/// Whether an override names something the configuration being served no longer has (a reload removed it).
/// It stays stored and has no effect until it expires.
fn is_stale(snap: &Snapshot, o: &Override) -> bool {
    match o.kind {
        Kind::DisableTarget => !snap.config.targets.contains_key(&o.subject),
        Kind::DrainProvider => !snap.config.providers.contains_key(&o.subject),
        Kind::PinRoute => !o.route.as_deref().is_some_and(|route| {
            snap.config
                .routes
                .get(route)
                .is_some_and(|spec| spec.target_refs().contains(&o.subject.as_str()))
        }),
    }
}

/// Whether the configuration being served allows this override to be created: it names things that exist,
/// and a pin neither adds a target to the route nor crosses a class a strict route forbids.
fn check_subject(
    snap: &Snapshot,
    kind: Kind,
    subject: &str,
    route: Option<&str>,
) -> Result<(), String> {
    match kind {
        Kind::DisableTarget => snap
            .config
            .targets
            .contains_key(subject)
            .then_some(())
            .ok_or_else(|| format!("target `{subject}` is not in the loaded configuration")),
        Kind::DrainProvider => snap
            .config
            .providers
            .contains_key(subject)
            .then_some(())
            .ok_or_else(|| format!("provider `{subject}` is not in the loaded configuration")),
        Kind::PinRoute => {
            let route = route.ok_or("`pin_route` names a route and a target")?;
            let spec = snap
                .config
                .routes
                .get(route)
                .ok_or_else(|| format!("route `{route}` is not in the loaded configuration"))?;
            if !spec.target_refs().contains(&subject) {
                return Err(format!(
                    "target `{subject}` is not one of route `{route}`'s targets; a pin never adds reach"
                ));
            }
            if !class_allows(snap, route, subject) {
                return Err(format!(
                    "route `{route}` does not allow the class of target `{subject}`"
                ));
            }
            Ok(())
        }
    }
}

fn invalid(message: impl Into<String>) -> Response {
    refuse(
        StatusCode::BAD_REQUEST,
        "invalid_request_error",
        "invalid_override",
        message,
    )
}

fn store_of(state: &AppState) -> Option<Arc<dyn OverrideStore>> {
    state.overrides.clone()
}

pub(super) async fn create_override(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<CreateBody>,
) -> Response {
    let snap = state.snapshot();
    let caller = match authenticate(&snap, &headers).await {
        Ok(caller) => caller,
        Err(error) => return error.into_response_for(WireFormat::OpenAiChat),
    };
    if !may_manage(caller.as_ref()) {
        return not_manageable();
    }
    let Some(store) = store_of(&state) else {
        return refuse(
            StatusCode::CONFLICT,
            "invalid_request_error",
            "overrides_need_ledger",
            "overrides need a `[ledger]`: without one nothing would survive a restart",
        );
    };
    let Some(kind) = Kind::parse(&body.kind) else {
        return invalid(format!(
            "unknown kind `{}`; use `disable_target`, `drain_provider` or `pin_route`",
            body.kind
        ));
    };
    if let Err(message) = check_subject(&snap, kind, &body.subject, body.route.as_deref()) {
        return invalid(message);
    }
    let created_by = caller.as_ref().map_or("anonymous", |key| key.id.as_str());
    let request = Create {
        kind,
        subject: body.subject,
        route: body.route,
        reason: body.reason,
        expires_in_secs: body.expires_in_secs,
        created_by: created_by.to_string(),
    };
    match store
        .create(request, snap.config.overrides, state.clock.now_ms())
        .await
    {
        Ok(created) => {
            tracing::info!(
                id = created.id,
                kind = created.kind.as_str(),
                subject = %created.subject,
                by = %created.created_by,
                expires_ms = created.expires_ms,
                "override created"
            );
            (StatusCode::CREATED, Json(override_json(&created, false))).into_response()
        }
        Err(OverrideError::Ledger(error)) => {
            tracing::error!(%error, "could not store an override");
            refuse(
                StatusCode::INTERNAL_SERVER_ERROR,
                "api_error",
                "ledger_error",
                "the override could not be stored",
            )
        }
        Err(error) => invalid(error.to_string()),
    }
}

pub(super) async fn list_overrides(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Response {
    let snap = state.snapshot();
    let caller = match authenticate(&snap, &headers).await {
        Ok(caller) => caller,
        Err(error) => return error.into_response_for(WireFormat::OpenAiChat),
    };
    if !may_list(caller.as_ref()) {
        return not_manageable();
    }
    let overrides: Vec<Value> = state
        .active_overrides()
        .await
        .iter()
        .map(|o| override_json(o, is_stale(&snap, o)))
        .collect();
    Json(json!({"overrides": overrides})).into_response()
}

pub(super) async fn remove_override(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> Response {
    let snap = state.snapshot();
    let caller = match authenticate(&snap, &headers).await {
        Ok(caller) => caller,
        Err(error) => return error.into_response_for(WireFormat::OpenAiChat),
    };
    if !may_manage(caller.as_ref()) {
        return not_manageable();
    }
    let not_found = || {
        refuse(
            StatusCode::NOT_FOUND,
            "not_found_error",
            "override_not_found",
            format!("no override {id} is in force"),
        )
    };
    let Some(store) = store_of(&state) else {
        return not_found();
    };
    let by = caller.as_ref().map_or("anonymous", |key| key.id.as_str());
    match store.revoke(id, by, state.clock.now_ms()).await {
        Ok(true) => {
            tracing::info!(id, by, "override removed");
            Json(json!({"id": id, "removed": true})).into_response()
        }
        Ok(false) => not_found(),
        Err(error) => {
            tracing::error!(%error, "could not remove an override");
            refuse(
                StatusCode::INTERNAL_SERVER_ERROR,
                "api_error",
                "ledger_error",
                "the override could not be removed",
            )
        }
    }
}
