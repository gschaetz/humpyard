//! The routes under `/local` that `humpyard up` adds to the GUI's service (ADR 0039, spec: `single-machine-up`).
//!
//! They sit behind every check the GUI has (a loopback `Host`, no cross-site request, the run's session, a JSON
//! content type for a change, a small body) and add three things: what a person needs to connect a tool, a
//! write-only form for provider keys, and one request that proves the path. No route returns a key, and none
//! logs one.

use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use axum::body::Body;
use axum::http::{HeaderValue, Method, StatusCode, header};
use axum::response::Response;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::Core;
use super::state::{ProviderFacts, provider_facts, route_names};
use crate::keystore::{Secret, Source, source};

pub(super) struct UpLocal {
    core: Arc<Core>,
}

impl UpLocal {
    pub(super) fn new(core: Arc<Core>) -> Self {
        Self { core }
    }
}

fn json_response(status: StatusCode, body: &serde_json::Value) -> Response {
    let mut response = Response::new(Body::from(body.to_string()));
    *response.status_mut() = status;
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

fn refuse(status: StatusCode, code: &str, message: &str) -> Response {
    json_response(
        status,
        &json!({"error": {"code": code, "message": message, "type": "invalid_request_error"}}),
    )
}

fn not_found() -> Response {
    refuse(StatusCode::NOT_FOUND, "not_found", "Not found.")
}

/// A provider the page may set a key for.
struct Provider {
    name: String,
    facts: ProviderFacts,
}

#[derive(Serialize)]
struct KeyStatus {
    provider: String,
    variable: String,
    source: Source,
}

// A key is never echoed, so this has no `Debug`, and a parse error is reported without the input.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NewKey {
    provider: String,
    key: String,
    #[serde(default)]
    save: bool,
}

const MAX_KEY: usize = 1024;

/// What went wrong while a key took effect: the status, a code and words for the page.
type Failure = (StatusCode, &'static str, String);

impl UpLocal {
    /// The providers a key may be set for: the configuration's, or before there is one, the built-in ones and any
    /// `--custom` endpoints.
    fn providers(&self) -> Vec<Provider> {
        if let Ok(text) = std::fs::read_to_string(self.core.config_path()) {
            return provider_facts(&text)
                .into_iter()
                .map(|(name, facts)| Provider { name, facts })
                .collect();
        }
        let mut out: Vec<Provider> = crate::init::PRESETS
            .iter()
            .map(|p| Provider {
                name: p.name.to_string(),
                facts: ProviderFacts {
                    variable: p.key_env.to_string(),
                    base_url: Some(p.base_url.to_string()),
                    model: p.fast.first().map(|m| (*m).to_string()),
                },
            })
            .collect();
        for text in &self.core.custom {
            if let Ok(pick) = crate::init::parse_custom(text) {
                out.push(Provider {
                    name: pick.name,
                    facts: ProviderFacts {
                        variable: pick.key_env,
                        base_url: Some(pick.base_url),
                        model: pick.fast,
                    },
                });
            }
        }
        out
    }

    fn statuses(&self) -> Result<Vec<KeyStatus>, String> {
        let saved = self.core.store.load().map_err(|e| e.to_string())?;
        let session = self.core.session.lock().clone();
        Ok(self
            .providers()
            .into_iter()
            .map(|p| KeyStatus {
                source: source(&p.facts.variable, &saved, &session, &*self.core.env),
                variable: p.facts.variable,
                provider: p.name,
            })
            .collect())
    }

    async fn connect(&self) -> Response {
        let text = std::fs::read_to_string(self.core.config_path()).unwrap_or_default();
        json_response(
            StatusCode::OK,
            &json!({
                "gateway": format!("http://{}", self.core.gateway.addr),
                "routes": route_names(&text),
                "has_config": self.core.has_config(),
                "gateway_running": self.core.gateway.running().await,
                "key_entry": self.core.key_entry,
            }),
        )
    }

    fn keys(&self) -> Response {
        match self.statuses() {
            Ok(providers) => json_response(
                StatusCode::OK,
                &json!({"key_entry": self.core.key_entry, "providers": providers}),
            ),
            Err(message) => refuse(
                StatusCode::INTERNAL_SERVER_ERROR,
                "keys_unreadable",
                &message,
            ),
        }
    }

    fn key_entry_disabled(&self) -> Option<Response> {
        (!self.core.key_entry).then(|| {
            refuse(
                StatusCode::FORBIDDEN,
                "key_entry_disabled",
                "Keys cannot be entered here because this page is served beyond this machine.",
            )
        })
    }

    /// Adds or replaces a key: checks it live, holds it (and saves it when asked), and starts or restarts the
    /// gateway on it. A failure at any step leaves the keys, the file and the gateway as they were.
    async fn set_key(&self, body: &[u8]) -> Response {
        if let Some(refusal) = self.key_entry_disabled() {
            return refusal;
        }
        let Ok(new) = serde_json::from_slice::<NewKey>(body) else {
            return refuse(
                StatusCode::BAD_REQUEST,
                "bad_request",
                "Send {\"provider\": \"...\", \"key\": \"...\", \"save\": false}.",
            );
        };
        let key = new.key.trim().to_string();
        if key.is_empty() || key.len() > MAX_KEY || key.chars().any(char::is_control) {
            return refuse(
                StatusCode::BAD_REQUEST,
                "bad_key",
                "That is not a usable key.",
            );
        }
        let providers = self.providers();
        let Some(provider) = providers.iter().find(|p| p.name == new.provider) else {
            return refuse(
                StatusCode::BAD_REQUEST,
                "unknown_provider",
                "No such provider is configured.",
            );
        };
        // Before there is a configuration, writing one checks every provider with a key live (the `init` flow).
        if self.core.has_config()
            && let Some(base_url) = &provider.facts.base_url
            && let Err(why) =
                crate::init::check_key(base_url, &key, provider.facts.model.as_deref()).await
        {
            return refuse(
                StatusCode::UNPROCESSABLE_ENTITY,
                "provider_rejected_key",
                &format!("{} did not accept this key: {why}", provider.name),
            );
        }
        self.apply_key(&provider.facts.variable, &key, new.save)
            .await
    }

    async fn apply_key(&self, variable: &str, key: &str, save: bool) -> Response {
        let core = &self.core;
        let Ok(previous_file) = core.store.load() else {
            return refuse(
                StatusCode::INTERNAL_SERVER_ERROR,
                "keys_unreadable",
                "The saved keys could not be read.",
            );
        };
        let previous_session = core.session.lock().clone();
        core.session
            .lock()
            .insert(variable.to_string(), Secret::new(key));
        match self.take_effect(variable, key, save).await {
            Ok(note) => json_response(StatusCode::OK, &json!({"ok": true, "note": note})),
            Err((status, error_code, message)) => {
                // Put everything back as it was, and bring the old gateway back if there was one.
                *core.session.lock() = previous_session;
                let _ = core.store.save(&previous_file);
                if core.has_config() && core.missing_keys().is_ok_and(|m| m.is_empty()) {
                    let _ = core.restart_gateway().await;
                }
                refuse(status, error_code, &message)
            }
        }
    }

    async fn take_effect(&self, variable: &str, key: &str, save: bool) -> Result<String, Failure> {
        let core = &self.core;
        if save {
            core.store.set(variable, key).map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "key_not_saved",
                    e.to_string(),
                )
            })?;
        }
        if !core.has_config() {
            // `init::run` is not `Send` (it takes plain `dyn Write` and `dyn BufRead`), so it runs on a blocking
            // thread of its own that drives it to the end.
            let first = Arc::clone(core);
            let runtime = tokio::runtime::Handle::current();
            let secret = key.to_string();
            let (written, progress) = tokio::task::spawn_blocking(move || {
                let mut progress = Vec::new();
                let written = runtime.block_on(super::first_config(&first, &mut progress));
                (
                    written,
                    String::from_utf8_lossy(&progress).replace(&secret, "<key>"),
                )
            })
            .await
            .map_err(|_| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "setup_failed",
                    "The first configuration could not be written.".to_string(),
                )
            })?;
            // What was checked, provider by provider, is what tells a person why a key was not accepted.
            let detail = |why: &str| format!("{why}\n{}", progress.trim());
            match written {
                Ok(true) => {}
                Ok(false) => {
                    return Err((
                        StatusCode::UNPROCESSABLE_ENTITY,
                        "provider_rejected_key",
                        detail("No provider accepted that key."),
                    ));
                }
                Err(e) => {
                    return Err((
                        StatusCode::UNPROCESSABLE_ENTITY,
                        "provider_rejected_key",
                        detail(&e),
                    ));
                }
            }
        }
        if core.missing_keys().is_ok_and(|m| !m.is_empty()) {
            // Other providers still have no key, and the gateway cannot load without it.
            core.gateway.stop().await;
            return Ok("saved; the gateway starts when every provider has a key".into());
        }
        core.restart_gateway()
            .await
            .map_err(|e| (StatusCode::BAD_GATEWAY, "gateway_restart_failed", e))?;
        Ok(if save {
            "saved on this machine; the gateway restarted".into()
        } else {
            "held for this run only; the gateway restarted".into()
        })
    }

    /// Removes a provider's key from the file and from this run, and restarts the gateway if it can still load.
    async fn remove_key(&self, provider: &str) -> Response {
        if let Some(refusal) = self.key_entry_disabled() {
            return refusal;
        }
        let Some(variable) = self
            .providers()
            .iter()
            .find(|p| p.name == provider)
            .map(|p| p.facts.variable.clone())
        else {
            return refuse(
                StatusCode::BAD_REQUEST,
                "unknown_provider",
                "No such provider is configured.",
            );
        };
        if let Err(e) = self.core.store.remove(&variable) {
            return refuse(
                StatusCode::INTERNAL_SERVER_ERROR,
                "key_not_removed",
                &e.to_string(),
            );
        }
        self.core.session.lock().remove(&variable);
        self.core.gateway.stop().await;
        let note = if !self.core.has_config() {
            "removed".to_string()
        } else if self.core.missing_keys().is_ok_and(|m| m.is_empty()) {
            match self.core.start_gateway().await {
                Ok(()) => "removed; the gateway restarted".to_string(),
                Err(e) => format!("removed; the gateway did not start: {e}"),
            }
        } else {
            "removed; the gateway is stopped until every provider has a key".to_string()
        };
        json_response(StatusCode::OK, &json!({"ok": true, "note": note}))
    }

    /// One small fixed request through the running gateway, so the page can say what answered and how fast.
    async fn test(&self) -> Response {
        json_response(StatusCode::OK, &self.run_test().await)
    }

    async fn run_test(&self) -> serde_json::Value {
        if !self.core.gateway.running().await {
            return json!({"ok": false, "step": "gateway", "message":
                "The gateway is not running. Add a key for every provider first."});
        }
        let text = std::fs::read_to_string(self.core.config_path()).unwrap_or_default();
        let routes = route_names(&text);
        let Some(model) = routes
            .iter()
            .find(|r| r.as_str() == "fast")
            .or_else(|| routes.first())
        else {
            return json!({"ok": false, "step": "configuration", "message":
                "The configuration has no route or target to ask."});
        };
        let Ok(client) = reqwest::Client::builder()
            .timeout(Duration::from_secs(60))
            .redirect(reqwest::redirect::Policy::none())
            .build()
        else {
            return json!({"ok": false, "step": "gateway", "message": "A test request could not be made."});
        };
        let started = Instant::now();
        let sent = client
            .post(format!(
                "http://{}/v1/chat/completions",
                self.core.gateway.addr
            ))
            .json(&json!({
                "model": model,
                "max_tokens": 200,
                "messages": [{"role": "user", "content": "Say \"ok\"."}],
            }))
            .send()
            .await;
        let elapsed = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        let response = match sent {
            Ok(response) => response,
            Err(e) => {
                let why = if e.is_timeout() {
                    "The gateway did not answer within 60 seconds."
                } else {
                    "The gateway could not be reached."
                };
                return json!({"ok": false, "step": "gateway", "message": why});
            }
        };
        let status = response.status();
        if status.is_success() {
            let target = response
                .headers()
                .get("x-humpyard-target")
                .and_then(|v| v.to_str().ok())
                .unwrap_or(model)
                .to_string();
            return json!({"ok": true, "model": model, "target": target, "latency_ms": elapsed});
        }
        let message = match status.as_u16() {
            401 | 403 => "The provider turned the request away. Check its key.".to_string(),
            429 => "The provider is limiting requests. Try again in a moment.".to_string(),
            code => format!("No provider could answer (the gateway said {code})."),
        };
        json!({"ok": false, "step": "provider", "message": message, "status": status.as_u16()})
    }
}

#[async_trait]
impl crate::ui::Local for UpLocal {
    async fn handle(&self, method: &Method, segments: &[String], body: &[u8]) -> Response {
        let parts: Vec<&str> = segments.iter().map(String::as_str).collect();
        match (method, parts.as_slice()) {
            (&Method::GET, ["connect"]) => self.connect().await,
            (&Method::GET, ["keys"]) => self.keys(),
            (&Method::POST, ["keys"]) => self.set_key(body).await,
            (&Method::DELETE, ["keys", provider]) => self.remove_key(provider).await,
            (&Method::POST, ["test"]) => self.test().await,
            (_, ["connect" | "keys" | "test"] | ["keys", _]) => refuse(
                StatusCode::METHOD_NOT_ALLOWED,
                "method_not_allowed",
                "This is not asked this way.",
            ),
            _ => not_found(),
        }
    }
}
