//! Cross-field validation of a parsed configuration: references, duplicates, budgets, headers.

use std::collections::BTreeMap;

use super::ConfigError;
use super::schema::{RawConfig, RouteSpec};

/// Parses `sha256:<64 hex>` (the prefix is optional).
pub(super) fn parse_hash(id: &str, text: &str) -> Result<[u8; 32], ConfigError> {
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

pub(super) fn invalid(message: impl Into<String>) -> ConfigError {
    ConfigError::Invalid(message.into())
}

/// A key definition must carry only a hash; any field that looks like the key itself is an error
/// that names the key id.
pub(super) fn reject_plaintext_keys(text: &str) -> Result<(), ConfigError> {
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

pub(super) fn validate(raw: &RawConfig) -> Result<(), ConfigError> {
    if raw.shutdown_grace_secs > 3600 {
        return Err(invalid("shutdown_grace_secs must be between 0 and 3600"));
    }
    if raw.providers.is_empty() {
        return Err(invalid("at least one provider is required"));
    }
    if raw.targets.is_empty() {
        return Err(invalid("at least one target is required"));
    }
    for (name, provider) in &raw.providers {
        validate_headers(name, &provider.headers)?;
    }
    validate_budgets(raw)?;
    validate_health(raw)?;
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
        validate_route(id, route)?;
    }
    Ok(())
}

/// One day: far beyond any useful cooldown, small enough that doubling can never overflow.
const MAX_COOLDOWN_SECS: u64 = 86_400;

pub(super) fn validate_health(raw: &RawConfig) -> Result<(), ConfigError> {
    let health = &raw.health;
    if health.failure_threshold == 0 {
        return Ok(()); // disabled: the cooldowns are unused
    }
    if health.cooldown_secs == 0 || health.cooldown_secs > MAX_COOLDOWN_SECS {
        return Err(invalid("health.cooldown_secs must be between 1 and 86400"));
    }
    if health.max_cooldown_secs < health.cooldown_secs
        || health.max_cooldown_secs > MAX_COOLDOWN_SECS
    {
        return Err(invalid(
            "health.max_cooldown_secs must be between cooldown_secs and 86400",
        ));
    }
    Ok(())
}

pub(super) fn validate_budgets(raw: &RawConfig) -> Result<(), ConfigError> {
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
pub(super) fn validate_headers(
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

pub(super) fn validate_route(id: &str, route: &RouteSpec) -> Result<(), ConfigError> {
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
