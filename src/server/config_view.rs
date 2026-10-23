//! `GET /v1/config`: the configuration the gateway is serving, with where each part came from (spec: `gateway-api`).
//!
//! The view is built from the loaded `Config` at the moment of the request, so it is exactly what is being served.
//! Secrets are never included: no API key value, no key hash, no provider header value (header names only).
//! Reading it is opt-in per key (`read_config`); a gateway with no keys is open, and so is this.

use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde_json::{Value, json};
use switchyard_protocol::WireFormat;

use super::{AppState, authenticate};
use crate::config::{Config, Endpoint};
use crate::overrides::Override;

/// The effective configuration of `config` as JSON, labelled with its fingerprint and load time.
pub(super) fn view(
    config: &Config,
    fingerprint: &str,
    loaded_at_ms: i64,
    overrides: &[Override],
) -> Value {
    let providers: serde_json::Map<String, Value> = config
        .providers
        .iter()
        .map(|(name, p)| {
            (
                name.clone(),
                json!({
                    "base_url": p.base_url,
                    "has_api_key": !p.api_key.is_empty(),
                    "timeout_secs": p.timeout_secs,
                    "max_retries": p.max_retries,
                    "header_names": p.headers.keys().collect::<Vec<_>>(),
                    "price_source": p.price_source,
                    "quota_source": p.quota_source,
                    "discover": p.discover.is_some(),
                }),
            )
        })
        .collect();
    let targets: serde_json::Map<String, Value> = config
        .targets
        .iter()
        .map(|(name, endpoints)| {
            (
                name.clone(),
                json!({
                    "source": source(config, name, endpoints, overrides),
                    "class": config.target_classes.get(name),
                    "strengths": config.target_strengths.get(name),
                    "quality": config.target_quality.get(name),
                    "endpoints": endpoints,
                }),
            )
        })
        .collect();
    let routes: serde_json::Map<String, Value> = config
        .routes
        .iter()
        .map(|(name, spec)| {
            let mut route = serde_json::to_value(spec).unwrap_or(Value::Null);
            if let Some(map) = route.as_object_mut() {
                map.insert("source".into(), json!("file"));
                if let Some(pool) = config.pools.get(name) {
                    map.insert(
                        "resolved".into(),
                        json!({
                            "members": pool.members,
                            "left_out": pool.left_out.iter().map(|l| json!({"target": l.target, "reason": l.reason})).collect::<Vec<_>>(),
                        }),
                    );
                }
            }
            (name.clone(), route)
        })
        .collect();
    let keys: serde_json::Map<String, Value> = config
        .keys
        .iter()
        .map(|(id, k)| {
            (
                id.clone(),
                json!({
                    "allowed_routes": k.allowed_routes,
                    "over_budget": format!("{:?}", k.over_budget).to_lowercase(),
                    "daily_usd": k.limits.daily_usd,
                    "monthly_usd": k.limits.monthly_usd,
                    "daily_tokens": k.limits.daily_tokens,
                    "monthly_tokens": k.limits.monthly_tokens,
                    "read_config": k.read_config,
                    "source": "file",
                }),
            )
        })
        .collect();
    json!({
        "object": "config",
        "config": {"fingerprint": fingerprint, "loaded_at_ms": loaded_at_ms},
        "settings": {
            "listen": config.listen.to_string(),
            "reload_poll_secs": config.reload_poll_secs,
            "decision_retention_days": config.decision_retention_days,
            "shutdown_grace_secs": config.shutdown_grace_secs,
            "ledger_configured": config.ledger.is_some(),
            "discovery": {
                "enabled": config.discovery.enabled,
                "state": config.discovery_state.as_str(),
                "generated_targets": config.generated_targets.len(),
            },
        },
        "providers": providers,
        "targets": targets,
        "routes": routes,
        "selectors": config.select,
        "keys": keys,
    })
}

/// Where a target came from; `override` while an active runtime override has taken it out of use (disabled, or
/// every endpoint on a drained provider).
fn source(
    config: &Config,
    target: &str,
    endpoints: &[Endpoint],
    overrides: &[Override],
) -> &'static str {
    let providers: Vec<&str> = endpoints.iter().map(|e| e.provider.as_str()).collect();
    if !overrides.is_empty()
        && crate::overrides::removed_by(overrides, "", target, &providers).is_some()
    {
        "override"
    } else if config.generated_targets.contains(target) {
        "generated"
    } else {
        "file"
    }
}

pub(super) async fn get_config(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    let snap = state.snapshot();
    let caller = match authenticate(&snap, &headers).await {
        Ok(caller) => caller,
        Err(error) => return error.into_response_for(WireFormat::OpenAiChat),
    };
    // No keys configured means an open gateway; otherwise only a key that opted in may read the view.
    if caller.as_ref().is_some_and(|key| !key.read_config) {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({"error": {
                "message": "this key may not read the configuration (set `read_config = true` on it)",
                "type": "permission_error",
                "code": "config_not_readable",
            }})),
        )
            .into_response();
    }
    let overrides = state.active_overrides().await;
    Json(view(
        &snap.config,
        &snap.fingerprint,
        snap.loaded_at_ms,
        &overrides,
    ))
    .into_response()
}
