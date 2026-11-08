//! `POST /admin/v1/config/patch` (ADR 0034): typed edits become a validated, reviewable patch against the
//! configuration file, and nothing is written.
//!
//! The order matters. The caller is authenticated and holds the `config` scope before the body is read; the body
//! is bounded before it is parsed; the fingerprint is checked against the file as it is now before any edit is
//! applied; and the result must pass the check a reload makes (plus a throwaway build of the serving state)
//! before a diff is offered. Every outcome leaves the file, the ledger, the reload status and the running
//! configuration as they were.

use std::sync::Arc;

use axum::Json;
use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::{Value, json};

use super::admin::authorize;
use super::{AppState, Snapshot, validate_patched, watched_fingerprint};
use crate::config::AdminScope;
use crate::config::patch::{Edit, MAX_EDITS, PatchError, apply, masked_diff};

/// The largest request body, read only after the caller is known.
const MAX_BODY_BYTES: usize = 64 * 1024;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    /// The fingerprint of the file the editor last read.
    fingerprint: String,
    edits: Vec<Value>,
}

/// Produces the patch for the edits in the request body. Public to `server::admin`, which owns the route.
pub(super) async fn patch(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: Body,
) -> Response {
    let principal = match authorize(&state, &headers, Some(AdminScope::Config)) {
        Ok(principal) => principal,
        Err(denied) => return denied.into_response(),
    };
    let (label, response) = match produce(&state, body).await {
        Ok(answer) => (answer.label, Json(answer.body).into_response()),
        Err(refusal) => (refusal.label, refusal.into_response()),
    };
    // Only who asked and how it ended: never the edits' values or any of the file.
    tracing::info!(admin = %principal.id, outcome = label, "configuration patch requested");
    response
}

/// A patch that was produced.
struct Answer {
    label: &'static str,
    body: Value,
}

/// Why no patch was produced: the status and error body to send, and a bounded label for the log.
struct Refusal {
    label: &'static str,
    status: StatusCode,
    code: &'static str,
    message: String,
    /// One field the caller can act on (`index`, `current_fingerprint`).
    extra: Option<(&'static str, Value)>,
}

type Refused = Box<Refusal>;

impl Refusal {
    fn new(
        label: &'static str,
        status: StatusCode,
        code: &'static str,
        message: String,
    ) -> Refused {
        Box::new(Self {
            label,
            status,
            code,
            message,
            extra: None,
        })
    }

    fn with(mut self: Box<Self>, name: &'static str, value: Value) -> Refused {
        self.extra = Some((name, value));
        self
    }

    fn into_response(self) -> Response {
        let mut error = json!({
            "message": self.message,
            "type": if self.status.is_server_error() { "api_error" } else { "invalid_request_error" },
            "code": self.code,
        });
        if let (Some((name, value)), Some(map)) = (self.extra, error.as_object_mut()) {
            map.insert(name.to_string(), value);
        }
        (self.status, Json(json!({ "error": error }))).into_response()
    }
}

async fn produce(state: &AppState, body: Body) -> Result<Answer, Refused> {
    let request = read_request(body).await?;
    let edits = typed(request.edits)?;
    let path = state
        .config_path
        .as_deref()
        .ok_or_else(|| unavailable("the gateway has no configuration file to edit"))?;
    let text = tokio::fs::read_to_string(path)
        .await
        .map_err(|e| unavailable(&e.to_string()))?;
    let running = state.snapshot();
    let current = watched_fingerprint(&text, running.config.discovery.file.as_deref());
    if current != request.fingerprint {
        return Err(stale(&current));
    }
    let patched = apply(&text, &edits).map_err(|e| match e {
        PatchError::Invalid { index, reason } => invalid_edit(index, &reason),
        PatchError::TooManyEdits => too_many(),
        PatchError::Unparsable(why) => unavailable(&why),
    })?;
    let config = validate_patched(&running.config, &patched).map_err(|why| rejected(&why))?;
    let fingerprint = watched_fingerprint(&patched, config.discovery.file.as_deref());
    // A reload also builds the serving state, which can fail. Build one the same way with nothing carried over from
    // the live one (its limiter and health state must not be shared with a copy that is thrown away).
    Snapshot::build(
        config,
        &state.clock,
        &state.extra_policy,
        &state.drains,
        None,
        fingerprint.clone(),
    )
    .map_err(|why| rejected(&why))?;
    let diff = masked_diff(&text, &patched).map_err(|e| unavailable(&e.to_string()))?;
    let changed = patched != text;
    Ok(Answer {
        label: if changed { "ok" } else { "unchanged" },
        body: json!({
            "object": "config_patch",
            "changed": changed,
            "base_fingerprint": current,
            "fingerprint": fingerprint,
            "edits": edits.len(),
            "diff": diff,
        }),
    })
}

async fn read_request(body: Body) -> Result<Request, Refused> {
    let bytes = axum::body::to_bytes(body, MAX_BODY_BYTES)
        .await
        .map_err(|_| {
            Refusal::new(
                "too_large",
                StatusCode::PAYLOAD_TOO_LARGE,
                "request_too_large",
                format!("the request body is larger than {MAX_BODY_BYTES} bytes"),
            )
        })?;
    let request: Request = serde_json::from_slice(&bytes).map_err(|e| {
        Refusal::new(
            "bad_request",
            StatusCode::BAD_REQUEST,
            "invalid_request",
            format!("the body must be {{\"fingerprint\": \"...\", \"edits\": [...]}}: {e}"),
        )
    })?;
    if request.edits.len() > MAX_EDITS {
        return Err(too_many());
    }
    Ok(request)
}

/// Each edit, checked for its shape; the first that is not one is named by position.
fn typed(edits: Vec<Value>) -> Result<Vec<Edit>, Refused> {
    edits
        .into_iter()
        .enumerate()
        .map(|(index, edit)| {
            serde_json::from_value::<Edit>(edit).map_err(|e| invalid_edit(index, &e.to_string()))
        })
        .collect()
}

fn too_many() -> Refused {
    Refusal::new(
        "bad_request",
        StatusCode::BAD_REQUEST,
        "invalid_request",
        format!("at most {MAX_EDITS} edits in one request"),
    )
}

fn invalid_edit(index: usize, reason: &str) -> Refused {
    Refusal::new(
        "invalid_edit",
        StatusCode::BAD_REQUEST,
        "invalid_edit",
        format!("edit {index}: {reason}"),
    )
    .with("index", json!(index))
}

fn stale(current: &str) -> Refused {
    Refusal::new(
        "stale",
        StatusCode::CONFLICT,
        "stale_fingerprint",
        "the configuration file has changed since the fingerprint you sent; read it again".into(),
    )
    .with("current_fingerprint", json!(current))
}

fn rejected(why: &str) -> Refused {
    Refusal::new(
        "rejected",
        StatusCode::UNPROCESSABLE_ENTITY,
        "config_rejected",
        format!("the configuration would not load: {why}"),
    )
}

fn unavailable(why: &str) -> Refused {
    Refusal::new(
        "unavailable",
        StatusCode::SERVICE_UNAVAILABLE,
        "config_file_unavailable",
        why.to_string(),
    )
}
