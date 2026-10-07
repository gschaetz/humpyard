//! Gateway configuration: providers, targets and routes from one TOML file, with API keys read
//! from the environment.

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawConfig {
    listen: SocketAddr,
    providers: BTreeMap<String, RawProvider>,
    targets: BTreeMap<String, RawTarget>,
    #[serde(default)]
    routes: BTreeMap<String, RouteSpec>,
    #[serde(default)]
    keys: BTreeMap<String, RawKey>,
    ledger: Option<RawLedger>,
    #[serde(default)]
    budget: RawBudget,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawKey {
    /// `sha256:<64 hex>` of the key, from `keygen`. Plaintext keys are never accepted.
    sha256: String,
    allowed_routes: Option<Vec<String>>,
    #[serde(default)]
    over_budget: OverBudget,
    daily_usd: Option<f64>,
    monthly_usd: Option<f64>,
    daily_tokens: Option<u64>,
    monthly_tokens: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawLedger {
    path: PathBuf,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawBudget {
    #[serde(default = "default_restricted_at")]
    restricted_at: f64,
    /// USD per million output tokens above which a target is ineligible while a key is restricted.
    restricted_max_output_price: Option<f64>,
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
struct RawProvider {
    base_url: String,
    api_key_env: String,
    #[serde(default = "default_timeout_secs")]
    timeout_secs: u64,
    /// Extra attempts on the same provider before failing over to the next endpoint.
    #[serde(default = "default_max_retries")]
    max_retries: u32,
    /// Extra HTTP headers sent on every call to this provider.
    #[serde(default)]
    headers: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawTarget {
    endpoints: Vec<Endpoint>,
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

/// A built-in Switchyard algorithm over named targets.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum RouteSpec {
    Passthrough {
        targets: Vec<String>,
    },
    Random {
        targets: Vec<String>,
        weights: Option<Vec<f64>>,
        seed: Option<u64>,
    },
    StageRouter {
        efficient: Vec<String>,
        capable: Vec<String>,
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
    /// Every target name the route mentions.
    fn target_refs(&self) -> Vec<&str> {
        let lists: Vec<&Vec<String>> = match self {
            Self::Passthrough { targets } | Self::Random { targets, .. } => vec![targets],
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

/// Parses `sha256:<64 hex>` (the prefix is optional).
fn parse_hash(id: &str, text: &str) -> Result<[u8; 32], ConfigError> {
    let hex_part = text.strip_prefix("sha256:").unwrap_or(text);
    let bytes = hex::decode(hex_part)
        .ok()
        .and_then(|b| <[u8; 32]>::try_from(b).ok());
    bytes.ok_or_else(|| {
        invalid(format!(
            "keys.{id}.sha256 must be `sha256:` followed by 64 hex digits"
        ))
    })
}

fn invalid(message: impl Into<String>) -> ConfigError {
    ConfigError::Invalid(message.into())
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
        Self::reject_plaintext_keys(text)?;
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
        Self::validate(&raw)?;

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
                    let sha256 = parse_hash(&id, &key.sha256)?;
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

    /// A key definition must carry only a hash; any field that looks like the key itself is an error
    /// that names the key id.
    fn reject_plaintext_keys(text: &str) -> Result<(), ConfigError> {
        let Ok(table) = text.parse::<toml::Table>() else {
            return Ok(()); // the typed parse reports the syntax error
        };
        let Some(keys) = table.get("keys").and_then(toml::Value::as_table) else {
            return Ok(());
        };
        for (id, key) in keys {
            let Some(fields) = key.as_table() else {
                continue;
            };
            for field in ["key", "api_key", "secret", "token", "plaintext"] {
                if fields.contains_key(field) {
                    return Err(invalid(format!(
                        "keys.{id}: plaintext keys are not allowed; set `sha256` to the hash printed by `keygen`"
                    )));
                }
            }
        }
        Ok(())
    }

    fn validate(raw: &RawConfig) -> Result<(), ConfigError> {
        if raw.providers.is_empty() {
            return Err(invalid("at least one provider is required"));
        }
        if raw.targets.is_empty() {
            return Err(invalid("at least one target is required"));
        }
        for (name, provider) in &raw.providers {
            Self::validate_headers(name, &provider.headers)?;
        }
        Self::validate_budgets(raw)?;
        for (id, target) in &raw.targets {
            if target.endpoints.is_empty() {
                return Err(invalid(format!("target `{id}` has no endpoints")));
            }
            for endpoint in &target.endpoints {
                if !raw.providers.contains_key(&endpoint.provider) {
                    return Err(invalid(format!(
                        "target `{id}` names unknown provider `{}`",
                        endpoint.provider
                    )));
                }
            }
        }
        for (id, route) in &raw.routes {
            if raw.targets.contains_key(id) {
                return Err(invalid(format!(
                    "`{id}` is defined as both a route and a target"
                )));
            }
            for name in route.target_refs() {
                if !raw.targets.contains_key(name) {
                    return Err(invalid(format!(
                        "route `{id}` names unknown target `{name}`"
                    )));
                }
            }
            Self::validate_route(id, route)?;
        }
        Ok(())
    }

    fn validate_budgets(raw: &RawConfig) -> Result<(), ConfigError> {
        let finite_non_negative = |v: f64| v.is_finite() && v >= 0.0;
        if !(0.0..=1.0).contains(&raw.budget.restricted_at) {
            return Err(invalid("budget.restricted_at must be between 0 and 1"));
        }
        if raw
            .budget
            .restricted_max_output_price
            .is_some_and(|p| !finite_non_negative(p))
        {
            return Err(invalid(
                "budget.restricted_max_output_price must not be negative",
            ));
        }
        for (id, target) in &raw.targets {
            for endpoint in &target.endpoints {
                if let Some(price) = endpoint.price {
                    let valid = [Some(price.input), Some(price.output), price.cached_input]
                        .into_iter()
                        .flatten()
                        .all(finite_non_negative);
                    if !valid {
                        return Err(invalid(format!(
                            "target `{id}`: endpoint prices must not be negative"
                        )));
                    }
                }
            }
        }
        let mut hashes: Vec<[u8; 32]> = Vec::new();
        let mut has_limits = false;
        let mut has_usd = false;
        for (id, key) in &raw.keys {
            for (field, value) in [
                ("daily_usd", key.daily_usd),
                ("monthly_usd", key.monthly_usd),
            ] {
                if let Some(v) = value {
                    if !finite_non_negative(v) {
                        return Err(invalid(format!("keys.{id}.{field} must not be negative")));
                    }
                    has_usd = true;
                }
            }
            has_limits |= key.daily_usd.is_some()
                || key.monthly_usd.is_some()
                || key.daily_tokens.is_some()
                || key.monthly_tokens.is_some();
            let hash = parse_hash(id, &key.sha256)?;
            if hashes.contains(&hash) {
                return Err(invalid(format!("keys.{id}: duplicate key hash")));
            }
            hashes.push(hash);
            for route in key.allowed_routes.iter().flatten() {
                if !raw.targets.contains_key(route) && !raw.routes.contains_key(route) {
                    return Err(invalid(format!(
                        "keys.{id}.allowed_routes names unknown route or target `{route}`"
                    )));
                }
            }
        }
        if has_limits && raw.ledger.is_none() {
            return Err(invalid(
                "budgets need a ledger: add a `[ledger]` section with a `path`",
            ));
        }
        if has_usd {
            for (id, target) in &raw.targets {
                for endpoint in &target.endpoints {
                    if endpoint.price.is_none() {
                        return Err(invalid(format!(
                            "target `{id}`: endpoint {}/{} needs a `price` because a key has a USD budget (use 0 for free)",
                            endpoint.provider, endpoint.model
                        )));
                    }
                }
            }
        }
        Ok(())
    }

    /// Headers must be valid HTTP and must not carry credentials: keys come from `api_key_env`.
    fn validate_headers(
        provider: &str,
        headers: &BTreeMap<String, String>,
    ) -> Result<(), ConfigError> {
        const AUTH_HEADERS: [&str; 3] = ["authorization", "x-api-key", "proxy-authorization"];
        for (name, value) in headers {
            if http::HeaderName::from_bytes(name.as_bytes()).is_err()
                || http::HeaderValue::from_str(value).is_err()
            {
                return Err(invalid(format!(
                    "providers.{provider}.headers: `{name}` is not a valid HTTP header"
                )));
            }
            if AUTH_HEADERS.contains(&name.to_ascii_lowercase().as_str()) {
                return Err(invalid(format!(
                    "providers.{provider}.headers: `{name}` would override authentication; use `api_key_env`"
                )));
            }
        }
        Ok(())
    }

    fn validate_route(id: &str, route: &RouteSpec) -> Result<(), ConfigError> {
        let non_empty = |label: &str, list: &[String]| {
            if list.is_empty() {
                Err(invalid(format!(
                    "route `{id}`: `{label}` must not be empty"
                )))
            } else {
                Ok(())
            }
        };
        match route {
            RouteSpec::Passthrough { targets } => non_empty("targets", targets),
            RouteSpec::Random {
                targets, weights, ..
            } => {
                non_empty("targets", targets)?;
                match weights {
                    Some(w) if w.len() != targets.len() => Err(invalid(format!(
                        "route `{id}`: `weights` must have one entry per target"
                    ))),
                    _ => Ok(()),
                }
            }
            RouteSpec::StageRouter {
                efficient,
                capable,
                confidence_threshold,
                ..
            } => {
                non_empty("efficient", efficient)?;
                non_empty("capable", capable)?;
                if !(0.0..=1.0).contains(confidence_threshold) {
                    return Err(invalid(format!(
                        "route `{id}`: `confidence_threshold` must be between 0 and 1"
                    )));
                }
                Ok(())
            }
            RouteSpec::LlmClassifier {
                efficient,
                capable,
                judge,
                base_threshold,
                threshold_step,
                confirmations,
                ..
            } => {
                non_empty("efficient", efficient)?;
                non_empty("capable", capable)?;
                non_empty("judge", judge)?;
                if !(0.0..=1.0).contains(base_threshold) || *threshold_step < 0.0 {
                    return Err(invalid(format!(
                        "route `{id}`: `base_threshold` must be between 0 and 1 and `threshold_step` must not be negative"
                    )));
                }
                if *confirmations == 0 {
                    return Err(invalid(format!(
                        "route `{id}`: `confirmations` must be at least 1"
                    )));
                }
                Ok(())
            }
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    const VALID: &str = r#"
listen = "127.0.0.1:8080"

[providers.groq]
base_url = "https://api.example.com/v1/"
api_key_env = "GROQ_KEY"

[providers.local]
base_url = "http://localhost:11434/v1"
api_key_env = "LOCAL_KEY"
timeout_secs = 30

[targets.fast]
endpoints = [
  { provider = "groq", model = "llama-3.3-70b" },
  { provider = "local", model = "llama3.3" },
]

[targets.smart]
endpoints = [{ provider = "groq", model = "big-model" }]

[routes.auto]
type = "stage_router"
efficient = ["fast"]
capable = ["smart"]
"#;

    fn env(name: &str) -> Option<String> {
        matches!(name, "GROQ_KEY" | "LOCAL_KEY").then(|| format!("secret-{name}"))
    }

    fn err(text: &str) -> String {
        Config::from_toml(text, env).unwrap_err().to_string()
    }

    #[test]
    fn valid_config_loads() {
        let config = Config::from_toml(VALID, env).unwrap();
        assert_eq!(
            config.providers["groq"].base_url,
            "https://api.example.com/v1"
        );
        assert_eq!(config.providers["groq"].api_key, "secret-GROQ_KEY");
        assert_eq!(config.providers["groq"].timeout_secs, 120);
        assert_eq!(config.providers["groq"].max_retries, 1);
        assert_eq!(config.providers["local"].timeout_secs, 30);
        assert_eq!(config.targets["fast"].len(), 2);
        assert_eq!(config.targets["fast"][1].model, "llama3.3");
        assert!(matches!(
            config.routes["auto"],
            RouteSpec::StageRouter {
                mode: PickerMode::EfficientFirst,
                ..
            }
        ));
        assert_eq!(config.model_names(), ["auto", "fast", "smart"]);
    }

    #[test]
    fn unknown_key_is_rejected() {
        assert!(err(&format!("bogus = 1\n{VALID}")).contains("bogus"));
    }

    #[test]
    fn inline_key_is_rejected() {
        let text = VALID.replacen("api_key_env = \"GROQ_KEY\"", "api_key = \"sk-x\"", 1);
        assert!(err(&text).contains("inline `api_key`"));
    }

    #[test]
    fn missing_env_var_names_provider_and_variable() {
        let message = Config::from_toml(VALID, |name| (name == "LOCAL_KEY").then(|| "k".into()))
            .unwrap_err()
            .to_string();
        assert!(
            message.contains("groq") && message.contains("GROQ_KEY"),
            "{message}"
        );
    }

    #[test]
    fn unknown_provider_in_endpoint() {
        let text = VALID.replace("provider = \"local\"", "provider = \"nope\"");
        assert!(err(&text).contains("unknown provider `nope`"));
    }

    #[test]
    fn unknown_target_in_route() {
        let text = VALID.replace("capable = [\"smart\"]", "capable = [\"ghost\"]");
        assert!(err(&text).contains("unknown target `ghost`"));
    }

    #[test]
    fn route_and_target_sharing_a_name() {
        let text = VALID.replace("[routes.auto]", "[routes.fast]");
        assert!(err(&text).contains("both a route and a target"));
    }

    #[test]
    fn duplicate_provider_names_are_rejected() {
        let text = format!("{VALID}\n[providers.groq]\nbase_url = \"x\"\napi_key_env = \"y\"\n");
        assert!(err(&text).contains("groq"));
    }

    #[test]
    fn target_without_endpoints() {
        let text = VALID.replace(
            "endpoints = [{ provider = \"groq\", model = \"big-model\" }]",
            "endpoints = []",
        );
        assert!(err(&text).contains("no endpoints"));
    }

    #[test]
    fn random_weights_must_match_targets() {
        let text = format!(
            "{VALID}\n[routes.split]\ntype = \"random\"\ntargets = [\"fast\", \"smart\"]\nweights = [1.0]\n"
        );
        assert!(err(&text).contains("weights"));
    }

    #[test]
    fn confidence_threshold_range() {
        let text = VALID.replace(
            "capable = [\"smart\"]",
            "capable = [\"smart\"]\nconfidence_threshold = 1.5",
        );
        assert!(err(&text).contains("confidence_threshold"));
    }

    #[test]
    fn every_route_type_parses() {
        let text = format!(
            r#"{VALID}
[routes.one]
type = "passthrough"
targets = ["fast"]
[routes.split]
type = "random"
targets = ["fast", "smart"]
weights = [0.7, 0.3]
seed = 7
[routes.judged]
type = "llm_classifier"
mode = "escalation"
efficient = ["fast"]
capable = ["smart"]
judge = ["fast"]
"#
        );
        let config = Config::from_toml(&text, env).unwrap();
        assert_eq!(config.routes.len(), 4);
    }

    #[test]
    fn provider_headers_load() {
        let text = VALID.replacen(
            "api_key_env = \"GROQ_KEY\"",
            "api_key_env = \"GROQ_KEY\"\nheaders = { \"x-app\" = \"conductor\" }",
            1,
        );
        let config = Config::from_toml(&text, env).unwrap();
        assert_eq!(config.providers["groq"].headers["x-app"], "conductor");
        assert!(config.providers["local"].headers.is_empty());
    }

    #[test]
    fn authentication_headers_are_rejected() {
        for name in ["authorization", "X-Api-Key"] {
            let text = VALID.replacen(
                "api_key_env = \"GROQ_KEY\"",
                &format!("api_key_env = \"GROQ_KEY\"\nheaders = {{ \"{name}\" = \"x\" }}"),
                1,
            );
            let message = err(&text);
            assert!(
                message.contains("groq") && message.contains(name),
                "{message}"
            );
        }
    }

    #[test]
    fn invalid_header_names_are_rejected() {
        let text = VALID.replacen(
            "api_key_env = \"GROQ_KEY\"",
            "api_key_env = \"GROQ_KEY\"\nheaders = { \"bad name\" = \"x\" }",
            1,
        );
        let message = err(&text);
        assert!(
            message.contains("groq") && message.contains("bad name"),
            "{message}"
        );
    }

    const HASH_A: &str = "sha256:0000000000000000000000000000000000000000000000000000000000000001";
    const HASH_B: &str = "sha256:0000000000000000000000000000000000000000000000000000000000000002";

    /// VALID with every endpoint priced, a ledger, and the given extra TOML appended.
    fn budgeted(extra: &str) -> String {
        let priced = VALID
            .replace(
                "{ provider = \"groq\", model = \"llama-3.3-70b\" }",
                "{ provider = \"groq\", model = \"llama-3.3-70b\", price = { input = 0.2, output = 0.8 } }",
            )
            .replace(
                "{ provider = \"local\", model = \"llama3.3\" }",
                "{ provider = \"local\", model = \"llama3.3\", price = { input = 0.0, output = 0.0 } }",
            )
            .replace(
                "[{ provider = \"groq\", model = \"big-model\" }]",
                "[{ provider = \"groq\", model = \"big-model\", price = { input = 3.0, output = 15.0, cached_input = 0.3 } }]",
            );
        format!("{priced}\n[ledger]\npath = \"humpyard.db\"\n{extra}")
    }

    #[test]
    fn valid_budget_config_loads() {
        let text = budgeted(&format!(
            "[keys.alice]\nsha256 = \"{HASH_A}\"\nallowed_routes = [\"auto\"]\ndaily_usd = 5.0\nmonthly_tokens = 1000000\nover_budget = \"free_only\"\n[budget]\nrestricted_at = 0.9\nrestricted_max_output_price = 2.0\n"
        ));
        let config = Config::from_toml(&text, env).unwrap();
        let alice = &config.keys["alice"];
        assert_eq!(alice.sha256[31], 1);
        assert_eq!(alice.over_budget, OverBudget::FreeOnly);
        assert_eq!(alice.limits.daily_usd, Some(5.0));
        assert_eq!(alice.limits.monthly_tokens, Some(1_000_000));
        assert_eq!(config.ledger.as_deref(), Some(Path::new("humpyard.db")));
        assert_eq!(config.budget.restricted_at, 0.9);
        assert_eq!(
            config.targets["smart"][0].price.unwrap().cached_input,
            Some(0.3)
        );
    }

    #[test]
    fn config_without_keys_is_open_with_defaults() {
        let config = Config::from_toml(VALID, env).unwrap();
        assert!(config.keys.is_empty() && config.ledger.is_none());
        assert_eq!(config.budget.restricted_at, 0.8);
    }

    #[test]
    fn plaintext_key_is_rejected_naming_the_key() {
        for field in ["key", "api_key", "secret"] {
            let text = budgeted(&format!(
                "[keys.alice]\nsha256 = \"{HASH_A}\"\n{field} = \"sk-plain\"\n"
            ));
            let message = err(&text);
            assert!(
                message.contains("keys.alice") && message.contains("plaintext"),
                "{message}"
            );
        }
    }

    #[test]
    fn malformed_or_duplicate_hashes_are_rejected() {
        let bad = budgeted("[keys.alice]\nsha256 = \"sha256:abc\"\n");
        assert!(err(&bad).contains("keys.alice.sha256"));
        let dup = budgeted(&format!(
            "[keys.alice]\nsha256 = \"{HASH_A}\"\n[keys.bob]\nsha256 = \"{HASH_A}\"\n"
        ));
        assert!(err(&dup).contains("duplicate"));
        let ok = budgeted(&format!(
            "[keys.alice]\nsha256 = \"{HASH_A}\"\n[keys.bob]\nsha256 = \"{HASH_B}\"\n"
        ));
        assert!(Config::from_toml(&ok, env).is_ok());
    }

    #[test]
    fn budget_without_ledger_is_rejected() {
        let text = format!("{VALID}\n[keys.alice]\nsha256 = \"{HASH_A}\"\ndaily_tokens = 10\n");
        let message = err(&text);
        assert!(message.contains("ledger"), "{message}");
    }

    #[test]
    fn key_without_limits_needs_no_ledger_or_prices() {
        let text = format!("{VALID}\n[keys.alice]\nsha256 = \"{HASH_A}\"\n");
        assert!(Config::from_toml(&text, env).is_ok());
    }

    #[test]
    fn usd_budget_requires_every_endpoint_priced() {
        let text = format!(
            "{VALID}\n[ledger]\npath = \"x.db\"\n[keys.alice]\nsha256 = \"{HASH_A}\"\ndaily_usd = 1.0\n"
        );
        let message = err(&text);
        assert!(
            message.contains("needs a `price`") && message.contains("groq/llama-3.3-70b"),
            "{message}"
        );
    }

    #[test]
    fn token_budget_does_not_require_prices() {
        let text = format!(
            "{VALID}\n[ledger]\npath = \"x.db\"\n[keys.alice]\nsha256 = \"{HASH_A}\"\ndaily_tokens = 5\n"
        );
        assert!(Config::from_toml(&text, env).is_ok());
    }

    #[test]
    fn invalid_limits_and_fractions_are_rejected() {
        let negative = budgeted(&format!(
            "[keys.alice]\nsha256 = \"{HASH_A}\"\ndaily_usd = -1.0\n"
        ));
        assert!(err(&negative).contains("keys.alice.daily_usd"));
        let fraction = budgeted("[budget]\nrestricted_at = 1.5\n");
        assert!(err(&fraction).contains("restricted_at"));
        let price = budgeted("[budget]\nrestricted_max_output_price = -2.0\n");
        assert!(err(&price).contains("restricted_max_output_price"));
    }

    #[test]
    fn allowlist_must_name_known_routes() {
        let text = budgeted(&format!(
            "[keys.alice]\nsha256 = \"{HASH_A}\"\nallowed_routes = [\"ghost\"]\n"
        ));
        assert!(err(&text).contains("unknown route or target `ghost`"));
    }

    #[test]
    fn debug_output_hides_keys() {
        let config = Config::from_toml(VALID, env).unwrap();
        assert!(!format!("{config:?}").contains("secret-"));
    }
}
