//! Gateway configuration: a TOML file plus the upstream API key read from the environment.

use std::net::SocketAddr;
use std::path::Path;

use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawConfig {
    listen: SocketAddr,
    upstream: RawUpstream,
    models: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawUpstream {
    base_url: String,
    api_key_env: String,
    #[serde(default = "default_timeout_secs")]
    timeout_secs: u64,
}

fn default_timeout_secs() -> u64 {
    120
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
    #[error("environment variable {0} (upstream.api_key_env) is not set")]
    MissingKey(String),
    #[error("`models` must list at least one model")]
    NoModels,
}

#[derive(Clone, Debug)]
pub struct Config {
    pub listen: SocketAddr,
    pub upstream: Upstream,
    pub models: Vec<String>,
}

#[derive(Clone)]
pub struct Upstream {
    pub base_url: String,
    pub api_key: String,
    pub timeout_secs: u64,
}

impl std::fmt::Debug for Upstream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Upstream")
            .field("base_url", &self.base_url)
            .field("api_key", &"<redacted>")
            .field("timeout_secs", &self.timeout_secs)
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
        if raw.models.is_empty() {
            return Err(ConfigError::NoModels);
        }
        let api_key = env(&raw.upstream.api_key_env)
            .filter(|key| !key.is_empty())
            .ok_or_else(|| ConfigError::MissingKey(raw.upstream.api_key_env.clone()))?;
        Ok(Self {
            listen: raw.listen,
            upstream: Upstream {
                base_url: raw.upstream.base_url.trim_end_matches('/').to_string(),
                api_key,
                timeout_secs: raw.upstream.timeout_secs,
            },
            models: raw.models,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID: &str = r#"
listen = "127.0.0.1:8080"
models = ["fast"]
[upstream]
base_url = "https://api.example.com/v1/"
api_key_env = "UP_KEY"
"#;

    fn env(name: &str) -> Option<String> {
        (name == "UP_KEY").then(|| "secret".to_string())
    }

    #[test]
    fn valid_config_loads() {
        let config = Config::from_toml(VALID, env).unwrap();
        assert_eq!(config.upstream.base_url, "https://api.example.com/v1");
        assert_eq!(config.upstream.api_key, "secret");
        assert_eq!(config.upstream.timeout_secs, 120);
    }

    #[test]
    fn unknown_key_is_rejected() {
        let text = format!("bogus = 1\n{VALID}");
        let err = Config::from_toml(&text, env).unwrap_err().to_string();
        assert!(err.contains("bogus"), "{err}");
    }

    #[test]
    fn inline_key_is_rejected() {
        let text = VALID.replace("api_key_env = \"UP_KEY\"", "api_key = \"sk-x\"");
        let err = Config::from_toml(&text, env).unwrap_err().to_string();
        assert!(err.contains("inline `api_key`"), "{err}");
    }

    #[test]
    fn missing_env_var_names_the_variable() {
        let err = Config::from_toml(VALID, |_| None).unwrap_err().to_string();
        assert!(err.contains("UP_KEY"), "{err}");
    }

    #[test]
    fn debug_output_hides_the_key() {
        let config = Config::from_toml(VALID, env).unwrap();
        assert!(!format!("{:?}", config.upstream).contains("secret"));
    }
}
