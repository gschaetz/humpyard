//! The TOML schema: the shapes serde reads, plus the public enums and value types that
//! describe providers, targets, routes and prices.

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RawConfig {
    pub(super) listen: SocketAddr,
    /// Seconds to wait for in-flight requests after a termination signal.
    #[serde(default = "default_shutdown_grace_secs")]
    pub(super) shutdown_grace_secs: u64,
    /// Re-read the config file this often and reload when its content changed (0 = off).
    #[serde(default)]
    pub(super) reload_poll_secs: u64,
    /// Days to keep routing decisions in the ledger database (0 = forever).
    #[serde(default = "default_decision_retention_days")]
    pub(super) decision_retention_days: u64,
    /// How often to read each provider's quota source, in seconds.
    #[serde(default = "default_quota_poll_secs")]
    pub(super) quota_poll_secs: u64,
    /// How old a quota snapshot may be before it counts as unknown, in seconds.
    #[serde(default = "default_quota_stale_secs")]
    pub(super) quota_stale_secs: u64,
    pub(super) providers: BTreeMap<String, RawProvider>,
    pub(super) targets: BTreeMap<String, RawTarget>,
    #[serde(default)]
    pub(super) routes: BTreeMap<String, RouteSpec>,
    #[serde(default)]
    pub(super) keys: BTreeMap<String, RawKey>,
    pub(super) ledger: Option<RawLedger>,
    #[serde(default)]
    pub(super) budget: RawBudget,
    #[serde(default)]
    pub(super) health: RawHealth,
    /// Expiring runtime overrides: how long one lasts by default and at most (`add-runtime-overrides`).
    #[serde(default)]
    pub(super) overrides: RawOverrides,
    /// The Prometheus `/metrics` endpoint (off by default).
    #[serde(default)]
    pub(super) metrics: RawMetrics,
    /// Model discovery settings (ADR 0022): where the generated targets file is and its limits.
    #[serde(default)]
    pub(super) discovery: RawDiscovery,
    /// Ordered rules choosing the route from request facts (`[[select]]`).
    #[serde(default)]
    pub(super) select: Vec<SelectorSpec>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RawDiscovery {
    /// The master switch. Off by default: nothing is read, merged or written until this is `true`, however
    /// the providers are set. Turning it off again stops it at the next reload.
    #[serde(default)]
    pub(super) enabled: bool,
    /// The file `humpyard models sync` writes and `serve` reads. Use an absolute path.
    pub(super) file: Option<std::path::PathBuf>,
    /// The `class` of every generated target.
    #[serde(default = "default_discovery_class")]
    pub(super) class: String,
    /// The most targets one sync run may add.
    #[serde(default = "default_max_added_per_run")]
    pub(super) max_added_per_run: u32,
    /// A sync run that would remove more than this share of the generated targets is refused.
    #[serde(default = "default_max_removed_fraction")]
    pub(super) max_removed_fraction: f64,
    /// `provider/model` ids never to generate.
    #[serde(default)]
    pub(super) exclude: Vec<String>,
}

impl Default for RawDiscovery {
    fn default() -> Self {
        Self {
            enabled: false,
            file: None,
            class: default_discovery_class(),
            max_added_per_run: default_max_added_per_run(),
            max_removed_fraction: default_max_removed_fraction(),
            exclude: Vec::new(),
        }
    }
}

fn default_discovery_class() -> String {
    "public".to_string()
}

fn default_max_added_per_run() -> u32 {
    10
}

fn default_max_removed_fraction() -> f64 {
    0.5
}

fn default_true() -> bool {
    true
}

/// A provider's opt-in to model discovery (ADR 0022).
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DiscoverSpec {
    /// Turn discovery off for this provider without deleting its settings (default true). Its generated
    /// targets stop being loaded at once and are removed at the next sync.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// What new models from this provider are treated as (plus `all`, and `reasoning` when the source says so).
    #[serde(default)]
    pub strengths: Vec<String>,
    /// Require the provider's own `/models` to list a model too (default true). Turn off only for a
    /// provider with no usable `/models`; the guide says this is weaker.
    #[serde(default = "default_true")]
    pub confirm: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RawMetrics {
    #[serde(default)]
    pub(super) enabled: bool,
    /// Serve `/metrics` on this address instead of the main listener.
    pub(super) listen: Option<SocketAddr>,
    /// `sha256:<64 hex>` of the credential that reads `/metrics` on the main listener.
    pub(super) bearer_sha256: Option<String>,
    /// Add a `key` label to the per-request counters.
    #[serde(default)]
    pub(super) key_labels: bool,
    #[serde(default = "default_key_label_limit")]
    pub(super) key_label_limit: usize,
}

fn default_key_label_limit() -> usize {
    100
}

impl Default for RawMetrics {
    fn default() -> Self {
        Self {
            enabled: false,
            listen: None,
            bearer_sha256: None,
            key_labels: false,
            key_label_limit: default_key_label_limit(),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RawHealth {
    #[serde(default = "default_failure_threshold")]
    pub(super) failure_threshold: u32,
    #[serde(default = "default_cooldown_secs")]
    pub(super) cooldown_secs: u64,
    #[serde(default = "default_max_cooldown_secs")]
    pub(super) max_cooldown_secs: u64,
    /// The longest an endpoint is cooled for a retry time its error stated (ADR 0018).
    #[serde(default = "default_max_retry_after_secs")]
    pub(super) max_retry_after_secs: u64,
    /// Leave a target whose endpoints are all cooling down out of a request's plan when another
    /// target remains (ADR 0018). On by default.
    #[serde(default = "default_skip_cold_targets")]
    pub(super) skip_cold_targets: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RawOverrides {
    /// How long an override lasts when its creator names no time, in seconds.
    #[serde(default = "default_override_default_secs")]
    pub(super) default_secs: u64,
    /// The longest an override may last, in seconds (at most seven days).
    #[serde(default = "default_override_max_secs")]
    pub(super) max_secs: u64,
}

impl Default for RawOverrides {
    fn default() -> Self {
        Self {
            default_secs: default_override_default_secs(),
            max_secs: default_override_max_secs(),
        }
    }
}

fn default_override_default_secs() -> u64 {
    4 * 60 * 60
}

fn default_override_max_secs() -> u64 {
    24 * 60 * 60
}

impl Default for RawHealth {
    fn default() -> Self {
        Self {
            failure_threshold: default_failure_threshold(),
            cooldown_secs: default_cooldown_secs(),
            max_cooldown_secs: default_max_cooldown_secs(),
            max_retry_after_secs: default_max_retry_after_secs(),
            skip_cold_targets: default_skip_cold_targets(),
        }
    }
}

fn default_skip_cold_targets() -> bool {
    true
}

fn default_failure_threshold() -> u32 {
    3
}

fn default_cooldown_secs() -> u64 {
    30
}

fn default_max_cooldown_secs() -> u64 {
    300
}

fn default_max_retry_after_secs() -> u64 {
    300
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RawKey {
    /// `sha256:<64 hex>` of the key, from `keygen`. Plaintext keys are never accepted.
    pub(super) sha256: String,
    pub(super) allowed_routes: Option<Vec<String>>,
    #[serde(default)]
    pub(super) over_budget: OverBudget,
    pub(super) daily_usd: Option<f64>,
    pub(super) monthly_usd: Option<f64>,
    pub(super) daily_tokens: Option<u64>,
    pub(super) monthly_tokens: Option<u64>,
    /// May read the effective configuration (`GET /v1/config`); off by default.
    #[serde(default)]
    pub(super) read_config: bool,
    /// May create and remove runtime overrides (`/v1/overrides`); off by default.
    #[serde(default)]
    pub(super) manage_overrides: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RawLedger {
    pub(super) path: PathBuf,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RawBudget {
    #[serde(default = "default_restricted_at")]
    pub(super) restricted_at: f64,
    /// USD per million output tokens above which a target is ineligible while a key is restricted.
    pub(super) restricted_max_output_price: Option<f64>,
}

impl Default for RawBudget {
    fn default() -> Self {
        Self {
            restricted_at: default_restricted_at(),
            restricted_max_output_price: None,
        }
    }
}

fn default_restricted_at() -> f64 {
    0.8
}

/// What happens when a key reaches a limit.
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OverBudget {
    /// Refuse with HTTP 402.
    #[default]
    Block,
    /// Keep serving, from zero-priced targets only.
    FreeOnly,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RawProvider {
    pub(super) base_url: String,
    pub(super) api_key_env: String,
    #[serde(default = "default_timeout_secs")]
    pub(super) timeout_secs: u64,
    /// Extra attempts on the same provider before failing over to the next endpoint.
    #[serde(default = "default_max_retries")]
    pub(super) max_retries: u32,
    /// Extra HTTP headers sent on every call to this provider.
    #[serde(default)]
    pub(super) headers: BTreeMap<String, String>,
    /// Where `humpyard prices` reads this provider's prices, for example `models-dev:opencode-go`.
    /// Only that command uses it; serving never fetches prices.
    pub(super) price_source: Option<String>,
    /// Where the gateway reads this provider's usage windows while serving, for example
    /// `opencode-go`. It is polled with the provider's own key, only at the provider's own host.
    pub(super) quota_source: Option<String>,
    /// Opt in to model discovery (needs `price_source`, which names the models.dev provider).
    #[serde(default)]
    pub(super) discover: Option<DiscoverSpec>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RawTarget {
    pub(super) endpoints: Vec<Endpoint>,
    /// What kind of pool this is, for reporting: conventionally `private`, `public` or `paid`.
    /// A fallback between targets of different classes is flagged on decisions and in `explain`.
    pub(super) class: Option<String>,
    /// Soft labels for what the model is good at (`coding`, `general`, ...). They only decide which
    /// pool routes include the target; they never remove it from a request (ADR 0021).
    #[serde(default)]
    pub(super) strengths: Vec<String>,
    /// How good the model is, from 0 to 1, as the operator states it (a benchmark score they trust). Never
    /// fetched at runtime; may be absent (unknown). Used by a pool's `min_quality` and by a `fastest`
    /// route's `quality_weight` (ADR 0020).
    #[serde(default)]
    pub(super) quality: Option<f64>,
}

fn default_quota_poll_secs() -> u64 {
    60
}

fn default_quota_stale_secs() -> u64 {
    300
}

fn default_decision_retention_days() -> u64 {
    90
}

fn default_shutdown_grace_secs() -> u64 {
    30
}

fn default_timeout_secs() -> u64 {
    120
}

fn default_max_retries() -> u32 {
    1
}

fn default_explore() -> f64 {
    0.1
}

fn default_latency_target_ms() -> u64 {
    2000
}

fn default_unknown_quality() -> f64 {
    0.5
}

fn default_min_samples() -> u32 {
    3
}

fn default_max_age_secs() -> u64 {
    600
}

fn default_confidence() -> f64 {
    0.5
}

/// One provider serving a target, with the model name that provider knows it by.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Endpoint {
    pub provider: String,
    pub model: String,
    /// USD per million tokens. Required on every endpoint when any key has a USD budget.
    #[serde(default)]
    pub price: Option<Price>,
    /// The model's context window in tokens. Undeclared means unknown, which means capable (ADR 0019).
    #[serde(default)]
    pub context_window: Option<u64>,
    /// Whether the model supports tool calls; `false` removes the target from requests that use tools.
    #[serde(default)]
    pub tools: Option<bool>,
    /// Whether the model accepts images; `false` removes the target from requests that carry them.
    #[serde(default)]
    pub vision: Option<bool>,
}

/// Token prices in USD per million tokens (numerically equal to micro-USD per token).
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Price {
    pub input: f64,
    pub output: f64,
    /// Price of input tokens served from the provider's cache; defaults to `input`.
    pub cached_input: Option<f64>,
    /// Higher (or lower) rates that apply once the prompt exceeds a size, ordered by threshold.
    #[serde(default)]
    pub tiers: Vec<PriceTier>,
}

/// Rates that replace the base price when a call's prompt exceeds `above_tokens`.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PriceTier {
    pub above_tokens: u64,
    pub input: f64,
    pub output: f64,
    /// Defaults to this tier's `input`.
    pub cached_input: Option<f64>,
}

/// The rates in force for one call: the base price or one of its tiers.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rates {
    pub input: f64,
    pub output: f64,
    pub cached_input: Option<f64>,
}

impl Rates {
    pub fn is_free(&self) -> bool {
        self.input == 0.0 && self.output == 0.0 && self.cached_input.unwrap_or(0.0) == 0.0
    }
}

impl Price {
    pub const FREE: Price = Price {
        input: 0.0,
        output: 0.0,
        cached_input: None,
        tiers: Vec::new(),
    };

    fn base(&self) -> Rates {
        Rates {
            input: self.input,
            output: self.output,
            cached_input: self.cached_input,
        }
    }

    /// Every rate set this price can charge: the base and each tier.
    fn all_rates(&self) -> impl Iterator<Item = Rates> + '_ {
        std::iter::once(self.base()).chain(self.tiers.iter().map(|t| Rates {
            input: t.input,
            output: t.output,
            cached_input: t.cached_input,
        }))
    }

    /// The rates for a call whose prompt is `prompt_tokens` long: the tier with the highest
    /// threshold the prompt exceeds, or the base price.
    pub fn rates_for(&self, prompt_tokens: u64) -> Rates {
        self.tiers
            .iter()
            .rev()
            .find(|t| prompt_tokens > t.above_tokens)
            .map_or_else(
                || self.base(),
                |t| Rates {
                    input: t.input,
                    output: t.output,
                    cached_input: t.cached_input,
                },
            )
    }

    /// The highest output rate across the base and every tier. Policy and budgets compare this, so
    /// a model that is expensive on long prompts is never mistaken for a cheap one.
    pub fn max_output(&self) -> f64 {
        self.all_rates().map(|r| r.output).fold(0.0, f64::max)
    }

    /// Free at every size: a free base with a paid tier is not free.
    pub fn is_free(&self) -> bool {
        self.all_rates().all(|r| r.is_free())
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum PickerMode {
    EfficientFirst,
    CapableFirst,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ClassifyTrigger {
    #[default]
    EveryRequest,
    UserTurn,
    NewSession,
}

fn default_threshold_step() -> f64 {
    0.1
}

fn default_confirmations() -> u32 {
    2
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ClassifierMode {
    Capability,
    Escalation,
}

/// One routing rule: when every given condition holds, the request follows `route`.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SelectorSpec {
    /// Shown in the `x-humpyard-rule` response header; defaults to `select[<index>]`.
    pub name: Option<String>,
    #[serde(default)]
    pub when: When,
    pub route: String,
}

/// Conditions of a rule, all of which must hold. Text conditions are globs (`*` matches any text).
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct When {
    /// The model (route or target) the client asked for.
    pub model: Option<String>,
    /// The authenticated key's id.
    pub key: Option<String>,
    /// The client's `x-humpyard-profile` header.
    pub profile: Option<String>,
    /// The agent id the client reports.
    pub agent: Option<String>,
    /// The task id the client reports.
    pub task: Option<String>,
    /// Whether the client marked the request as coming from a sub-agent.
    pub subagent: Option<bool>,
    /// Whether the client asked for a streamed response.
    pub stream: Option<bool>,
    /// Estimated prompt size in tokens (a conservative local estimate, inclusive bounds).
    pub prompt_tokens: Option<TokenRange>,
    /// Whether the request defines tools.
    pub tools: Option<bool>,
    /// Whether the request carries images.
    pub images: Option<bool>,
    /// Header name to glob. Credential headers cannot be matched.
    #[serde(default)]
    pub header: BTreeMap<String, String>,
    /// Tag name to glob: the value of the client's `x-humpyard-tag-<name>` header.
    #[serde(default)]
    pub tag: BTreeMap<String, String>,
}

/// Inclusive bounds on a token count; at least one bound is required.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TokenRange {
    pub min: Option<u64>,
    pub max: Option<u64>,
}

impl When {
    /// A rule with no conditions matches every request.
    pub fn is_catch_all(&self) -> bool {
        *self == Self::default()
    }
}

/// Why a call to a target failed, as far as handing the request to the next target is concerned.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FallbackClass {
    /// The prompt does not fit the model's context window.
    Overflow,
    /// HTTP 429.
    RateLimit,
    /// The target did not answer in time.
    Timeout,
    /// HTTP 5xx or 408.
    ServerError,
    /// The connection failed (refused, DNS, reset).
    Connection,
    /// HTTP 403.
    Forbidden,
    /// The endpoint is gone or rejects the key: HTTP 404 or 401, or a 400/404 naming a missing model.
    /// On by default only for `fastest` and pool routes (ADR 0023).
    Unavailable,
}

impl FallbackClass {
    /// The name used in config and reports.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Overflow => "overflow",
            Self::RateLimit => "rate_limit",
            Self::Timeout => "timeout",
            Self::ServerError => "server_error",
            Self::Connection => "connection",
            Self::Forbidden => "forbidden",
            Self::Unavailable => "unavailable",
        }
    }
}

/// Which failures make a route hand the request to its next target. Without a list, every failure falls
/// through, except `unavailable`, which is included only on `fastest` and pool routes (ADR 0023).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FallbackOn {
    classes: Option<std::collections::BTreeSet<FallbackClass>>,
    unavailable_by_default: bool,
}

impl FallbackOn {
    /// Exactly these classes, as the operator listed them.
    pub fn only(classes: &[FallbackClass]) -> Self {
        Self {
            classes: Some(classes.iter().copied().collect()),
            unavailable_by_default: false,
        }
    }

    /// With no list, also fall through on `unavailable` (routes built to try many targets).
    #[must_use]
    pub fn with_unavailable_by_default(mut self) -> Self {
        self.unavailable_by_default = true;
        self
    }

    /// The configured classes, or `None` when no list was given.
    pub fn classes(&self) -> Option<Vec<FallbackClass>> {
        self.classes
            .as_ref()
            .map(|set| set.iter().copied().collect())
    }

    pub fn allows(&self, class: FallbackClass) -> bool {
        match &self.classes {
            Some(set) => set.contains(&class),
            None => class != FallbackClass::Unavailable || self.unavailable_by_default,
        }
    }
}

/// A guarded tail for a pool route: fixed targets tried after every pool member, reached only when a
/// failure of one of the listed classes ends the members (ADR 0024).
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct TailSpec {
    pub targets: Vec<String>,
    /// The failure classes that may reach the tail; `overflow` when absent.
    #[serde(default)]
    pub on: Option<Vec<FallbackClass>>,
}

impl TailSpec {
    /// Which failures of the last pool member may move on to the tail.
    pub fn guard(&self) -> FallbackOn {
        match &self.on {
            Some(classes) => FallbackOn::only(classes),
            None => FallbackOn::only(&[FallbackClass::Overflow]),
        }
    }
}

/// A pool: every target that carries `strength` (and, when set, declares a context window of at least
/// `min_context_window`), resolved when the configuration loads (ADR 0021).
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PoolSpec {
    pub strength: String,
    #[serde(default)]
    pub min_context_window: Option<u64>,
    /// Keep only targets that declare at least this quality (0 to 1). An undeclared quality does not qualify.
    #[serde(default)]
    pub min_quality: Option<f64>,
}

/// A built-in Switchyard algorithm over named targets.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum RouteSpec {
    Passthrough {
        #[serde(default)]
        targets: Vec<String>,
        /// Take every target with a strength instead of listing them (resolved at load).
        #[serde(default)]
        pool: Option<PoolSpec>,
        /// Failures that move on to the next target; all of them when absent.
        #[serde(default)]
        fallback_on: Option<Vec<FallbackClass>>,
        /// Target classes this route may use at all; targets of any other class (and unclassed
        /// targets) are removed before routing, so no failure can reach them (ADR 0017).
        #[serde(default)]
        allowed_classes: Option<Vec<String>>,
    },
    /// Orders the targets that are still eligible by recently observed time to answer, fastest
    /// first, with some exploration (ADR 0020).
    Fastest {
        #[serde(default)]
        targets: Vec<String>,
        /// Take every target with a strength instead of listing them (resolved at load).
        #[serde(default)]
        pool: Option<PoolSpec>,
        /// Probability that a request tries a random eligible target first.
        #[serde(default = "default_explore")]
        explore: f64,
        /// Answered attempts a target needs before its figure is believed.
        #[serde(default = "default_min_samples")]
        min_samples: u32,
        /// A figure older than this counts as unmeasured.
        #[serde(default = "default_max_age_secs")]
        max_age_secs: u64,
        /// How much a target's declared quality counts against its speed when ordering (0 to 1; 0 keeps
        /// the pure speed order).
        #[serde(default)]
        quality_weight: f64,
        /// The answer time that earns full speed marks, in milliseconds.
        #[serde(default = "default_latency_target_ms")]
        latency_target_ms: u64,
        /// The quality assumed for a target that declares none.
        #[serde(default = "default_unknown_quality")]
        unknown_quality: f64,
        /// Makes the exploration draws reproducible.
        seed: Option<u64>,
        /// Failures that move on to the next target; all of them when absent.
        #[serde(default)]
        fallback_on: Option<Vec<FallbackClass>>,
        /// Target classes this route may use at all (ADR 0017).
        #[serde(default)]
        allowed_classes: Option<Vec<String>>,
        /// Targets tried after the pool's members, reached only on the listed failures (ADR 0024).
        #[serde(default)]
        then: Option<TailSpec>,
    },
    Random {
        #[serde(default)]
        targets: Vec<String>,
        /// Take every target with a strength instead of listing them (resolved at load); weights
        /// cannot be combined with it.
        #[serde(default)]
        pool: Option<PoolSpec>,
        weights: Option<Vec<f64>>,
        seed: Option<u64>,
        /// Failures that move on to the next target; all of them when absent.
        #[serde(default)]
        fallback_on: Option<Vec<FallbackClass>>,
        /// Target classes this route may use at all; targets of any other class (and unclassed
        /// targets) are removed before routing, so no failure can reach them (ADR 0017).
        #[serde(default)]
        allowed_classes: Option<Vec<String>>,
    },
    StageRouter {
        efficient: Vec<String>,
        capable: Vec<String>,
        /// Failures that move on to the next target; all of them when absent.
        #[serde(default)]
        fallback_on: Option<Vec<FallbackClass>>,
        /// Target classes this route may use at all; targets of any other class (and unclassed
        /// targets) are removed before routing, so no failure can reach them (ADR 0017).
        #[serde(default)]
        allowed_classes: Option<Vec<String>>,
        #[serde(default = "default_picker_mode")]
        mode: PickerMode,
        #[serde(default = "default_confidence")]
        confidence_threshold: f64,
    },
    LlmClassifier {
        mode: ClassifierMode,
        efficient: Vec<String>,
        capable: Vec<String>,
        judge: Vec<String>,
        /// Failures that move on to the next target; all of them when absent.
        #[serde(default)]
        fallback_on: Option<Vec<FallbackClass>>,
        /// Target classes this route may use at all; targets of any other class (and unclassed
        /// targets) are removed before routing, so no failure can reach them (ADR 0017).
        #[serde(default)]
        allowed_classes: Option<Vec<String>>,
        /// Capability mode: lowest solve probability that routes a supported task to `efficient`.
        #[serde(default = "default_confidence")]
        base_threshold: f64,
        /// Capability mode: threshold added per capability-boundary step.
        #[serde(default = "default_threshold_step")]
        threshold_step: f64,
        /// Capability mode: how often the judge re-decides a session's target.
        #[serde(default)]
        classify_trigger: ClassifyTrigger,
        /// Escalation mode: consecutive escalate verdicts before the session latches to `capable`.
        #[serde(default = "default_confirmations")]
        confirmations: u32,
    },
}

fn default_picker_mode() -> PickerMode {
    PickerMode::EfficientFirst
}

impl RouteSpec {
    /// Which failures hand the request to the next target.
    pub fn fallback_on(&self) -> FallbackOn {
        let listed = match self {
            Self::Passthrough { fallback_on, .. }
            | Self::Fastest { fallback_on, .. }
            | Self::Random { fallback_on, .. }
            | Self::StageRouter { fallback_on, .. }
            | Self::LlmClassifier { fallback_on, .. } => fallback_on.as_deref(),
        };
        match listed {
            // A list is taken exactly as written.
            Some(classes) => FallbackOn::only(classes),
            // No list: every class falls through; `unavailable` too on routes built to try many targets
            // that may be gone (`fastest` and pool routes), and not on any other (ADR 0023).
            None if self.tries_many_unreliable_targets() => {
                FallbackOn::default().with_unavailable_by_default()
            }
            None => FallbackOn::default(),
        }
    }

    /// A `fastest` route, or any route that is a pool.
    fn tries_many_unreliable_targets(&self) -> bool {
        match self {
            Self::Fastest { .. } => true,
            Self::Passthrough { pool, .. } | Self::Random { pool, .. } => pool.is_some(),
            Self::StageRouter { .. } | Self::LlmClassifier { .. } => false,
        }
    }

    /// The classes this route is restricted to, if it is strict (ADR 0017).
    pub fn allowed_classes(&self) -> Option<&[String]> {
        match self {
            Self::Passthrough {
                allowed_classes, ..
            }
            | Self::Fastest {
                allowed_classes, ..
            }
            | Self::Random {
                allowed_classes, ..
            }
            | Self::StageRouter {
                allowed_classes, ..
            }
            | Self::LlmClassifier {
                allowed_classes, ..
            } => allowed_classes.as_deref(),
        }
    }

    /// For a route that can be a pool: its target list (filled in when the pool resolves), its pool
    /// if it has one, and whether it carries per-target weights.
    pub(super) fn pool_parts_mut(&mut self) -> Option<(&mut Vec<String>, Option<&PoolSpec>, bool)> {
        match self {
            Self::Passthrough { targets, pool, .. } | Self::Fastest { targets, pool, .. } => {
                Some((targets, pool.as_ref(), false))
            }
            Self::Random {
                targets,
                pool,
                weights,
                ..
            } => Some((targets, pool.as_ref(), weights.is_some())),
            Self::StageRouter { .. } | Self::LlmClassifier { .. } => None,
        }
    }

    /// The guarded tail of a pool route, if it has one.
    pub fn tail(&self) -> Option<&TailSpec> {
        match self {
            Self::Fastest { then, .. } => then.as_ref(),
            _ => None,
        }
    }

    /// Every target name the route mentions.
    pub fn target_refs(&self) -> Vec<&str> {
        let lists: Vec<&Vec<String>> = match self {
            Self::Fastest { targets, then, .. } => {
                let mut lists = vec![targets];
                lists.extend(then.as_ref().map(|t| &t.targets));
                lists
            }
            Self::Passthrough { targets, .. } | Self::Random { targets, .. } => vec![targets],
            Self::StageRouter {
                efficient, capable, ..
            } => vec![efficient, capable],
            Self::LlmClassifier {
                efficient,
                capable,
                judge,
                ..
            } => vec![efficient, capable, judge],
        };
        lists.into_iter().flatten().map(String::as_str).collect()
    }
}
