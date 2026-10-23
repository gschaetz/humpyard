//! HTTP surface: inference endpoints in three client protocols, served through Switchyard's
//! `run` over the provider pool.

use std::collections::HashMap;
use std::convert::Infallible;
use std::path::PathBuf;
use std::sync::Arc;

use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, HeaderName, HeaderValue, StatusCode};
use axum::response::sse::{Event, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use futures::{Stream, StreamExt};
use parking_lot::{Mutex, RwLock};
use serde_json::{Value, json};
use std::time::Duration;
use tokio::sync::{mpsc, watch};

use switchyard_llm_client::run as run_algorithm;
use switchyard_protocol::{
    Category, LlmRequest, LlmResponse, Metadata, ModelId, Request, WireFormat,
};
use switchyard_translation::{
    LlmStreamError, RawEventStream, decode_request, encode_aggregated_response_with_extensions,
    encode_request, encode_stream_with_extensions, util::SWITCHYARD_METADATA_KEY,
};

use crate::auth::{ConfigKeyStore, KeyRecord, KeyStore, hash_key, presented_key};
use crate::budget::{BudgetPolicy, BudgetTracker};
use crate::clock::Clock;
use crate::clock::SystemClock;
use crate::config::CREDENTIAL_HEADERS;
use crate::config::Config;
use crate::config::OverBudget;
use crate::error::GatewayError;
use crate::estimate::estimate_tokens;
use crate::latency::Latency;
use crate::ledger::Incapable;
use crate::ledger::{Decision, Ledger};
use crate::metering::{Accounting, CALL_ID_HEADER, CallContext, StreamTemplate};
use crate::num::f64_from_u64;
use crate::overrides::{Drains, LedgerOverrides, NoDrains, Override, OverrideStore, StoreDrains};
use crate::policy::{
    All, AllowAll, BudgetState, KeyContext, PolicyContext, RequestMeta, RoutingPolicy,
};
use crate::pool::{self, PROVIDER_HEADER, TargetClient};
use crate::routing::{Plan, Route, Routes};
use crate::select::{Facts, Features, Selectors};

const TARGET_HEADER: &str = "x-humpyard-target";
const TOKEN_COUNT_HEADER: &str = "x-humpyard-token-count";
/// Response headers naming the route that served a request and the selector rule that chose it.
mod config_view;
mod decisions;
mod explain;
mod metrics;
mod overrides;
mod quota;
mod sessions;
mod stats;

const ROUTE_HEADER: &str = "x-humpyard-route";
const RULE_HEADER: &str = "x-humpyard-rule";
/// Response header carrying the request's id, which also marks its usage rows and its decision.
const REQUEST_ID_HEADER: &str = "x-humpyard-request-id";

/// Everything derived from the config file. It is replaced whole on a reload (ADR 0012): a request
/// takes the current snapshot once and uses it to the end, so rules never change under it.
struct Snapshot {
    config: Config,
    targets: HashMap<ModelId, Arc<TargetClient>>,
    routes: Routes,
    selectors: Selectors,
    policy: Arc<dyn RoutingPolicy>,
    keys: Arc<dyn KeyStore>,
    /// Short hash of the config file this snapshot came from, for `GET /v1/health`.
    fingerprint: String,
    loaded_at_ms: i64,
}

pub struct AppState {
    /// The current configuration; swapped by a reload.
    snapshot: RwLock<Arc<Snapshot>>,
    /// Flips to `true` when shutdown gives up waiting: running requests are cancelled.
    hard_stop: watch::Receiver<bool>,
    accounting: Arc<Accounting>,
    tracker: Option<Arc<BudgetTracker>>,
    clock: Arc<dyn Clock>,
    /// The latest provider quota snapshots (kept across reloads).
    quota: Arc<crate::quota::QuotaStore>,
    /// Runtime overrides, kept across reloads; none without a ledger, because nothing would survive a restart.
    overrides: Option<Arc<dyn OverrideStore>>,
    /// The providers those overrides drain, read by every target's client; kept across reloads.
    drains: Arc<dyn Drains>,
    /// Operational metrics, served at `/metrics` when enabled (ADR 0016).
    metrics: Arc<crate::metrics::Metrics>,
    /// The embedder's policy; it is composed with the budget policy of every snapshot.
    extra_policy: Arc<dyn RoutingPolicy>,
    /// The outcome of the latest reload attempt.
    reload: Mutex<ReloadStatus>,
    /// Serializes reloads.
    reloading: tokio::sync::Mutex<()>,
}

impl AppState {
    fn snapshot(&self) -> Arc<Snapshot> {
        self.snapshot.read().clone()
    }

    /// The overrides in force now. Notices any that have run out first (writing their audit row), which costs
    /// nothing until one has.
    async fn active_overrides(&self) -> Vec<Override> {
        let Some(store) = &self.overrides else {
            return Vec::new();
        };
        let now_ms = self.clock.now_ms();
        if let Err(error) = store.sweep(now_ms).await {
            tracing::warn!(%error, "could not record an expired override");
        }
        store.active(now_ms)
    }
}

/// What the last reload attempt did, for `GET /v1/health`.
#[derive(Clone, Default)]
struct ReloadStatus {
    /// How many reloads have been attempted since start (successful or not).
    attempts: u64,
    at_ms: Option<i64>,
    ok: Option<bool>,
    message: Option<String>,
}

pub async fn router(config: Config) -> Result<Router, String> {
    router_with_options(config, Options::default()).await
}

/// Like [`router`], with a custom policy deciding which targets are eligible per request. The
/// budget policy always applies in addition.
pub async fn router_with_policy(
    config: Config,
    policy: Arc<dyn RoutingPolicy>,
) -> Result<Router, String> {
    router_with_options(
        config,
        Options {
            policy: Some(policy),
            ..Options::default()
        },
    )
    .await
}

/// Embedding hooks: an extra routing policy and a clock (tests control budget periods with it).
#[derive(Default)]
pub struct Options {
    pub policy: Option<Arc<dyn RoutingPolicy>>,
    pub clock: Option<Arc<dyn Clock>>,
}

pub async fn router_with_options(config: Config, options: Options) -> Result<Router, String> {
    Ok(build(config, options, "startup".to_string()).await?.router)
}

/// How long, after giving up on in-flight requests, to wait for their connections to close.
const HARD_STOP_WAIT: Duration = Duration::from_secs(2);
/// How long to wait for queued usage entries to reach the ledger before exiting.
const FLUSH_TIMEOUT: Duration = Duration::from_secs(10);

/// Serves on `listener` until the process is asked to stop, then shuts down cleanly.
///
/// The first message on `signals` starts a drain: no new connections, in-flight requests may
/// finish for up to `shutdown_grace_secs`. A second message, or the end of the grace period,
/// cancels whatever is still running (streams end with an error event). Either way every usage
/// entry is flushed to the ledger before this returns, and a failed flush is an error.
///
/// # Errors
/// If the router cannot be built, the server fails, or the ledger cannot be flushed in time.
pub async fn run(
    config: Config,
    listener: tokio::net::TcpListener,
    mut signals: mpsc::Receiver<()>,
    reload: Option<Reload>,
) -> Result<(), String> {
    let grace = Duration::from_secs(config.shutdown_grace_secs);
    let poll_secs = config.reload_poll_secs;
    let initial_fingerprint = reload
        .as_ref()
        .and_then(|r| std::fs::read_to_string(&r.path).ok())
        .map_or_else(
            || "startup".to_string(),
            |text| watched_fingerprint(&text, config.discovery.file.as_deref()),
        );
    log_discovery(None, &config);
    let built = build(config, Options::default(), initial_fingerprint).await?;
    let poll = (poll_secs > 0).then(|| Duration::from_secs(poll_secs));
    let reloader = reload.map(|r| tokio::spawn(reload_loop(built.state.clone(), r, poll)));
    let metrics_listener = spawn_metrics_listener(&built.state).await?;
    let (drain_tx, drain_rx) = tokio::sync::oneshot::channel::<()>();
    let server = axum::serve(listener, built.router)
        .with_graceful_shutdown(async move {
            let _ = drain_rx.await;
        })
        .into_future();
    tokio::pin!(server);

    let mut cancelled = false;
    tokio::select! {
        result = &mut server => result.map_err(|e| e.to_string())?,
        Some(()) = signals.recv() => {
            tracing::info!("shutdown requested; draining in-flight requests");
            let _ = drain_tx.send(());
            tokio::select! {
                result = &mut server => result.map_err(|e| e.to_string())?,
                Some(()) = signals.recv() => {
                    tracing::warn!("second shutdown signal; cancelling remaining requests");
                    cancelled = true;
                }
                () = tokio::time::sleep(grace) => {
                    tracing::warn!(grace_secs = grace.as_secs(), "grace period over; cancelling remaining requests");
                    cancelled = true;
                }
            }
        }
    }
    if let Some(reloader) = reloader {
        reloader.abort();
    }
    if let Some(task) = metrics_listener {
        task.abort();
    }
    if cancelled {
        let _ = built.hard_stop.send(true);
        if tokio::time::timeout(HARD_STOP_WAIT, &mut server)
            .await
            .is_err()
        {
            tracing::warn!("connections did not close in time");
        }
    }

    if tokio::time::timeout(FLUSH_TIMEOUT, built.accounting.flush())
        .await
        .is_err()
    {
        let (failed, dropped) = built
            .accounting
            .ledger()
            .map_or((0, 0), |l| (l.failed(), l.dropped()));
        tracing::error!(
            failed,
            dropped,
            "ledger flush timed out; usage entries may be lost"
        );
        return Err("ledger flush timed out; usage entries may be lost".into());
    }
    if let Some(ledger) = built.accounting.ledger() {
        let (failed, dropped) = (ledger.failed(), ledger.dropped());
        if failed + dropped > 0 {
            tracing::warn!(failed, dropped, "some usage entries were not written");
        }
    }
    tracing::info!("shutdown complete");
    Ok(())
}

/// Where a running gateway re-reads its config from, and what asks it to.
pub struct Reload {
    /// The config file to re-read.
    pub path: PathBuf,
    /// Each message asks for one reload (the binary sends one per SIGHUP).
    pub triggers: mpsc::Receiver<()>,
}

/// Reloads on every trigger, and, when `poll` is set, whenever the file's content differs from the
/// last version looked at (a bad file is attempted once, not on every tick). Triggers that arrive
/// while a reload runs are merged into one more.
async fn reload_loop(state: Arc<AppState>, mut reload: Reload, poll: Option<Duration>) {
    let mut ticker = poll.map(|every| {
        let mut ticker = tokio::time::interval(every);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        ticker
    });
    let mut last_seen = state.snapshot().fingerprint.clone();
    loop {
        let signalled = if let Some(ticker) = ticker.as_mut() {
            tokio::select! {
                trigger = reload.triggers.recv() => {
                    if trigger.is_none() { return; }
                    true
                }
                _ = ticker.tick() => false,
            }
        } else {
            if reload.triggers.recv().await.is_none() {
                return;
            }
            true
        };
        if !signalled {
            let Ok(text) = std::fs::read_to_string(&reload.path) else {
                continue;
            };
            let generated = state.snapshot().config.discovery.file.clone();
            let seen = watched_fingerprint(&text, generated.as_deref());
            if seen == last_seen {
                continue;
            }
            last_seen = seen;
        }
        loop {
            reload_config(&state, &reload.path).await;
            if let Ok(text) = std::fs::read_to_string(&reload.path) {
                let generated = state.snapshot().config.discovery.file.clone();
                last_seen = watched_fingerprint(&text, generated.as_deref());
            }
            if reload.triggers.try_recv().is_err() {
                break;
            }
            while reload.triggers.try_recv().is_ok() {}
        }
    }
}

/// Reads, validates and builds the config at `path` and, only if all of that works, swaps it in.
/// A failure of any kind keeps the previous configuration serving and is recorded.
async fn reload_config(state: &AppState, path: &std::path::Path) {
    let _one_at_a_time = state.reloading.lock().await;
    let outcome = try_reload(state, path);
    state.metrics.observe_reload(outcome.is_ok());
    let mut status = state.reload.lock();
    status.attempts += 1;
    status.at_ms = Some(state.clock.now_ms());
    match outcome {
        Ok(fingerprint) => {
            tracing::info!(%fingerprint, "configuration reloaded");
            status.ok = Some(true);
            status.message = Some(format!("loaded {fingerprint}"));
        }
        Err(error) => {
            tracing::error!(%error, "configuration reload failed; the previous configuration keeps serving");
            status.ok = Some(false);
            status.message = Some(error);
        }
    }
}

fn try_reload(state: &AppState, path: &std::path::Path) -> Result<String, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let config = Config::from_toml_with(&text, |name| std::env::var(name).ok(), &|file| {
        std::fs::read_to_string(file).ok()
    })
    .map_err(|e| e.to_string())?;
    let current = state.snapshot();
    restart_only_changes(&current.config, &config)?;
    log_pool_changes(&current.config, &config);
    log_discovery(Some(&current.config), &config);
    let fingerprint = watched_fingerprint(&text, config.discovery.file.as_deref());
    let next = Snapshot::build(
        config,
        &state.clock,
        &state.extra_policy,
        &state.drains,
        Some(&current),
        fingerprint.clone(),
    )?;
    if let Some(tracker) = &state.tracker {
        tracker.set_restricted_at(next.config.budget.restricted_at);
    }
    state.metrics.set_vocabulary(&next.config);
    *state.snapshot.write() = Arc::new(next);
    Ok(fingerprint)
}

/// Names, in the log, each pool route whose membership a reload changed (ADR 0021): a strength moved
/// from one target to another reshapes a route without anyone editing it.
fn log_pool_changes(old: &Config, new: &Config) {
    for (route, pool) in &new.pools {
        let before = old.pools.get(route).map(|p| &p.members);
        if before != Some(&pool.members) {
            tracing::info!(
                route = %route,
                strength = %pool.strength,
                members = %pool.members.join(","),
                "pool membership changed"
            );
        }
    }
}

/// Settings a live process cannot change; a reload that touches one is refused as a whole.
fn restart_only_changes(old: &Config, new: &Config) -> Result<(), String> {
    let refuse = |what: &str| {
        Err(format!(
            "`{what}` changed: that needs a restart, so nothing was reloaded"
        ))
    };
    if old.listen != new.listen {
        return refuse("listen");
    }
    if old.ledger != new.ledger {
        return refuse("ledger path");
    }
    if old.shutdown_grace_secs != new.shutdown_grace_secs {
        return refuse("shutdown_grace_secs");
    }
    if old.reload_poll_secs != new.reload_poll_secs {
        return refuse("reload_poll_secs");
    }
    if old.decision_retention_days != new.decision_retention_days {
        return refuse("decision_retention_days");
    }
    if old.keys.is_empty() != new.keys.is_empty() {
        return refuse(
            "having keys at all (the budget tracker is only created for keyed gateways)",
        );
    }
    Ok(())
}

/// Everything `run` needs to serve and then shut down cleanly.
struct Built {
    router: Router,
    state: Arc<AppState>,
    accounting: Arc<Accounting>,
    hard_stop: watch::Sender<bool>,
}

impl Snapshot {
    /// Builds everything the config describes. With `previous` (the snapshot being replaced) the
    /// state of unchanged endpoints and routes is carried over.
    fn build(
        config: Config,
        clock: &Arc<dyn Clock>,
        extra_policy: &Arc<dyn RoutingPolicy>,
        drains: &Arc<dyn Drains>,
        previous: Option<&Snapshot>,
        fingerprint: String,
    ) -> Result<Self, String> {
        let targets = pool::build(&config, clock, drains, previous.map(|p| &p.targets))?;
        let routes = Routes::build(&config, previous.map(|p| (&p.config, &p.routes)))?;
        let policy: Arc<dyn RoutingPolicy> = Arc::new(All(vec![
            extra_policy.clone(),
            Arc::new(BudgetPolicy::new(&config)),
        ]));
        Ok(Self {
            selectors: Selectors::new(&config.select),
            keys: Arc::new(ConfigKeyStore::new(&config.keys)),
            loaded_at_ms: clock.now_ms(),
            config,
            targets,
            routes,
            policy,
            fingerprint,
        })
    }
}

/// A short, stable name for a config file's content.
/// The fingerprint a reload compares: the main config text, and the generated targets file when one is
/// configured, so a new generated file reloads the gateway exactly as an edit would (ADR 0022). Without a
/// generated file this is the plain fingerprint of the text.
/// The fingerprint the gateway would report after loading `path` (and the generated file `config` names), for
/// `check-config`, so a repository can be compared with a running gateway.
#[must_use]
pub fn config_fingerprint(path: &std::path::Path, config: &Config) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    Some(watched_fingerprint(&text, config.discovery.file.as_deref()))
}

fn watched_fingerprint(text: &str, generated: Option<&std::path::Path>) -> String {
    match generated {
        None => fingerprint(text),
        Some(path) => {
            let extra = std::fs::read_to_string(path).unwrap_or_default();
            fingerprint(&format!("{text}\n\u{0}generated targets\u{0}\n{extra}"))
        }
    }
}

/// Says what became of the generated targets file, when that changed (or at startup): loaded, or ignored
/// and why. A missing or empty file before the first sync is normal and is not noise.
fn log_discovery(old: Option<&Config>, new: &Config) {
    use crate::config::DiscoveryState;
    if old.is_some_and(|o| o.discovery_state == new.discovery_state) {
        return;
    }
    match &new.discovery_state {
        DiscoveryState::Loaded(count) => {
            tracing::info!(count, "generated targets loaded");
        }
        DiscoveryState::Ignored(reason) => {
            tracing::warn!(%reason, "generated targets file ignored; serving with the main configuration");
        }
        DiscoveryState::Off
        | DiscoveryState::Disabled
        | DiscoveryState::Missing
        | DiscoveryState::Empty => {}
    }
}

fn fingerprint(text: &str) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(text.as_bytes()))[..12].to_string()
}

/// Whether every endpoint of `target` is on a provider the overrides drain, which takes the whole target out of
/// a plan. With some endpoints left the target stays, and the pool skips the drained ones itself.
fn fully_drained(snap: &Snapshot, target: &str, overrides: &[Override]) -> bool {
    snap.config.targets.get(target).is_some_and(|endpoints| {
        !endpoints.is_empty()
            && endpoints
                .iter()
                .all(|e| crate::overrides::drains(overrides, &e.provider))
    })
}

/// The runtime-override store, which lives in the ledger, and the view of its drains that the pool reads; the
/// store is none, and nothing is drained, without a ledger.
async fn open_overrides(
    ledger: Option<&Ledger>,
    clock: &Arc<dyn Clock>,
) -> Result<(Option<Arc<dyn OverrideStore>>, Arc<dyn Drains>), String> {
    let Some(ledger) = ledger else {
        return Ok((None, Arc::new(NoDrains)));
    };
    let store: Arc<dyn OverrideStore> = Arc::new(
        LedgerOverrides::open(ledger.override_db(), clock.now_ms())
            .await
            .map_err(|e| e.to_string())?,
    );
    let drains = Arc::new(StoreDrains::new(store.clone(), clock.clone()));
    Ok((Some(store), drains))
}

async fn build(config: Config, options: Options, fingerprint: String) -> Result<Built, String> {
    let clock: Arc<dyn Clock> = options.clock.unwrap_or_else(|| Arc::new(SystemClock));
    let extra_policy: Arc<dyn RoutingPolicy> = options.policy.unwrap_or_else(|| Arc::new(AllowAll));
    let ledger = match &config.ledger {
        Some(path) => Some(
            Ledger::open_with(
                path,
                crate::ledger::DEFAULT_CAPACITY,
                config.decision_retention_days,
            )
            .await
            .map_err(|e| e.to_string())?,
        ),
        None => None,
    };
    let (overrides, drains) = open_overrides(ledger.as_ref(), &clock).await?;
    let tracker = (!config.keys.is_empty())
        .then(|| Arc::new(BudgetTracker::new(clock.clone(), &config.budget)));
    if let (Some(tracker), Some(ledger)) = (&tracker, &ledger) {
        tracker.hydrate(ledger).await.map_err(|e| e.to_string())?;
    }
    let metrics = Arc::new(crate::metrics::Metrics::new(crate::metrics::Settings {
        key_labels: config.metrics.key_labels,
        key_label_limit: config.metrics.key_label_limit,
    }));
    metrics.set_vocabulary(&config);
    let metrics_main = config.metrics.enabled && config.metrics.listen.is_none();
    let accounting = Arc::new(
        Accounting::new(ledger, clock.clone(), tracker.clone()).with_metrics(metrics.clone()),
    );
    let snapshot = Snapshot::build(config, &clock, &extra_policy, &drains, None, fingerprint)?;
    let (hard_stop_tx, hard_stop) = watch::channel(false);
    let state = Arc::new(AppState {
        snapshot: RwLock::new(Arc::new(snapshot)),
        hard_stop,
        accounting: accounting.clone(),
        tracker,
        clock: clock.clone(),
        quota: Arc::new(crate::quota::QuotaStore::default()),
        overrides,
        drains,
        metrics,
        extra_policy,
        reload: Mutex::new(ReloadStatus::default()),
        reloading: tokio::sync::Mutex::new(()),
    });
    let mut app = Router::new()
        .route("/healthz", get(|| async { StatusCode::OK }))
        .route("/v1/models", get(list_models))
        .route("/v1/key/info", get(key_info))
        .route("/v1/health", get(endpoint_health))
        .route("/v1/config", get(config_view::get_config))
        .route(
            "/v1/overrides",
            get(overrides::list_overrides).post(overrides::create_override),
        )
        .route(
            "/v1/overrides/{id}",
            axum::routing::delete(overrides::remove_override),
        )
        .route("/v1/route/explain", post(explain::explain_route))
        .route("/v1/decisions", get(decisions::list_decisions))
        .route("/v1/stats", get(stats::get_stats))
        .route("/v1/stats/switches", get(sessions::get_switches))
        .route("/v1/quota", get(quota::get_quota))
        .route("/v1/quota/history", get(quota::get_history))
        .route(
            "/v1/messages/count_tokens",
            post(|s, h, b| count_tokens(s, WireFormat::AnthropicMessages, h, b)),
        )
        .route(
            "/v1/responses/input_tokens",
            post(|s, h, b| count_tokens(s, WireFormat::OpenAiResponses, h, b)),
        )
        .route(
            "/v1/chat/completions",
            post(|s, h, b| infer(s, WireFormat::OpenAiChat, h, b)),
        )
        .route(
            "/v1/responses",
            post(|s, h, b| infer(s, WireFormat::OpenAiResponses, h, b)),
        )
        .route(
            "/v1/messages",
            post(|s, h, b| infer(s, WireFormat::AnthropicMessages, h, b)),
        );
    if metrics_main {
        app = app.route("/metrics", get(metrics::get_metrics));
    }
    let app = app.with_state(state.clone());
    spawn_quota_poller(&state, clock);
    Ok(Built {
        router: app,
        state,
        accounting,
        hard_stop: hard_stop_tx,
    })
}

/// Serves `/metrics` alone on its own address when `[metrics] listen` is set, so it is unreachable
/// from the public port by construction (ADR 0016).
async fn spawn_metrics_listener(
    state: &Arc<AppState>,
) -> Result<Option<tokio::task::JoinHandle<()>>, String> {
    let config = state.snapshot().config.metrics.clone();
    let Some(addr) = config.listen.filter(|_| config.enabled) else {
        return Ok(None);
    };
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|e| format!("cannot bind metrics listener {addr}: {e}"))?;
    tracing::info!(listen = %addr, "serving metrics");
    let app = Router::new()
        .route("/metrics", get(metrics::get_metrics))
        .with_state(state.clone());
    Ok(Some(tokio::spawn(async move {
        if let Err(error) = axum::serve(listener, app).await {
            tracing::error!(%error, "metrics listener stopped");
        }
    })))
}

/// Quota polling reads the live configuration each cycle and ends by itself once the server is
/// dropped (it holds only weak references). Polls worth keeping go to the ledger as history.
fn spawn_quota_poller(state: &Arc<AppState>, clock: Arc<dyn Clock>) {
    let live = Arc::downgrade(state);
    let history = Arc::downgrade(state);
    tokio::spawn(crate::quota::run(
        state.quota.clone(),
        clock,
        move || {
            live.upgrade()
                .map(|state| crate::quota::plan(&state.snapshot().config))
        },
        move |provider, windows, now_ms| {
            if let Some(state) = history.upgrade()
                && let Some(ledger) = state.accounting.ledger()
            {
                ledger.record_quota(
                    windows
                        .iter()
                        .map(|w| crate::ledger::QuotaSample {
                            ts_ms: now_ms,
                            provider: provider.to_string(),
                            window: w.name.clone(),
                            percent: w.percent,
                            status: w.status.clone(),
                            resets_at: w.resets_at.clone(),
                        })
                        .collect(),
                );
            }
        },
    ));
}

async fn list_models(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    let snap = state.snapshot();
    let caller = match authenticate(&snap, &headers).await {
        Ok(caller) => caller,
        Err(error) => return error.into_response_for(WireFormat::OpenAiChat),
    };
    let data: Vec<Value> = snap
        .config
        .model_names()
        .into_iter()
        .filter(|id| caller.as_ref().is_none_or(|key| key.may_use(id)))
        .map(|id| json!({"id": id, "object": "model", "created": 0, "owned_by": "humpyard"}))
        .collect();
    axum::Json(json!({"object": "list", "data": data})).into_response()
}

/// Token counting for the Anthropic (`count_tokens`) and Responses (`input_tokens`) protocols,
/// answered locally: no provider has a count call, so the number is an estimate of the prompt the
/// routed model would receive (see `crate::estimate`). It makes no upstream call and no ledger
/// entry, so it skips the budget and policy stages and stays available to a key at its limit;
/// authentication, the allowlist and the model check still apply.
async fn count_tokens(
    State(state): State<Arc<AppState>>,
    format: WireFormat,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let snap = state.snapshot();
    let counted = async {
        let caller = authenticate(&snap, &headers).await?;
        let decoded = decode(format, &body)?;
        let info = request_info(&headers);
        let stream = decoded.body["stream"].as_bool().unwrap_or(false);
        select_route(
            &snap,
            caller.as_ref(),
            &decoded.model,
            stream,
            request_features(&decoded.llm_request, &snap.selectors, &snap.config),
            &selector_headers(&headers),
            &info,
        )?;
        let upstream_form = encode_request(&decoded.llm_request, WireFormat::OpenAiChat)
            .map_err(|e| GatewayError::BadRequest(format!("cannot translate request: {e}")))?;
        Ok::<_, GatewayError>((decoded.model, estimate_tokens(&upstream_form)))
    }
    .await;
    match counted {
        Ok((model, input_tokens)) => {
            tracing::info!(%format, model, input_tokens, "token count (estimate)");
            let body = match format {
                WireFormat::OpenAiResponses => {
                    json!({"object": "response.input_tokens", "input_tokens": input_tokens})
                }
                _ => json!({"input_tokens": input_tokens}),
            };
            let mut response = axum::Json(body).into_response();
            set_header(&mut response, TOKEN_COUNT_HEADER, "estimate");
            response
        }
        Err(error) => {
            tracing::warn!(%format, status = error.status().as_u16(), error = %error, "token count failed");
            error.into_response_for(format)
        }
    }
}

/// Each endpoint's health: who is being skipped and for how long. Needs a key when the gateway
/// has keys, like every other `/v1` call.
async fn endpoint_health(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    let snap = state.snapshot();
    if let Err(error) = authenticate(&snap, &headers).await {
        return error.into_response_for(WireFormat::OpenAiChat);
    }
    let mut targets: Vec<_> = snap.targets.iter().collect();
    targets.sort_by(|a, b| a.0.cmp(b.0));
    let report: Vec<_> = targets
        .into_iter()
        .map(|(target, client)| {
            let endpoints: Vec<_> = client
                .health()
                .into_iter()
                .map(|e| {
                    json!({
                        "provider": e.provider,
                        "model": e.model,
                        "state": e.snapshot.state.as_str(),
                        "consecutive_failures": e.snapshot.consecutive_failures,
                        "cooldown_remaining_ms": e.snapshot.cooldown_remaining_ms,
                        "cooldown_source": e.snapshot.cooldown_source.map(crate::health::CooldownSource::as_str),
                    })
                })
                .collect();
            let figure = state.accounting.latency().figure(target.as_str());
            json!({
                "target": target.to_string(),
                "current_latency_ms": figure.map(|f| f.ms),
                "latency_samples": figure.map(|f| f.samples),
                "endpoints": endpoints,
            })
        })
        .collect();
    let reload = state.reload.lock().clone();
    axum::Json(json!({
        "targets": report,
        "config": {"fingerprint": snap.fingerprint, "loaded_at_ms": snap.loaded_at_ms},
        "reload": {
            "attempts": reload.attempts,
            "at_ms": reload.at_ms,
            "ok": reload.ok,
            "message": reload.message,
        },
    }))
    .into_response()
}

/// The calling key's id, limits, spend and budget state.
async fn key_info(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    let snap = state.snapshot();
    let format = WireFormat::OpenAiChat;
    let caller = match authenticate(&snap, &headers).await {
        Ok(Some(caller)) => caller,
        Ok(None) => {
            return GatewayError::ModelNotFound("key info needs a key: the gateway is open".into())
                .into_response_for(format);
        }
        Err(error) => return error.into_response_for(format),
    };
    let Some(tracker) = &state.tracker else {
        return GatewayError::Internal("budget tracking is not running".into())
            .into_response_for(format);
    };
    let status = tracker.status(&caller.id, &caller.limits);
    let usd = |micro: u64| f64_from_u64(micro) / 1_000_000.0;
    let remaining = |limit: Option<f64>, spent: f64| limit.map(|l| (l - spent).max(0.0));
    axum::Json(json!({
        "id": caller.id,
        "state": match status.state {
            BudgetState::Healthy => "healthy",
            BudgetState::Restricted => "restricted",
            BudgetState::Exhausted => "exhausted",
        },
        "binding_limit": status.binding_limit,
        "limits": {
            "daily_usd": status.limits.daily_usd,
            "monthly_usd": status.limits.monthly_usd,
            "daily_tokens": status.limits.daily_tokens,
            "monthly_tokens": status.limits.monthly_tokens,
        },
        "spend": {
            "daily_usd": usd(status.day.micro_usd),
            "monthly_usd": usd(status.month.micro_usd),
            "daily_tokens": status.day.tokens,
            "monthly_tokens": status.month.tokens,
        },
        "remaining": {
            "daily_usd": remaining(status.limits.daily_usd, usd(status.day.micro_usd)),
            "monthly_usd": remaining(status.limits.monthly_usd, usd(status.month.micro_usd)),
            "daily_tokens": remaining(status.limits.daily_tokens.map(f64_from_u64), f64_from_u64(status.day.tokens)),
            "monthly_tokens": remaining(status.limits.monthly_tokens.map(f64_from_u64), f64_from_u64(status.month.tokens)),
        },
    }))
    .into_response()
}

/// Resolves once shutdown has given up waiting. If the signal can never fire (the router was built
/// without `run`), it never resolves.
async fn stopped(mut stop: watch::Receiver<bool>) {
    loop {
        if *stop.borrow() {
            return;
        }
        if stop.changed().await.is_err() {
            std::future::pending::<()>().await;
        }
    }
}

/// Identifies the caller. `None` means the gateway is open (no keys configured).
async fn authenticate(
    snap: &Snapshot,
    headers: &HeaderMap,
) -> Result<Option<KeyRecord>, GatewayError> {
    if !snap.keys.enforces_auth() {
        return Ok(None);
    }
    let key = presented_key(headers).ok_or_else(|| {
        GatewayError::Unauthorized(
            "missing API key: send `Authorization: Bearer <key>` or `x-api-key`".into(),
        )
    })?;
    match snap.keys.lookup(&hash_key(key)).await {
        Some(record) => Ok(Some(record)),
        None => Err(GatewayError::Unauthorized("invalid API key".into())),
    }
}

async fn infer(
    State(state): State<Arc<AppState>>,
    format: WireFormat,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let snap = state.snapshot();
    let started = std::time::Instant::now();
    let mut guard = DecisionGuard::new(&state, &snap, format);
    let request_id = guard.decision.request_id.clone();
    let caller = match authenticate(&snap, &headers).await {
        Ok(caller) => caller,
        Err(error) => {
            tracing::warn!(%format, status = error.status().as_u16(), error = %error, request_id, "request rejected");
            return guard.refuse(error, format);
        }
    };
    let key_id = caller.as_ref().map_or("-", |k| k.id.as_str()).to_string();
    guard.decision.key_id = caller.as_ref().map(|k| k.id.clone());
    guard.decision.session_id = request_info(&headers).session_id;
    let outcome = tokio::select! {
        outcome = handle(&state, &snap, format, &headers, &body, caller.as_ref(), &mut guard) => outcome,
        () = stopped(state.hard_stop.clone()) => {
            Err(GatewayError::Unavailable("gateway is shutting down".into()))
        }
    };
    match outcome {
        Ok((target, response)) => {
            let provider = response
                .headers()
                .get(PROVIDER_HEADER)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("")
                .to_string();
            tracing::info!(
                %format,
                key = key_id,
                target,
                route = response
                    .headers()
                    .get(ROUTE_HEADER)
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or(""),
                rule = response
                    .headers()
                    .get(RULE_HEADER)
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or(""),
                provider,
                status = response.status().as_u16(),
                elapsed_ms = started.elapsed().as_millis(),
                request_id,
                "request"
            );
            guard.succeed(response, &target, &provider)
        }
        Err(error) => {
            tracing::warn!(%format, key = key_id, status = error.status().as_u16(), error = %error, request_id, "request failed");
            guard.refuse(error, format)
        }
    }
}

/// Owns a request's decision for its whole life and records it however the request ends: served,
/// refused, failed, or abandoned (a client that disconnects drops the handler, and `Drop` records
/// the decision as `cancelled`). See ADR 0013.
struct DecisionGuard {
    accounting: Arc<Accounting>,
    metrics: Arc<crate::metrics::Metrics>,
    decision: Decision,
    /// The request's upstream calls, once routing has reached them.
    ctx: Option<Arc<CallContext>>,
    started: std::time::Instant,
    recorded: bool,
    /// Target class labels, applied to the attempts when the decision is recorded.
    classes: HashMap<String, String>,
    /// Kept apart from `decision` because recording moves the decision out.
    decision_id: String,
}

impl DecisionGuard {
    fn new(state: &AppState, snap: &Snapshot, format: WireFormat) -> Self {
        let now_ms = state.clock.now_ms();
        let request_id = new_request_id(now_ms);
        Self {
            accounting: state.accounting.clone(),
            metrics: state.metrics.clone(),
            decision_id: request_id.clone(),
            decision: Decision {
                request_id,
                ts_ms: now_ms,
                format: format.to_string(),
                ..Decision::default()
            },
            ctx: None,
            started: std::time::Instant::now(),
            recorded: false,
            classes: snap
                .config
                .target_classes
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
        }
    }

    /// Records the decision with its final facts. Does nothing if already recorded.
    fn record(&mut self, status: u16, outcome: &str, message: Option<String>) {
        if std::mem::replace(&mut self.recorded, true) {
            return;
        }
        if let Some(ctx) = &self.ctx {
            self.decision.attempts = ctx.attempts();
            for attempt in &mut self.decision.attempts {
                attempt.class = self.classes.get(&attempt.target).cloned();
            }
        }
        self.decision.status = status;
        self.decision.outcome = outcome.to_string();
        self.decision.message = message;
        self.decision.latency_ms =
            u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX);
        self.metrics.observe_decision(&self.decision);
        if let Some(ledger) = self.accounting.ledger() {
            ledger.record_decision(std::mem::take(&mut self.decision));
        }
    }

    /// The request was served: stamps the id on the response and records the decision.
    fn succeed(mut self, mut response: Response, target: &str, provider: &str) -> Response {
        self.decision.served_target = Some(target.to_string());
        self.decision.served_provider = Some(provider.to_string());
        self.record(response.status().as_u16(), "ok", None);
        set_header(&mut response, REQUEST_ID_HEADER, &self.request_id());
        response
    }

    /// The request ended in an error: records it, and renders the error with the id.
    fn refuse(mut self, error: GatewayError, format: WireFormat) -> Response {
        self.record(
            error.status().as_u16(),
            error.outcome(),
            Some(error.to_string()),
        );
        let id = self.request_id();
        let mut response = error.into_response_for(format);
        set_header(&mut response, REQUEST_ID_HEADER, &id);
        response
    }

    fn request_id(&self) -> String {
        self.decision_id.clone()
    }
}

impl Drop for DecisionGuard {
    fn drop(&mut self) {
        // Reaching here unrecorded means the handler was dropped mid-request (client gone).
        self.record(499, "cancelled", None);
    }
}

/// 16 hex characters of randomness; the clock stands in if randomness is unavailable, because a
/// request must never fail for want of an id.
fn new_request_id(now_ms: i64) -> String {
    let mut bytes = [0_u8; 8];
    if getrandom::fill(&mut bytes).is_err() {
        bytes = now_ms.to_be_bytes();
    }
    hex::encode(bytes)
}

fn budget_label(state: BudgetState) -> &'static str {
    match state {
        BudgetState::Healthy => "healthy",
        BudgetState::Restricted => "restricted",
        BudgetState::Exhausted => "exhausted",
    }
}

/// A client request, decoded.
struct Decoded {
    body: Value,
    llm_request: LlmRequest,
    model: String,
}

/// What the headers say about this request.
struct RequestInfo {
    metadata: Metadata,
    session_id: Option<String>,
    meta: RequestMeta,
}

/// A finished upstream run, before the response is encoded.
struct Executed {
    selected: ModelId,
    response: switchyard_protocol::Response,
    ctx: Arc<CallContext>,
    stream_template: Option<StreamTemplate>,
}

/// The request path, in the order `docs/invariants.md` (item 5) fixes: decode, authorize, check
/// the budget, plan with the policy, run, encode. Nothing reaches an upstream before the first
/// four succeed.
async fn handle(
    state: &AppState,
    snap: &Snapshot,
    format: WireFormat,
    headers: &HeaderMap,
    raw: &[u8],
    caller: Option<&KeyRecord>,
    guard: &mut DecisionGuard,
) -> Result<(String, Response), GatewayError> {
    let Decoded {
        body,
        llm_request,
        model,
    } = decode(format, raw)?;
    let info = request_info(headers);
    let stream = body["stream"].as_bool().unwrap_or(false);
    let features = request_features(&llm_request, &snap.selectors, &snap.config);
    guard.decision.requested_model.clone_from(&model);
    guard.decision.stream = stream;
    guard.decision.prompt_tokens = features.prompt_tokens;
    guard.decision.tools = features.tools;
    guard.decision.images = features.images;
    let selection = select_route(
        snap,
        caller,
        &model,
        stream,
        features,
        &selector_headers(headers),
        &info,
    )?;
    guard.decision.route = Some(selection.name.clone());
    guard.decision.rule = Some(selection.rule.clone());
    guard.decision.source = Some(
        if selection.rule == "default" {
            "requested_model"
        } else {
            "selector"
        }
        .to_string(),
    );
    guard.decision.skipped_rules.clone_from(&selection.skipped);
    guard.decision.excluded_targets = excluded_targets(snap, &selection.name);
    let budget = check_budget(state, caller)?;
    guard.decision.budget = budget.map(|b| budget_label(b).to_string());
    let overrides = state.active_overrides().await;
    let Planned {
        plan,
        cold_skipped,
        incapable,
        ..
    } = plan_route(
        snap,
        selection.route,
        &selection.name,
        &PlanInput {
            caller,
            budget,
            info: &info,
            features: &features,
            latency: Some(state.accounting.latency()),
            overrides: &overrides,
        },
    )
    .map_err(|failure| {
        guard.decision.incapable_targets = failure.incapable;
        failure.error
    })?;
    guard.decision.cold_targets = cold_skipped;
    guard.decision.incapable_targets = incapable;
    guard.decision.ranking = plan.ranking.as_ref().map(|ranked| crate::ledger::Ranking {
        order: ranked.order.clone(),
        reason: ranked.reason.as_str().to_string(),
    });
    guard.decision.eligible_targets = plan
        .models
        .models_for(&Category::Any)
        .iter()
        .map(ToString::to_string)
        .collect();
    let extensions = llm_request.extensions.clone();
    let request = Request {
        llm_request,
        raw_request: Some(body),
        metadata: Some(info.metadata),
    };
    let ctx = CallContext::new(
        guard.decision.request_id.clone(),
        state.accounting.clone(),
        caller.map(|k| k.id.clone()),
        info.session_id,
        selection.name.clone(),
    );
    guard.ctx = Some(ctx.clone());
    let executed = execute(snap, &ctx, plan, request).await?;
    let (target, mut response) = encode(format, state.hard_stop.clone(), executed, &extensions)?;
    set_header(&mut response, ROUTE_HEADER, &selection.name);
    set_header(&mut response, RULE_HEADER, &selection.rule);
    Ok((target, response))
}

/// Stage 1: parse the body and translate it into Switchyard's neutral form.
fn decode(format: WireFormat, raw: &[u8]) -> Result<Decoded, GatewayError> {
    let mut body: Value = serde_json::from_slice(raw)
        .map_err(|e| GatewayError::BadRequest(format!("invalid JSON body: {e}")))?;
    // Only trusted translation hops may supply preservation state; strip any from clients.
    if let Some(metadata) = body.get_mut("metadata").and_then(Value::as_object_mut) {
        metadata.remove(SWITCHYARD_METADATA_KEY);
    }
    let llm_request = decode_request(format, &body)
        .map_err(|e| GatewayError::BadRequest(format!("cannot translate request: {e}")))?;
    let model = llm_request
        .model
        .clone()
        .filter(|m| !m.trim().is_empty())
        .ok_or_else(|| GatewayError::BadRequest("`model` is required".into()))?;
    if format == WireFormat::OpenAiResponses
        && body
            .get("previous_response_id")
            .is_some_and(|v| !v.is_null())
    {
        return Err(GatewayError::BadRequest(
            "`previous_response_id` is not supported: stateful responses are unavailable".into(),
        ));
    }
    Ok(Decoded {
        body,
        llm_request,
        model,
    })
}

/// Stage 2: the key may use this route, and the route exists.
fn authorize<'a>(
    snap: &'a Snapshot,
    caller: Option<&KeyRecord>,
    model: &str,
) -> Result<&'a Route, GatewayError> {
    if let Some(key) = caller
        && !key.may_use(model)
    {
        return Err(GatewayError::Forbidden(format!(
            "this key may not use `{model}`"
        )));
    }
    snap.routes
        .get(model)
        .ok_or_else(|| GatewayError::ModelNotFound(model.to_string()))
}

/// What the request contains, as selector rules see it. The prompt size is an estimate and is only
/// computed when some rule looks at it.
fn request_features(
    llm_request: &switchyard_protocol::LlmRequest,
    selectors: &Selectors,
    config: &Config,
) -> Features {
    // The prompt is measured only when a rule needs the ordinary estimate or an endpoint declares a
    // context window (which needs the low one, ADR 0019); otherwise no request pays for it.
    let want_estimate = selectors.needs_prompt_estimate();
    let want_low = config.declares_context_windows();
    let body = if want_estimate || want_low {
        encode_request(llm_request, WireFormat::OpenAiChat).ok()
    } else {
        None
    };
    Features {
        // Both estimates come from the same body, so whenever it is measured both are recorded: the
        // decision then carries the ordinary estimate for tuning the low one (docs/routing.md).
        prompt_tokens: body.as_ref().map(estimate_tokens),
        prompt_tokens_low: body.as_ref().map(crate::estimate::estimate_tokens_low),
        tools: !llm_request.tools.is_empty(),
        images: llm_request.messages.iter().any(|message| {
            message
                .content
                .iter()
                .any(|block| matches!(block, switchyard_protocol::ContentBlock::Image { .. }))
        }),
    }
}

/// The route a request follows, and why.
struct Selection<'a> {
    route: &'a Route,
    name: String,
    /// The selector rule that chose it, or `default` when the requested model named the route.
    rule: String,
    /// Rules that matched but were skipped because the key may not use their route.
    skipped: Vec<String>,
}

/// Request headers rules may match: names lower-cased, credential headers left out.
fn selector_headers(headers: &HeaderMap) -> HashMap<String, String> {
    headers
        .iter()
        .filter(|(name, _)| !CREDENTIAL_HEADERS.contains(&name.as_str()))
        .filter_map(|(name, value)| {
            Some((name.as_str().to_string(), value.to_str().ok()?.to_string()))
        })
        .collect()
}

/// Stage 2: choose the route. Selector rules go first; a rule only applies if the key may use its
/// route, so client headers can narrow the flow but never widen it (ADR 0011). With no applicable
/// rule the requested model names the route, subject to the key's allowlist as before.
fn select_route<'a>(
    snap: &'a Snapshot,
    caller: Option<&KeyRecord>,
    model: &str,
    stream: bool,
    features: Features,
    header_facts: &HashMap<String, String>,
    info: &RequestInfo,
) -> Result<Selection<'a>, GatewayError> {
    let mut skipped_rules = Vec::new();
    if !snap.selectors.is_empty() {
        let facts = Facts {
            model,
            key: caller.map(|k| k.id.as_str()),
            headers: header_facts,
            agent: info.meta.agent_id.as_deref(),
            task: info.meta.task_id.as_deref(),
            subagent: info.meta.is_subagent,
            stream,
            features,
        };
        let (chosen, skipped) = snap
            .selectors
            .choose_detailed(&facts, |route| caller.is_none_or(|key| key.may_use(route)));
        skipped_rules = skipped.into_iter().map(String::from).collect();
        if let Some(chosen) = chosen
            && let Some(route) = snap.routes.get(chosen.route)
        {
            return Ok(Selection {
                route,
                name: chosen.route.to_string(),
                rule: chosen.rule.to_string(),
                skipped: skipped_rules,
            });
        }
    }
    let route = authorize(snap, caller, model)?;
    Ok(Selection {
        route,
        name: model.to_string(),
        rule: "default".to_string(),
        skipped: skipped_rules,
    })
}

/// Stage 2b: what the client's headers tell us about the session and agent.
/// Header names, after the ones the routing engine recognizes, that carry a client's session id.
const SESSION_ID_FALLBACKS: [&str; 2] = ["session_id", "x-session-affinity"];
const MAX_SESSION_ID_LEN: usize = 256;

/// A session id is client text that reaches the ledger and statistics, so it must be 1 to 256 visible
/// ASCII characters; anything else is ignored rather than failing the request.
fn usable_session_id(id: &str) -> bool {
    (1..=MAX_SESSION_ID_LEN).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_graphic())
}

fn fallback_session_id(headers: &HeaderMap) -> Option<String> {
    SESSION_ID_FALLBACKS.iter().find_map(|name| {
        headers
            .get(*name)
            .and_then(|value| value.to_str().ok())
            .filter(|id| usable_session_id(id))
            .map(str::to_string)
    })
}

fn request_info(headers: &HeaderMap) -> RequestInfo {
    // `wire_format` stays unset: setting it pins the backend to the client's protocol, but every
    // endpoint speaks OpenAI Chat and the IR is translated for it.
    let mut metadata = Metadata::from_headers(headers);
    // Headers the routing engine recognizes win; the fallbacks cover clients (OpenClaw with session
    // affinity on) that send a session-shaped name it does not know. The id goes into the metadata so the
    // engine's per-session state and the recorded session agree.
    let session_id = metadata
        .session_id
        .clone()
        .filter(|id| usable_session_id(id))
        .or_else(|| fallback_session_id(headers));
    metadata.session_id.clone_from(&session_id);
    let meta = RequestMeta {
        agent_id: metadata.agent_id.clone(),
        parent_agent_id: metadata.parent_agent_id.clone(),
        is_subagent: metadata.is_subagent,
        task_id: metadata.task_id.clone(),
    };
    RequestInfo {
        metadata,
        session_id,
        meta,
    }
}

/// Stage 3: refuse a key that has used up its budget (unless it may continue on free targets).
/// Returns the key's budget state, or `None` for open-mode requests.
fn check_budget(
    state: &AppState,
    caller: Option<&KeyRecord>,
) -> Result<Option<BudgetState>, GatewayError> {
    let Some((key, tracker)) = caller.zip(state.tracker.as_ref()) else {
        return Ok(None);
    };
    let status = tracker.status(&key.id, &key.limits);
    if status.state == BudgetState::Exhausted && key.over_budget == OverBudget::Block {
        return Err(GatewayError::BudgetExceeded(format!(
            "budget exhausted: key `{}` reached its {}",
            key.id,
            status.binding_limit.unwrap_or("limit")
        )));
    }
    Ok(Some(status.state))
}

/// Whether the route may use `target` given its `allowed_classes`. A route without the setting
/// (the default) may use every target; a strict route only targets of a listed class (ADR 0017).
fn class_allows(snap: &Snapshot, route: &str, target: &str) -> bool {
    let Some(allowed) = snap
        .config
        .routes
        .get(route)
        .and_then(crate::config::RouteSpec::allowed_classes)
    else {
        return true;
    };
    snap.config
        .target_classes
        .get(target)
        .is_some_and(|class| allowed.iter().any(|a| a == class))
}

/// The targets a strict route mentions but its class list excludes, in the order it mentions them.
fn excluded_targets(snap: &Snapshot, route: &str) -> Vec<String> {
    let Some(spec) = snap.config.routes.get(route) else {
        return Vec::new();
    };
    let mut excluded: Vec<String> = Vec::new();
    for target in spec.target_refs() {
        if !class_allows(snap, route, target) && !excluded.iter().any(|e| e == target) {
            excluded.push(target.to_string());
        }
    }
    excluded
}

/// Why no endpoint of `target` can serve a request with `features`, or `None` when one can: tools,
/// images, or a prompt certainly larger than every declared context window (ADR 0019). An endpoint
/// that declares nothing is capable, so an undeclared config removes nothing.
fn incapable_reason(snap: &Snapshot, target: &str, features: &Features) -> Option<&'static str> {
    let endpoints = snap.config.targets.get(target)?;
    let mut first = None;
    for endpoint in endpoints {
        let reason = if features.tools && endpoint.tools == Some(false) {
            Some("tools")
        } else if features.images && endpoint.vision == Some(false) {
            Some("vision")
        } else if let (Some(window), Some(low)) =
            (endpoint.context_window, features.prompt_tokens_low)
            && low > window
        {
            Some("context")
        } else {
            None
        };
        first.get_or_insert(reason?);
    }
    first
}

/// The sentence explaining why a target was removed for capability, for the refusal message.
fn incapable_sentence(incapable: &Incapable, features: &Features) -> String {
    let why = match incapable.reason.as_str() {
        "tools" => "the request uses tools but its endpoints declare tools = false".to_string(),
        "vision" => {
            "the request carries images but its endpoints declare vision = false".to_string()
        }
        _ => format!(
            "the prompt is at least about {} tokens, more than its declared context_window",
            features.prompt_tokens_low.unwrap_or(0)
        ),
    };
    format!("`{}`: {why}", incapable.target)
}

/// Planning failed: the error to answer with, and the targets capability removed (recorded on the
/// decision even when the request is refused).
struct PlanFailure {
    error: GatewayError,
    incapable: Vec<Incapable>,
}

/// A planned route plus what health did to it (ADR 0018).
struct Planned {
    plan: Plan,
    /// Targets removed because none of their endpoints could serve the request (ADR 0019).
    incapable: Vec<Incapable>,
    /// Targets left out because every endpoint was cooling down and another target remained.
    cold_skipped: Vec<String>,
    /// Every eligible target was cold, so none was skipped: the route is tried as it would be without
    /// health skipping (health never denies service).
    fail_open: bool,
}

/// Whether every endpoint of `target` is cooling down. `probing` is warm: the cooldown has ended and
/// the next request is the probe that lets the target recover.
fn is_cold(snap: &Snapshot, target: &str) -> bool {
    snap.targets
        .iter()
        .find(|(id, _)| id.as_str() == target)
        .is_some_and(|(_, client)| {
            let health = client.health();
            !health.is_empty()
                && health
                    .iter()
                    .all(|e| e.snapshot.state == crate::health::State::CoolingDown)
        })
}

/// What planning needs to know about one request besides the route.
#[derive(Clone, Copy)]
struct PlanInput<'a> {
    caller: Option<&'a KeyRecord>,
    budget: Option<BudgetState>,
    info: &'a RequestInfo,
    features: &'a Features,
    /// Orders a `fastest` route's targets; `None` leaves the written order (explain does not explore).
    latency: Option<&'a Latency>,
    /// The runtime overrides in force; they only ever remove candidates.
    overrides: &'a [Override],
}

/// Stage 4: let the routing policy narrow the targets, a strict route drop the targets of classes it
/// does not allow, and health skip cold targets while another remains, all before the algorithm sees
/// any of them; nothing eligible is a refusal.
fn plan_route(
    snap: &Snapshot,
    route: &Route,
    model: &str,
    input: &PlanInput,
) -> Result<Planned, PlanFailure> {
    let PlanInput {
        caller,
        budget,
        info,
        features,
        latency,
        overrides,
    } = *input;
    let context = PolicyContext {
        route: model,
        session_id: info.session_id.as_deref(),
        metadata: &info.meta,
        key: caller.map(|key| KeyContext {
            id: &key.id,
            budget: budget.unwrap_or(BudgetState::Healthy),
        }),
    };
    let base = |target: &str| {
        class_allows(snap, model, target) && snap.policy.is_eligible(&context, target)
    };
    // Targets none of whose endpoints can serve this request, among those classes and policy leave
    // eligible, in the route's order (ADR 0019). They are removed before health is consulted.
    let mut incapable: Vec<Incapable> = Vec::new();
    if let Some(spec) = snap.config.routes.get(model) {
        for target in spec.target_refs() {
            if base(target)
                && !incapable.iter().any(|i| i.target == target)
                && let Some(reason) = incapable_reason(snap, target, features)
            {
                incapable.push(Incapable {
                    target: target.to_string(),
                    reason: reason.to_string(),
                });
            }
        }
    }
    // Runtime overrides come next, after strictness and capabilities and before health, so health and a
    // guarded tail see the reduced set. They only remove candidates.
    let capable = |target: &str| base(target) && !incapable.iter().any(|i| i.target == target);
    let eligible = |target: &str| {
        capable(target)
            && !crate::overrides::removes(overrides, model, target)
            && !fully_drained(snap, target, overrides)
    };
    // The cold targets among those capable, in the route's order.
    let mut cold: Vec<String> = Vec::new();
    if snap.config.health.skip_cold_targets
        && let Some(spec) = snap.config.routes.get(model)
    {
        for target in spec.target_refs() {
            if eligible(target) && is_cold(snap, target) && !cold.iter().any(|c| c == target) {
                cold.push(target.to_string());
            }
        }
    }
    // First pass leaves cold targets out; if that leaves nothing, plan again with them (fail open).
    let quality = |target: &str| snap.config.target_quality.get(target).copied();
    let warm_plan = route.plan_ranked(
        model,
        |target| eligible(target) && !cold.iter().any(|c| c == target),
        latency,
        &quality,
    );
    // A pool whose members are all cold must try them again before a paid tail is used (ADR 0024).
    let warm_plan = warm_plan.filter(|plan| !plan.tail_only || cold.is_empty());
    let (plan, cold_skipped, fail_open) = if let Some(plan) = warm_plan {
        (Some(plan), cold, false)
    } else {
        let open = !cold.is_empty();
        (
            route.plan_ranked(model, eligible, latency, &quality),
            Vec::new(),
            open,
        )
    };
    if let Some(plan) = plan {
        return Ok(Planned {
            plan,
            incapable,
            cold_skipped,
            fail_open,
        });
    }
    // Nothing is left. If capability alone emptied the route, say why (known before any upstream
    // call); otherwise it is the budget or the routing policy.
    let error = if !incapable.is_empty() && route.plan(model, base).is_some() {
        let reasons: Vec<String> = incapable
            .iter()
            .map(|i| incapable_sentence(i, features))
            .collect();
        GatewayError::NoCapableTarget(format!(
            "no target of route `{model}` can serve this request: {}",
            reasons.join("; ")
        ))
    } else if !overrides.is_empty() && route.plan(model, capable).is_some() {
        GatewayError::OverrideNoTarget(format!(
            "runtime overrides removed every target of route `{model}`; remove the override to restore service"
        ))
    } else if budget == Some(BudgetState::Exhausted) {
        GatewayError::BudgetExceeded(format!(
            "budget exhausted and route `{model}` has no free target to continue on"
        ))
    } else {
        GatewayError::Unavailable(format!(
            "no eligible target for route `{model}`: all are excluded by routing policy"
        ))
    };
    Err(PlanFailure { error, incapable })
}

/// Stage 5: run the route's algorithm over the metered clients and settle the usage accounting.
async fn execute(
    snap: &Snapshot,
    ctx: &Arc<CallContext>,
    plan: Plan,
    request: Request,
) -> Result<Executed, GatewayError> {
    let names: Vec<ModelId> = plan.models.models_for(&Category::Any).to_vec();
    let clients = ctx.router(
        &snap.targets,
        &names,
        &plan.fallback,
        plan.tail_guard
            .as_ref()
            .map(|g| (g.last_member.as_str(), &g.on)),
    );
    let models = Arc::new(plan.models);
    let (selected, mut response) =
        match run_algorithm(plan.algorithm, clients, request, models, None).await {
            Ok(done) => done,
            Err(error) => {
                ctx.finish_err();
                return Err(error.into());
            }
        };
    let answer_id = response
        .upstream_headers
        .remove(CALL_ID_HEADER)
        .and_then(|v| v.to_str().ok()?.parse::<u64>().ok());
    let stream_template = ctx.finish_ok(answer_id);
    Ok(Executed {
        selected,
        response,
        ctx: ctx.clone(),
        stream_template,
    })
}

/// Stage 6: encode the answer for the client's protocol and attach the attribution headers.
fn encode(
    format: WireFormat,
    stop: watch::Receiver<bool>,
    executed: Executed,
    extensions: &switchyard_protocol::ProviderExtensions,
) -> Result<(String, Response), GatewayError> {
    let Executed {
        selected,
        mut response,
        ctx,
        stream_template,
    } = executed;
    let served = response.served_model().unwrap_or(&selected).to_string();
    let upstream_headers = std::mem::take(&mut response.upstream_headers);
    let mut http_response = match response.llm_response {
        LlmResponse::Agg(agg) => {
            let body =
                encode_aggregated_response_with_extensions(&agg, format, Some(&served), extensions)
                    .map_err(|e| GatewayError::Internal(format!("cannot encode response: {e}")))?;
            axum::Json(body).into_response()
        }
        LlmResponse::Stream(stream) => {
            let stream = match stream_template {
                Some(template) => ctx.tap(stream, template),
                None => stream,
            };
            let events =
                encode_stream_with_extensions(stream, format, Some(served.clone()), extensions)
                    .map_err(|e| GatewayError::Internal(e.to_string()))?;
            frame_stream(events, format, stop).into_response()
        }
    };
    set_header(&mut http_response, TARGET_HEADER, &served);
    if let Some(provider) = upstream_headers
        .get(PROVIDER_HEADER)
        .and_then(|v| v.to_str().ok())
    {
        set_header(&mut http_response, PROVIDER_HEADER, provider);
    }
    Ok((served, http_response))
}

fn set_header(response: &mut Response, name: &'static str, value: &str) {
    if let Ok(value) = HeaderValue::from_str(value) {
        response
            .headers_mut()
            .insert(HeaderName::from_static(name), value);
    }
}

type SseStream = std::pin::Pin<Box<dyn Stream<Item = Result<Event, Infallible>> + Send>>;

/// Frames translated events as SSE for the client's protocol. Dropping the returned body drops
/// the upstream stream, which cancels the upstream request.
fn frame_stream(
    stream: RawEventStream,
    format: WireFormat,
    stop: watch::Receiver<bool>,
) -> Sse<SseStream> {
    let framed = async_stream::stream! {
        let mut stream = stream;
        let mut failed = false;
        loop {
            let item = tokio::select! {
                item = stream.next() => item,
                () = stopped(stop.clone()) => {
                    // Shutdown gave up waiting: end the stream with an error event. Dropping the
                    // stream afterwards lets the usage tap record it as cancelled.
                    yield Ok(error_event(format, "gateway is shutting down"));
                    break;
                }
            };
            let Some(item) = item else { break };
            let event = match item {
                Ok(value) => frame_event(format, &value),
                Err(LlmStreamError::Upstream(value)) => {
                    failed = true;
                    frame_event(format, &value)
                }
                Err(LlmStreamError::Client(error)) => {
                    tracing::warn!("stream iteration failed");
                    failed = true;
                    error_event(format, &error.to_string())
                }
            };
            yield Ok(event);
            if failed {
                break;
            }
        }
        // `[DONE]` marks a successful OpenAI Chat stream; it must not follow a failed turn.
        if !failed && format == WireFormat::OpenAiChat {
            yield Ok(Event::default().data("[DONE]"));
        }
    };
    Sse::new(Box::pin(framed) as SseStream)
}

/// Anthropic and Responses clients expect a named SSE event matching the payload's `type`.
fn frame_event(format: WireFormat, value: &Value) -> Event {
    let data = value.to_string();
    match format {
        WireFormat::OpenAiChat => Event::default().data(data),
        WireFormat::AnthropicMessages | WireFormat::OpenAiResponses => {
            let name = value["type"].as_str().unwrap_or("message");
            Event::default().event(name).data(data)
        }
    }
}

fn error_event(format: WireFormat, message: &str) -> Event {
    let body = GatewayError::UpstreamUnreachable(message.to_string()).body(format);
    match format {
        WireFormat::OpenAiChat => Event::default().data(body.to_string()),
        _ => Event::default().event("error").data(body.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn raw(value: &Value) -> Vec<u8> {
        serde_json::to_vec(value).unwrap()
    }

    #[test]
    fn decode_accepts_a_chat_request_and_names_the_model() {
        let body = json!({"model": "fast", "messages": [{"role": "user", "content": "hi"}]});
        let decoded = decode(WireFormat::OpenAiChat, &raw(&body)).unwrap();
        assert_eq!(decoded.model, "fast");
        assert_eq!(decoded.body, body);
    }

    #[test]
    fn decode_rejects_bad_json_a_missing_model_and_stateful_responses() {
        let bad = decode(WireFormat::OpenAiChat, b"{nope").err().unwrap();
        assert!(matches!(bad, GatewayError::BadRequest(m) if m.contains("invalid JSON")));

        let no_model = json!({"messages": [{"role": "user", "content": "hi"}]});
        let err = decode(WireFormat::OpenAiChat, &raw(&no_model))
            .err()
            .unwrap();
        assert!(matches!(err, GatewayError::BadRequest(m) if m.contains("`model` is required")));

        let blank = json!({"model": "  ", "messages": [{"role": "user", "content": "hi"}]});
        assert!(decode(WireFormat::OpenAiChat, &raw(&blank)).is_err());

        let stateful = json!({"model": "m", "input": "hi", "previous_response_id": "resp_1"});
        let err = decode(WireFormat::OpenAiResponses, &raw(&stateful))
            .err()
            .unwrap();
        assert!(matches!(err, GatewayError::BadRequest(m) if m.contains("previous_response_id")));
        // A null value is not a reference to earlier state.
        let null_id = json!({"model": "m", "input": "hi", "previous_response_id": null});
        assert!(decode(WireFormat::OpenAiResponses, &raw(&null_id)).is_ok());
    }

    #[test]
    fn decode_strips_client_supplied_preservation_state() {
        let body = json!({
            "model": "m",
            "messages": [{"role": "user", "content": "hi"}],
            "metadata": {SWITCHYARD_METADATA_KEY: {"forged": true}, "keep": 1}
        });
        let decoded = decode(WireFormat::OpenAiChat, &raw(&body)).unwrap();
        assert!(
            decoded.body["metadata"]
                .get(SWITCHYARD_METADATA_KEY)
                .is_none()
        );
        assert_eq!(decoded.body["metadata"]["keep"], 1);
    }

    #[test]
    fn request_info_reads_session_and_agent_headers() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-switchyard-session-id",
            HeaderValue::from_static("sess-1"),
        );
        let info = request_info(&headers);
        assert_eq!(info.session_id.as_deref(), Some("sess-1"));

        let mut blank = HeaderMap::new();
        blank.insert("x-switchyard-session-id", HeaderValue::from_static(""));
        assert_eq!(request_info(&blank).session_id, None);
        assert_eq!(request_info(&HeaderMap::new()).session_id, None);
    }

    fn with(pairs: &[(&'static str, &str)]) -> HeaderMap {
        let mut headers = HeaderMap::new();
        for (name, value) in pairs {
            headers.insert(*name, HeaderValue::from_str(value).unwrap());
        }
        headers
    }

    #[test]
    fn session_id_fallbacks_run_after_the_recognized_headers_in_order() {
        let session = |h: &HeaderMap| request_info(h).session_id;
        assert_eq!(
            session(&with(&[("session_id", "abc")])).as_deref(),
            Some("abc")
        );
        assert_eq!(
            session(&with(&[("x-session-affinity", "aff")])).as_deref(),
            Some("aff")
        );
        // Recognized names win over the fallbacks; between fallbacks, `session_id` is first.
        let both = with(&[("x-switchyard-session-id", "one"), ("session_id", "two")]);
        assert_eq!(session(&both).as_deref(), Some("one"));
        let order = with(&[("session_id", "first"), ("x-session-affinity", "second")]);
        assert_eq!(session(&order).as_deref(), Some("first"));
        // A request id is not a session.
        assert_eq!(session(&with(&[("x-client-request-id", "r1")])), None);
    }

    #[test]
    fn a_fallback_id_reaches_the_engine_metadata_too() {
        let info = request_info(&with(&[("session_id", "abc")]));
        assert_eq!(info.metadata.session_id.as_deref(), Some("abc"));
    }

    #[test]
    fn unusable_session_ids_are_ignored_and_the_next_candidate_is_tried() {
        let session = |h: &HeaderMap| request_info(h).session_id;
        let long = "a".repeat(257);
        assert_eq!(session(&with(&[("session_id", &long)])), None);
        assert_eq!(
            session(&with(&[("session_id", &"a".repeat(256))])).map(|s| s.len()),
            Some(256)
        );
        assert_eq!(session(&with(&[("session_id", "has space")])), None);
        assert_eq!(session(&with(&[("session_id", "")])), None);
        let next = with(&[("session_id", "bad id"), ("x-session-affinity", "good")]);
        assert_eq!(session(&next).as_deref(), Some("good"));
        let mut raw = HeaderMap::new();
        raw.insert("session_id", HeaderValue::from_bytes(b"caf\xe9").unwrap());
        assert_eq!(session(&raw), None, "non-ASCII bytes are not a session id");
    }
}
