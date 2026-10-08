//! Configuration tests: parsing, validation, budgets, keys and headers.

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

#[test]
fn shutdown_grace_defaults_to_thirty_seconds_and_accepts_the_valid_range() {
    assert_eq!(
        Config::from_toml(VALID, env).unwrap().shutdown_grace_secs,
        30
    );
    for secs in [0, 5, 3600] {
        let text = format!("shutdown_grace_secs = {secs}\n{VALID}");
        assert_eq!(
            Config::from_toml(&text, env).unwrap().shutdown_grace_secs,
            secs
        );
    }
}

#[test]
fn shutdown_grace_above_an_hour_is_rejected_naming_the_key() {
    let text = format!("shutdown_grace_secs = 3601\n{VALID}");
    let message = err(&text);
    assert!(message.contains("shutdown_grace_secs"), "{message}");
}

#[test]
fn health_defaults_and_overrides() {
    let defaults = Config::from_toml(VALID, env).unwrap().health;
    assert_eq!(
        (
            defaults.failure_threshold,
            defaults.cooldown_secs,
            defaults.max_cooldown_secs
        ),
        (3, 30, 300)
    );
    let text = format!(
        "{VALID}\n[health]\nfailure_threshold = 5\ncooldown_secs = 2\nmax_cooldown_secs = 9\n"
    );
    let health = Config::from_toml(&text, env).unwrap().health;
    assert_eq!(
        (
            health.failure_threshold,
            health.cooldown_secs,
            health.max_cooldown_secs
        ),
        (5, 2, 9)
    );
}

#[test]
fn health_values_are_validated_by_key_and_ignored_when_disabled() {
    for (section, key) in [
        ("cooldown_secs = 0", "cooldown_secs"),
        (
            "cooldown_secs = 86401\nmax_cooldown_secs = 86400",
            "cooldown_secs",
        ),
        (
            "cooldown_secs = 60\nmax_cooldown_secs = 30",
            "max_cooldown_secs",
        ),
        ("max_cooldown_secs = 86401", "max_cooldown_secs"),
    ] {
        let message = err(&format!("{VALID}\n[health]\n{section}\n"));
        assert!(message.contains(key), "{section}: {message}");
    }
    let disabled = format!("{VALID}\n[health]\nfailure_threshold = 0\ncooldown_secs = 0\n");
    assert!(Config::from_toml(&disabled, env).is_ok());
    assert!(err(&format!("{VALID}\n[health]\nbogus = 1\n")).contains("bogus"));
}
