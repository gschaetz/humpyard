//! Gateway configuration: providers, targets and routes from one TOML file, with API keys read
//! from the environment.

mod schema;
#[cfg(test)]
mod tests;
mod validate;

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use schema::RawConfig;
pub use schema::{
    ClassifierMode, ClassifyTrigger, Endpoint, OverBudget, PickerMode, Price, RouteSpec,
};

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

#[derive(Clone, Debug)]
pub struct Config {
    pub listen: SocketAddr,
    /// How long to wait for in-flight requests after SIGINT/SIGTERM (0 to 3600 seconds).
    pub shutdown_grace_secs: u64,
    pub providers: BTreeMap<String, Provider>,
    pub targets: BTreeMap<String, Vec<Endpoint>>,
    pub routes: BTreeMap<String, RouteSpec>,
    /// Virtual keys by id. Empty means the gateway is open and unbudgeted.
    pub keys: BTreeMap<String, KeyConfig>,
    pub ledger: Option<PathBuf>,
    pub budget: BudgetConfig,
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
}

impl std::fmt::Debug for Provider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Provider")
            .field("base_url", &self.base_url)
            .field("api_key", &"<redacted>")
            .field("timeout_secs", &self.timeout_secs)
            .field("max_retries", &self.max_retries)
            .field("header_names", &self.headers.keys().collect::<Vec<_>>())
            .finish()
    }
}

impl Config {
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        let text = std::fs::read_to_string(path).map_err(|source| ConfigError::Read {
            path: path.display().to_string(),
            source,
        })?;
        Self::from_toml(&text, |name| std::env::var(name).ok())
    }

    /// `env` looks up environment variables so tests do not touch the process environment.
    pub fn from_toml(
        text: &str,
        env: impl Fn(&str) -> Option<String>,
    ) -> Result<Self, ConfigError> {
        validate::reject_plaintext_keys(text)?;
        let raw: RawConfig = toml::from_str(text).map_err(|e| {
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
        validate::validate(&raw)?;

        let mut providers = BTreeMap::new();
        for (name, p) in raw.providers {
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
                },
            );
        }
        Ok(Self {
            listen: raw.listen,
            shutdown_grace_secs: raw.shutdown_grace_secs,
            providers,
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
            ledger: raw.ledger.map(|l| l.path),
            budget: BudgetConfig {
                restricted_at: raw.budget.restricted_at,
                restricted_max_output_price: raw.budget.restricted_max_output_price,
            },
        })
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
