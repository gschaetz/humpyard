//! The TOML schema: the shapes serde reads, plus the public enums and value types that
//! describe providers, targets, routes and prices.

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::path::PathBuf;

use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RawConfig {
    pub(super) listen: SocketAddr,
    /// Seconds to wait for in-flight requests after a termination signal.
    #[serde(default = "default_shutdown_grace_secs")]
    pub(super) shutdown_grace_secs: u64,
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
    /// Ordered rules choosing the route from request facts (`[[select]]`).
    #[serde(default)]
    pub(super) select: Vec<SelectorSpec>,
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
}

impl Default for RawHealth {
    fn default() -> Self {
        Self {
            failure_threshold: default_failure_threshold(),
            cooldown_secs: default_cooldown_secs(),
            max_cooldown_secs: default_max_cooldown_secs(),
        }
    }
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
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RawTarget {
    pub(super) endpoints: Vec<Endpoint>,
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

fn default_confidence() -> f64 {
    0.5
}

/// One provider serving a target, with the model name that provider knows it by.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Endpoint {
    pub provider: String,
    pub model: String,
    /// USD per million tokens. Required on every endpoint when any key has a USD budget.
    #[serde(default)]
    pub price: Option<Price>,
}

/// Token prices in USD per million tokens (numerically equal to micro-USD per token).
#[derive(Clone, Copy, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Price {
    pub input: f64,
    pub output: f64,
    /// Price of input tokens served from the provider's cache; defaults to `input`.
    pub cached_input: Option<f64>,
}

impl Price {
    pub const FREE: Price = Price {
        input: 0.0,
        output: 0.0,
        cached_input: None,
    };

    pub fn is_free(&self) -> bool {
        self.input == 0.0 && self.output == 0.0 && self.cached_input.unwrap_or(0.0) == 0.0
    }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum PickerMode {
    EfficientFirst,
    CapableFirst,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq)]
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

#[derive(Clone, Copy, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ClassifierMode {
    Capability,
    Escalation,
}

/// One routing rule: when every given condition holds, the request follows `route`.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SelectorSpec {
    /// Shown in the `x-humpyard-rule` response header; defaults to `select[<index>]`.
    pub name: Option<String>,
    #[serde(default)]
    pub when: When,
    pub route: String,
}

/// Conditions of a rule, all of which must hold. Text conditions are globs (`*` matches any text).
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
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
    /// Header name to glob. Credential headers cannot be matched.
    #[serde(default)]
    pub header: BTreeMap<String, String>,
    /// Tag name to glob: the value of the client's `x-humpyard-tag-<name>` header.
    #[serde(default)]
    pub tag: BTreeMap<String, String>,
}

impl When {
    /// A rule with no conditions matches every request.
    pub fn is_catch_all(&self) -> bool {
        *self == Self::default()
    }
}

/// Why a call to a target failed, as far as handing the request to the next target is concerned.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
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
}

/// Which failures make a route hand the request to its next target. The default is all of them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FallbackOn(Option<std::collections::BTreeSet<FallbackClass>>);

impl FallbackOn {
    pub fn only(classes: &[FallbackClass]) -> Self {
        Self(Some(classes.iter().copied().collect()))
    }

    pub fn allows(&self, class: FallbackClass) -> bool {
        self.0.as_ref().is_none_or(|set| set.contains(&class))
    }
}

/// A built-in Switchyard algorithm over named targets.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum RouteSpec {
    Passthrough {
        targets: Vec<String>,
        /// Failures that move on to the next target; all of them when absent.
        #[serde(default)]
        fallback_on: Option<Vec<FallbackClass>>,
    },
    Random {
        targets: Vec<String>,
        weights: Option<Vec<f64>>,
        seed: Option<u64>,
        /// Failures that move on to the next target; all of them when absent.
        #[serde(default)]
        fallback_on: Option<Vec<FallbackClass>>,
    },
    StageRouter {
        efficient: Vec<String>,
        capable: Vec<String>,
        /// Failures that move on to the next target; all of them when absent.
        #[serde(default)]
        fallback_on: Option<Vec<FallbackClass>>,
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
        match self {
            Self::Passthrough { fallback_on, .. }
            | Self::Random { fallback_on, .. }
            | Self::StageRouter { fallback_on, .. }
            | Self::LlmClassifier { fallback_on, .. } => fallback_on
                .as_deref()
                .map_or_else(FallbackOn::default, FallbackOn::only),
        }
    }

    /// Every target name the route mentions.
    pub(super) fn target_refs(&self) -> Vec<&str> {
        let lists: Vec<&Vec<String>> = match self {
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
