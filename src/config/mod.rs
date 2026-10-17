//! Gateway configuration: providers, targets and routes from one TOML file, with API keys read
//! from the environment.

mod discovered;
mod pools;
mod schema;
#[cfg(test)]
mod tests;
mod validate;

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

pub use discovered::DiscoveryState;
pub use pools::{LeftOut, PoolReport};
use schema::RawConfig;
pub use schema::{
    ClassifierMode, ClassifyTrigger, DiscoverSpec, Endpoint, FallbackClass, FallbackOn, OverBudget,
    PickerMode, PoolSpec, Price, PriceTier, Rates, RouteSpec, SelectorSpec, TailSpec, TokenRange,
    When,
};
use validate::invalid;
pub use validate::{CREDENTIAL_HEADERS, QUOTA_SOURCES, parse_price_source};

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("cannot read {path}: {source}")]
    Read {
        path: String,
        source: std::io::Error,
    },
    #[error("invalid config: {0}")]
    Parse(String),
    #[error("environment variable {var} (providers.{provider}.api_key_env) is not set")]
    MissingKey { provider: String, var: String },
    #[error("invalid config: {0}")]
    Invalid(String),
}

/// The `/metrics` endpoint (ADR 0016). Needs a restart to change.
#[derive(Clone, Debug, Default)]
pub struct MetricsConfig {
    pub enabled: bool,
    /// A separate address to serve on; `None` serves on the main listener.
    pub listen: Option<SocketAddr>,
    /// SHA-256 of the credential that reads `/metrics` on the main listener.
    pub credential_sha256: Option<[u8; 32]>,
    pub key_labels: bool,
    pub key_label_limit: usize,
}

#[derive(Clone, Debug)]
pub struct Config {
    pub listen: SocketAddr,
    /// How long to wait for in-flight requests after SIGINT/SIGTERM (0 to 3600 seconds).
    pub shutdown_grace_secs: u64,
    /// How often to check the config file for changes and reload; 0 turns polling off (SIGHUP
    /// still works). Needs a restart to change.
    pub reload_poll_secs: u64,
    /// Days to keep routing decisions (0 = forever). Needs a restart to change.
    pub decision_retention_days: u64,
    /// Seconds between reads of each provider's quota source (1 to 3600).
    pub quota_poll_secs: u64,
    /// Seconds after which a quota snapshot counts as unknown (at least `quota_poll_secs`).
    pub quota_stale_secs: u64,
    pub providers: BTreeMap<String, Provider>,
    pub targets: BTreeMap<String, Vec<Endpoint>>,
    /// Optional class label per target (`private`, `public`, `paid`, or your own).
    pub target_classes: BTreeMap<String, String>,
    /// Soft labels per target (only targets that declare some); they decide pool membership and
    /// nothing else (ADR 0021).
    pub target_strengths: BTreeMap<String, Vec<String>>,
    /// The quality (0 to 1) each target declares; targets that declare none are absent.
    pub target_quality: BTreeMap<String, f64>,
    /// How each pool route resolved when this configuration loaded.
    pub pools: BTreeMap<String, PoolReport>,
    /// The `[discovery]` section.
    pub discovery: DiscoverySettings,
    /// What became of the generated targets file at this load (ADR 0022).
    pub discovery_state: DiscoveryState,
    /// Names of the targets that came from the generated file (`auto-*`).
    pub generated_targets: std::collections::BTreeSet<String>,
    pub routes: BTreeMap<String, RouteSpec>,
    /// Virtual keys by id. Empty means the gateway is open and unbudgeted.
    pub keys: BTreeMap<String, KeyConfig>,
    pub ledger: Option<PathBuf>,
    pub budget: BudgetConfig,
    pub health: HealthConfig,
    pub metrics: MetricsConfig,
    /// Routing rules, first match wins.
    pub select: Vec<SelectorSpec>,
}

/// Endpoint circuit-breaker settings. A `failure_threshold` of 0 disables health tracking.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HealthConfig {
    pub failure_threshold: u32,
    pub cooldown_secs: u64,
    pub max_cooldown_secs: u64,
    /// The longest an endpoint is cooled because of a retry time its error stated.
    pub max_retry_after_secs: u64,
    /// Skip a target whose endpoints are all cooling down when another target remains (ADR 0018).
    pub skip_cold_targets: bool,
}

#[derive(Clone, Debug)]
pub struct KeyConfig {
    pub sha256: [u8; 32],
    pub allowed_routes: Option<Vec<String>>,
    pub over_budget: OverBudget,
    pub limits: Limits,
}

/// Per-period limits; `None` means unlimited.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Limits {
    pub daily_usd: Option<f64>,
    pub monthly_usd: Option<f64>,
    pub daily_tokens: Option<u64>,
    pub monthly_tokens: Option<u64>,
}

impl Limits {
    pub fn is_empty(&self) -> bool {
        *self == Limits::default()
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BudgetConfig {
    pub restricted_at: f64,
    pub restricted_max_output_price: Option<f64>,
}

#[derive(Clone)]
pub struct Provider {
    pub base_url: String,
    pub api_key: String,
    pub timeout_secs: u64,
    pub max_retries: u32,
    pub headers: BTreeMap<String, String>,
    /// A price source for `humpyard prices` (for example `models-dev:opencode-go`), if any.
    pub price_source: Option<String>,
    /// Where to read this provider's usage windows while serving (for example `opencode-go`).
    pub quota_source: Option<String>,
    /// Opted in to model discovery (ADR 0022).
    pub discover: Option<DiscoverSpec>,
}

/// The `[discovery]` section: where `humpyard models sync` writes and `serve` reads, and the limits on a run.
#[derive(Clone, Debug, PartialEq)]
pub struct DiscoverySettings {
    /// The master switch (default false).
    pub enabled: bool,
    pub file: Option<PathBuf>,
    pub class: String,
    pub max_added_per_run: u32,
    pub max_removed_fraction: f64,
    pub exclude: Vec<String>,
}

impl std::fmt::Debug for Provider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Provider")
            .field("base_url", &self.base_url)
            .field("api_key", &"<redacted>")
            .field("timeout_secs", &self.timeout_secs)
            .field("max_retries", &self.max_retries)
            .field("header_names", &self.headers.keys().collect::<Vec<_>>())
            .field("price_source", &self.price_source)
            .field("quota_source", &self.quota_source)
            .field("discover", &self.discover)
            .finish()
    }
}

/// Reads a file's text, if it can be read (the generated targets file).
type FileReader<'a> = &'a dyn Fn(&Path) -> Option<String>;

/// The providers with their keys resolved from the environment.
fn build_providers(
    raw: BTreeMap<String, schema::RawProvider>,
    env: &dyn Fn(&str) -> Option<String>,
) -> Result<BTreeMap<String, Provider>, ConfigError> {
    let mut providers = BTreeMap::new();
    for (name, p) in raw {
        let api_key = env(&p.api_key_env)
            .filter(|key| !key.is_empty())
            .ok_or_else(|| ConfigError::MissingKey {
                provider: name.clone(),
                var: p.api_key_env.clone(),
            })?;
        providers.insert(
            name,
            Provider {
                base_url: p.base_url.trim_end_matches('/').to_string(),
                api_key,
                timeout_secs: p.timeout_secs,
                max_retries: p.max_retries,
                headers: p.headers,
                price_source: p.price_source,
                quota_source: p.quota_source,
                discover: p.discover,
            },
        );
    }
    Ok(providers)
}

impl Config {
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        let text = std::fs::read_to_string(path).map_err(|source| ConfigError::Read {
            path: path.display().to_string(),
            source,
        })?;
        Self::from_toml_with(&text, |name| std::env::var(name).ok(), &|file| {
            std::fs::read_to_string(file).ok()
        })
    }

    /// `env` looks up environment variables so tests do not touch the process environment. The generated
    /// targets file (if `[discovery] file` names one) is not read; use [`Config::from_toml_with`] for that.
    pub fn from_toml(
        text: &str,
        env: impl Fn(&str) -> Option<String>,
    ) -> Result<Self, ConfigError> {
        Self::from_toml_with(text, env, &|_| None)
    }

    /// Like [`Config::from_toml`], also reading the generated targets file through `read` and merging it
    /// (ADR 0022). That file can never be the reason a configuration fails to load: if merging it makes
    /// the configuration invalid, it is dropped and the main configuration loads alone.
    pub fn from_toml_with(
        text: &str,
        env: impl Fn(&str) -> Option<String>,
        read: FileReader<'_>,
    ) -> Result<Self, ConfigError> {
        match Self::assemble(text, &env, Some(read)) {
            Ok(config) => Ok(config),
            Err(first) => {
                let mut alone = Self::assemble(text, &env, None)?;
                alone.discovery_state = DiscoveryState::Ignored(format!(
                    "merging it made the configuration invalid: {first}"
                ));
                Ok(alone)
            }
        }
    }

    fn assemble(
        text: &str,
        env: &dyn Fn(&str) -> Option<String>,
        read: Option<FileReader<'_>>,
    ) -> Result<Self, ConfigError> {
        validate::reject_plaintext_keys(text)?;
        let mut raw: RawConfig = toml::from_str(text).map_err(|e| {
            let message = e.to_string();
            if message.contains("unknown field `api_key`") {
                ConfigError::Parse(
                    "inline `api_key` is not allowed; set `api_key_env` to an environment variable name"
                        .into(),
                )
            } else {
                ConfigError::Parse(message)
            }
        })?;
        // Generated targets are merged first so pools (by strength) see them (ADR 0022); strengths are
        // checked and pool routes get their targets before the rest is validated, so a pool is validated
        // like a hand-written route (ADR 0021).
        validate::discovery(&raw)?;
        let (generated_targets, discovery_state) = match read {
            Some(read) => discovered::load(&mut raw, read),
            None => (std::collections::BTreeSet::new(), DiscoveryState::Off),
        };
        let pools = pools::resolve(&mut raw)?;
        validate::validate(&raw)?;

        let providers = build_providers(raw.providers, env)?;
        let metrics = validate::metrics(&raw.metrics, raw.listen, &raw.keys)?;
        Ok(Self {
            listen: raw.listen,
            shutdown_grace_secs: raw.shutdown_grace_secs,
            reload_poll_secs: raw.reload_poll_secs,
            quota_poll_secs: raw.quota_poll_secs,
            quota_stale_secs: raw.quota_stale_secs,
            decision_retention_days: raw.decision_retention_days,
            providers,
            target_classes: raw
                .targets
                .iter()
                .filter_map(|(id, t)| Some((id.clone(), t.class.clone()?)))
                .collect(),
            target_strengths: raw
                .targets
                .iter()
                .filter(|(_, t)| !t.strengths.is_empty())
                .map(|(id, t)| (id.clone(), t.strengths.clone()))
                .collect(),
            target_quality: raw
                .targets
                .iter()
                .filter_map(|(id, t)| Some((id.clone(), t.quality?)))
                .collect(),
            pools,
            discovery: DiscoverySettings {
                enabled: raw.discovery.enabled,
                file: raw.discovery.file,
                class: raw.discovery.class,
                max_added_per_run: raw.discovery.max_added_per_run,
                max_removed_fraction: raw.discovery.max_removed_fraction,
                exclude: raw.discovery.exclude,
            },
            discovery_state,
            generated_targets,
            targets: raw
                .targets
                .into_iter()
                .map(|(id, t)| (id, t.endpoints))
                .collect(),
            routes: raw.routes,
            keys: raw
                .keys
                .into_iter()
                .map(|(id, key)| {
                    let sha256 = validate::parse_hash(&id, &key.sha256)?;
                    Ok((
                        id,
                        KeyConfig {
                            sha256,
                            allowed_routes: key.allowed_routes,
                            over_budget: key.over_budget,
                            limits: Limits {
                                daily_usd: key.daily_usd,
                                monthly_usd: key.monthly_usd,
                                daily_tokens: key.daily_tokens,
                                monthly_tokens: key.monthly_tokens,
                            },
                        },
                    ))
                })
                .collect::<Result<_, ConfigError>>()?,
            metrics,
            ledger: raw.ledger.map(|l| l.path),
            select: raw.select,
            health: HealthConfig {
                failure_threshold: raw.health.failure_threshold,
                cooldown_secs: raw.health.cooldown_secs,
                max_cooldown_secs: raw.health.max_cooldown_secs,
                max_retry_after_secs: raw.health.max_retry_after_secs,
                skip_cold_targets: raw.health.skip_cold_targets,
            },
            budget: BudgetConfig {
                restricted_at: raw.budget.restricted_at,
                restricted_max_output_price: raw.budget.restricted_max_output_price,
            },
        })
    }

    /// Whether any endpoint declares a context window, so the gateway must estimate prompt sizes
    /// even when no selector rule needs it (ADR 0019).
    pub fn declares_context_windows(&self) -> bool {
        self.targets
            .values()
            .flatten()
            .any(|endpoint| endpoint.context_window.is_some())
    }

    /// Route and target names clients may request, routes first.
    pub fn model_names(&self) -> Vec<&str> {
        self.routes
            .keys()
            .chain(self.targets.keys())
            .map(String::as_str)
            .collect()
    }
}
