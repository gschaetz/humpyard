//! `GET /metrics`: the Prometheus view of the metrics registry (ADR 0016), plus the values that
//! are read from their owners at scrape time (breaker states, ledger backlog, quota, build).

use std::sync::Arc;

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};

use super::{AppState, Snapshot};
use crate::auth::{hash_key, presented_key};
use crate::metrics::{self, Family, Value};
use crate::quota::{View, identity};

const CONTENT_TYPE: &str = "text/plain; version=0.0.4; charset=utf-8";

/// Serves the registry. The credential, when configured, is the only thing that opens this path:
/// a virtual key never does, and this credential opens nothing else.
pub(super) async fn get_metrics(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Response {
    let snap = state.snapshot();
    if let Some(expected) = snap.config.metrics.credential_sha256 {
        let presented = presented_key(&headers).map(hash_key);
        if presented != Some(expected) {
            return (
                StatusCode::UNAUTHORIZED,
                [(header::WWW_AUTHENTICATE, "Bearer")],
                "a metrics credential is required\n",
            )
                .into_response();
        }
    }
    let body = state.metrics.render(scraped(&state, &snap));
    ([(header::CONTENT_TYPE, CONTENT_TYPE)], body).into_response()
}

/// Values the registry does not own, read now from the breakers, the ledger and the quota store.
fn scraped(state: &AppState, snap: &Snapshot) -> Vec<Family> {
    let mut families = vec![
        metrics::gauge(
            "build_info",
            "The running version; always 1.",
            vec![(
                vec![("version", env!("CARGO_PKG_VERSION").to_string())],
                Value::Int(1),
            )],
        ),
        metrics::gauge(
            "config_info",
            "The fingerprint of the serving configuration; always 1.",
            vec![(
                vec![("fingerprint", snap.fingerprint.clone())],
                Value::Int(1),
            )],
        ),
        metrics::gauge(
            "config_loaded_timestamp_seconds",
            "When the serving configuration was loaded, in Unix seconds.",
            vec![(
                vec![],
                Value::Int(u64::try_from(snap.loaded_at_ms / 1000).unwrap_or(0)),
            )],
        ),
        metrics::gauge(
            "config_last_reload_ok",
            "1 if the latest reload attempt was accepted (or none was made), 0 if it was rejected.",
            vec![(
                vec![],
                Value::Int(u64::from(state.reload.lock().ok != Some(false))),
            )],
        ),
        endpoint_state(snap),
        target_latency(state, snap),
    ];
    if let Some(ledger) = state.accounting.ledger() {
        families.push(metrics::gauge(
            "ledger_queue_depth",
            "Messages waiting for the ledger writer.",
            vec![(
                vec![],
                Value::Int(u64::try_from(ledger.queue_depth()).unwrap_or(u64::MAX)),
            )],
        ));
        families.push(metrics::counter(
            "ledger_dropped_total",
            "Ledger entries dropped because the writer's queue was full.",
            vec![(vec![], Value::Int(ledger.dropped()))],
        ));
        families.push(metrics::counter(
            "ledger_failed_total",
            "Ledger entries the writer failed to store.",
            vec![(vec![], Value::Int(ledger.failed()))],
        ));
    }
    if let Some(store) = &state.overrides {
        families.extend(override_gauges(
            &store.active(state.clock.now_ms()),
            state.clock.now_ms(),
        ));
    }
    families.extend(quota(state, snap));
    families.extend(discovery(snap));
    families
}

/// How many runtime overrides are in force by kind, and how old the oldest is. The kinds are a fixed
/// vocabulary, so every one is exported, at 0 when none is active.
fn override_gauges(active: &[crate::overrides::Override], now_ms: i64) -> Vec<Family> {
    use crate::overrides::Kind;
    let by_kind = [Kind::DisableTarget, Kind::DrainProvider, Kind::PinRoute]
        .into_iter()
        .map(|kind| {
            let count = active.iter().filter(|o| o.kind == kind).count();
            (
                vec![("kind", kind.as_str().to_string())],
                Value::Int(u64::try_from(count).unwrap_or(u64::MAX)),
            )
        })
        .collect();
    let oldest_ms = active
        .iter()
        .map(|o| now_ms.saturating_sub(o.created_ms))
        .max()
        .unwrap_or(0);
    vec![
        metrics::gauge(
            "overrides_active",
            "Runtime overrides in force, by kind.",
            by_kind,
        ),
        metrics::gauge(
            "override_oldest_age_seconds",
            "How long the oldest runtime override in force has been active; 0 when none is.",
            vec![(vec![], Value::Int(crate::num::from_i64(oldest_ms / 1000)))],
        ),
    ]
}

/// The generated targets file (ADR 0022): how many targets it merged and what became of it. Present only
/// when a file is configured.
fn discovery(snap: &Snapshot) -> Vec<Family> {
    const STATES: [&str; 5] = ["disabled", "missing", "empty", "loaded", "ignored"];
    if snap.config.discovery.file.is_none() {
        return Vec::new();
    }
    let current = snap.config.discovery_state.as_str();
    vec![
        metrics::gauge(
            "discovery_targets",
            "Generated targets merged from the discovery file at the last load.",
            vec![(
                vec![],
                Value::Int(u64::try_from(snap.config.discovery_state.loaded()).unwrap_or(u64::MAX)),
            )],
        ),
        metrics::gauge(
            "discovery_file_state",
            "State of the generated targets file at the last load: 1 for the state it is in (an ignored file means the main configuration is serving alone).",
            STATES
                .iter()
                .map(|state| {
                    (
                        vec![("state", (*state).to_string())],
                        Value::Int(u64::from(*state == current)),
                    )
                })
                .collect(),
        ),
    ]
}

/// The smoothed time to answer of each configured target that has answered: what `fastest` routes
/// order by (ADR 0020). Only configured target names are labels.
fn target_latency(state: &AppState, snap: &Snapshot) -> Family {
    let mut targets: Vec<_> = snap.config.targets.keys().collect();
    targets.sort();
    let samples = targets
        .into_iter()
        .filter_map(|target| {
            let figure = state.accounting.latency().figure(target)?;
            Some((
                vec![("target", target.clone())],
                Value::Float(figure.ms / 1000.0),
            ))
        })
        .collect();
    metrics::gauge(
        "target_latency_seconds",
        "Smoothed time to answer of each target, as latency-ordered routes see it; absent until the target has answered.",
        samples,
    )
}

/// One series per endpoint and state, 1 for the state it is in (the Prometheus idiom for enums).
fn endpoint_state(snap: &Snapshot) -> Family {
    const STATES: [&str; 3] = ["healthy", "cooling_down", "probing"];
    let mut targets: Vec<_> = snap.targets.iter().collect();
    targets.sort_by(|a, b| a.0.cmp(b.0));
    let mut samples = Vec::new();
    for (target, client) in targets {
        for endpoint in client.health() {
            let current = endpoint.snapshot.state.as_str();
            for state in STATES {
                samples.push((
                    vec![
                        ("target", target.to_string()),
                        ("provider", endpoint.provider.clone()),
                        ("model", endpoint.model.clone()),
                        ("state", state.to_string()),
                    ],
                    Value::Int(u64::from(state == current)),
                ));
            }
        }
    }
    metrics::gauge(
        "endpoint_state",
        "Circuit breaker state of each endpoint: 1 for the state it is in.",
        samples,
    )
}

/// Percent used per window and the age of the snapshot, for providers whose quota is fresh.
fn quota(state: &AppState, snap: &Snapshot) -> Vec<Family> {
    let config = &snap.config;
    let now = state.clock.now_ms();
    let stale_ms = i64::try_from(config.quota_stale_secs.saturating_mul(1000)).unwrap_or(i64::MAX);
    let (mut percent, mut age) = (Vec::new(), Vec::new());
    for (name, provider) in &config.providers {
        let Some(source) = provider.quota_source.as_deref() else {
            continue;
        };
        let view = state
            .quota
            .view(name, &identity(source, &provider.base_url), now, stale_ms);
        if let View::Fresh {
            windows, age_ms, ..
        } = view
        {
            for window in windows {
                percent.push((
                    vec![("provider", name.clone()), ("window", window.name.clone())],
                    Value::Float(window.percent),
                ));
            }
            age.push((
                vec![("provider", name.clone())],
                Value::Int(u64::try_from(age_ms / 1000).unwrap_or(0)),
            ));
        }
    }
    vec![
        metrics::gauge(
            "quota_percent",
            "Percent of a provider's plan window used, as the provider reports it.",
            percent,
        ),
        metrics::gauge(
            "quota_snapshot_age_seconds",
            "Age of the latest fresh quota reading, per provider.",
            age,
        ),
    ]
}
