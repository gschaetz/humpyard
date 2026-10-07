//! Gateway configuration: providers, targets and routes from one TOML file, with API keys read
//! from the environment.

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::path::Path;

use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawConfig {
    listen: SocketAddr,
    providers: BTreeMap<String, RawProvider>,
    targets: BTreeMap<String, RawTarget>,
    #[serde(default)]
    routes: BTreeMap<String, RouteSpec>,
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
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum PickerMode {
    EfficientFirst,
    CapableFirst,
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
}

#[derive(Clone)]
pub struct Provider {
    pub base_url: String,
    pub api_key: String,
    pub timeout_secs: u64,
    pub max_retries: u32,
}

impl std::fmt::Debug for Provider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Provider")
            .field("base_url", &self.base_url)
            .field("api_key", &"<redacted>")
            .field("timeout_secs", &self.timeout_secs)
            .field("max_retries", &self.max_retries)
            .finish()
    }
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
        })
    }

    fn validate(raw: &RawConfig) -> Result<(), ConfigError> {
        if raw.providers.is_empty() {
            return Err(invalid("at least one provider is required"));
        }
        if raw.targets.is_empty() {
            return Err(invalid("at least one target is required"));
        }
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
                ..
            } => {
                non_empty("efficient", efficient)?;
                non_empty("capable", capable)?;
                non_empty("judge", judge)
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
    fn debug_output_hides_keys() {
        let config = Config::from_toml(VALID, env).unwrap();
        assert!(!format!("{config:?}").contains("secret-"));
    }
}
