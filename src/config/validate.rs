//! Cross-field validation of a parsed configuration: references, duplicates, budgets, headers.

use std::collections::BTreeMap;

use std::net::SocketAddr;

use super::schema::{RawConfig, RawKey, RawMetrics, RouteSpec, TailSpec};
use super::{ConfigError, MetricsConfig};

/// Parses `sha256:<64 hex>` (the prefix is optional).
pub(super) fn parse_hash(id: &str, text: &str) -> Result<[u8; 32], ConfigError> {
    parse_hash_at(&format!("keys.{id}.sha256"), text)
}

/// Like [`parse_hash`], naming the config field in the error.
pub(super) fn parse_hash_at(field: &str, text: &str) -> Result<[u8; 32], ConfigError> {
    let hex_part = text.strip_prefix("sha256:").unwrap_or(text);
    let bytes = hex::decode(hex_part)
        .ok()
        .and_then(|b| <[u8; 32]>::try_from(b).ok());
    bytes.ok_or_else(|| {
        invalid(format!(
            "{field} must be `sha256:` followed by 64 hex digits"
        ))
    })
}

/// Checks the `[metrics]` block and parses its credential.
pub(super) fn metrics(
    raw: &RawMetrics,
    main_listen: SocketAddr,
    keys: &BTreeMap<String, RawKey>,
) -> Result<MetricsConfig, ConfigError> {
    if !(1..=1000).contains(&raw.key_label_limit) {
        return Err(invalid("metrics.key_label_limit must be 1 to 1000"));
    }
    let credential = raw
        .bearer_sha256
        .as_deref()
        .map(|text| parse_hash_at("metrics.bearer_sha256", text))
        .transpose()?;
    if raw.enabled {
        if raw.listen == Some(main_listen) {
            return Err(invalid(
                "metrics.listen must differ from listen; leave it out to serve /metrics on the main listener",
            ));
        }
        if raw.listen.is_none() && credential.is_none() {
            return Err(invalid(
                "metrics on the main listener need metrics.bearer_sha256 (a credential only /metrics accepts); \
                 or set metrics.listen to a separate address",
            ));
        }
    }
    if let Some(hash) = credential {
        for (id, key) in keys {
            if parse_hash(id, &key.sha256)? == hash {
                return Err(invalid(format!(
                    "metrics.bearer_sha256 must not be the key `{id}`; the metrics credential is separate from every virtual key"
                )));
            }
        }
    }
    Ok(MetricsConfig {
        enabled: raw.enabled,
        listen: raw.listen,
        credential_sha256: credential,
        key_labels: raw.key_labels,
        key_label_limit: raw.key_label_limit,
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
    if raw.reload_poll_secs > 3600 {
        return Err(invalid("reload_poll_secs must be between 0 and 3600"));
    }
    if !(1..=3600).contains(&raw.quota_poll_secs) {
        return Err(invalid("quota_poll_secs must be between 1 and 3600"));
    }
    if raw.quota_stale_secs < raw.quota_poll_secs || raw.quota_stale_secs > 86_400 {
        return Err(invalid(
            "quota_stale_secs must be at least quota_poll_secs and at most 86400",
        ));
    }
    if !(1..=604_800).contains(&raw.overrides.max_secs) {
        return Err(invalid(
            "overrides.max_secs must be between 1 and 604800 (seven days)",
        ));
    }
    if !(1..=raw.overrides.max_secs).contains(&raw.overrides.default_secs) {
        return Err(invalid(
            "overrides.default_secs must be at least 1 and not above overrides.max_secs",
        ));
    }
    if raw.decision_retention_days > 3650 {
        return Err(invalid(
            "decision_retention_days must be between 0 and 3650",
        ));
    }
    if raw.providers.is_empty() {
        return Err(invalid("at least one provider is required"));
    }
    if raw.targets.is_empty() {
        return Err(invalid("at least one target is required"));
    }
    for (name, provider) in &raw.providers {
        validate_headers(name, &provider.headers)?;
        if let Some(source) = &provider.quota_source {
            validate_quota_source(name, source, &provider.base_url)?;
        }
        if let Some(spec) = &provider.price_source
            && parse_price_source(spec).is_none()
        {
            return Err(invalid(format!(
                "providers.{name}.price_source: `{spec}` is not a known price source; the available source is `models-dev:<provider id>`"
            )));
        }
    }
    validate_budgets(raw)?;
    validate_health(raw)?;
    validate_selectors(raw)?;
    validate_classes(raw)?;
    for (id, target) in &raw.targets {
        if target.endpoints.is_empty() {
            return Err(invalid(format!("target `{id}` has no endpoints")));
        }
        for endpoint in &target.endpoints {
            if endpoint.context_window == Some(0) {
                return Err(invalid(format!(
                    "target `{id}`: context_window must be at least 1 token"
                )));
            }
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
        validate_strict(id, route, raw)?;
    }
    Ok(())
}

/// Headers that carry credentials: a rule can neither match on them nor leak them into routing.
pub const CREDENTIAL_HEADERS: [&str; 5] = [
    "authorization",
    "x-api-key",
    "cookie",
    "set-cookie",
    "proxy-authorization",
];

/// A class label: 1 to 32 lowercase letters, digits, `-` or `_`.
fn valid_class(class: &str) -> bool {
    !class.is_empty()
        && class.len() <= 32
        && class
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
}

pub(super) fn validate_classes(raw: &RawConfig) -> Result<(), ConfigError> {
    for (id, target) in &raw.targets {
        if let Some(class) = &target.class
            && !valid_class(class)
        {
            return Err(invalid(format!(
                "target `{id}`: class `{class}` must be 1 to 32 lowercase letters, digits, - or _ (for example private, public, paid)"
            )));
        }
    }
    Ok(())
}

/// A strict route (`allowed_classes`, ADR 0017) fails closed at load: every class is a valid
/// label, every target it mentions has a class (an unclassed target would be excluded silently),
/// and at least one target is allowed, so the route can serve.
fn validate_strict(id: &str, route: &RouteSpec, raw: &RawConfig) -> Result<(), ConfigError> {
    let Some(allowed) = route.allowed_classes() else {
        return Ok(());
    };
    if allowed.is_empty() {
        return Err(invalid(format!(
            "route `{id}`: allowed_classes must list at least one class"
        )));
    }
    if let Some(bad) = allowed.iter().find(|c| !valid_class(c)) {
        return Err(invalid(format!(
            "route `{id}`: allowed class `{bad}` must be 1 to 32 lowercase letters, digits, - or _"
        )));
    }
    if let Some(tail) = route.tail() {
        for name in &tail.targets {
            let class = raw.targets.get(name).and_then(|t| t.class.as_deref());
            if !class.is_some_and(|c| allowed.iter().any(|a| a == c)) {
                return Err(invalid(format!(
                    "route `{id}`: `then` target `{name}` is not of an allowed class ({}); a tail the route forbids could never be reached",
                    allowed.join(", ")
                )));
            }
        }
    }
    let mut any_allowed = false;
    for name in route.target_refs() {
        match raw.targets.get(name).and_then(|t| t.class.as_deref()) {
            None => {
                return Err(invalid(format!(
                    "route `{id}` sets allowed_classes but target `{name}` has no class; give it one so the route cannot silently drop or use it"
                )));
            }
            Some(class) => any_allowed |= allowed.iter().any(|a| a == class),
        }
    }
    if !any_allowed {
        return Err(invalid(format!(
            "route `{id}`: none of its targets has an allowed class ({}), so it could never serve",
            allowed.join(", ")
        )));
    }
    Ok(())
}

pub(super) fn validate_selectors(raw: &RawConfig) -> Result<(), ConfigError> {
    let mut names = std::collections::BTreeSet::new();
    let mut catch_all_seen: Option<String> = None;
    for (index, rule) in raw.select.iter().enumerate() {
        let label = rule
            .name
            .clone()
            .unwrap_or_else(|| format!("select[{index}]"));
        if !names.insert(label.clone()) {
            return Err(invalid(format!("selector name `{label}` is used twice")));
        }
        if !raw.routes.contains_key(&rule.route) && !raw.targets.contains_key(&rule.route) {
            return Err(invalid(format!(
                "selector `{label}` names unknown route `{}`",
                rule.route
            )));
        }
        if let Some(earlier) = &catch_all_seen {
            return Err(invalid(format!(
                "selector `{label}` can never match: the catch-all `{earlier}` before it matches everything"
            )));
        }
        if let Some(range) = rule.when.prompt_tokens {
            match (range.min, range.max) {
                (None, None) => {
                    return Err(invalid(format!(
                        "selector `{label}`: prompt_tokens needs a `min` or a `max`"
                    )));
                }
                (Some(min), Some(max)) if min > max => {
                    return Err(invalid(format!(
                        "selector `{label}`: prompt_tokens min {min} is above max {max}"
                    )));
                }
                _ => {}
            }
        }
        for name in rule.when.header.keys() {
            let lower = name.to_ascii_lowercase();
            if CREDENTIAL_HEADERS.contains(&lower.as_str()) {
                return Err(invalid(format!(
                    "selector `{label}` cannot match the credential header `{name}`"
                )));
            }
            if http::HeaderName::from_bytes(name.as_bytes()).is_err() {
                return Err(invalid(format!(
                    "selector `{label}` has an invalid header name `{name}`"
                )));
            }
        }
        for name in rule.when.tag.keys() {
            if name.is_empty()
                || !name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
            {
                return Err(invalid(format!(
                    "selector `{label}` has an invalid tag name `{name}` (letters, digits, - and _)"
                )));
            }
        }
        if rule.when.is_catch_all() {
            catch_all_seen = Some(label);
        }
    }
    Ok(())
}

/// One day: far beyond any useful cooldown, small enough that doubling can never overflow.
const MAX_COOLDOWN_SECS: u64 = 86_400;

/// Settings for model discovery (ADR 0022): the provider opt-ins and the `[discovery]` section. Runs before
/// the generated targets are merged, so a hand-written target named like a generated one is caught here
/// instead of silently disabling the generated file.
pub(super) fn discovery(raw: &RawConfig) -> Result<(), ConfigError> {
    for (name, provider) in &raw.providers {
        let Some(spec) = &provider.discover else {
            continue;
        };
        if provider.price_source.is_none() {
            return Err(invalid(format!(
                "providers.{name}.discover needs `price_source` (it names the models.dev provider discovery reads)"
            )));
        }
        super::pools::validate_strengths(&format!("providers.{name}.discover"), &spec.strengths)?;
    }
    let settings = &raw.discovery;
    if settings.enabled && settings.file.is_none() {
        return Err(invalid(
            "discovery.enabled needs `file`: the generated targets file `serve` reads and `models sync` writes",
        ));
    }
    if settings.class.trim().is_empty() || settings.class.len() > 32 {
        return Err(invalid("discovery.class must be 1 to 32 characters"));
    }
    if settings.max_added_per_run == 0 {
        return Err(invalid("discovery.max_added_per_run must be at least 1"));
    }
    if !(0.0..=1.0).contains(&settings.max_removed_fraction) {
        return Err(invalid(
            "discovery.max_removed_fraction must be between 0 and 1",
        ));
    }
    for id in &settings.exclude {
        let provider = id.split_once('/').map(|(provider, _)| provider);
        if !provider.is_some_and(|p| raw.providers.contains_key(p)) {
            return Err(invalid(format!(
                "discovery.exclude: `{id}` must be `<provider>/<model>` with a configured provider"
            )));
        }
    }
    if settings.file.is_some() {
        let prefix = super::discovered::PREFIX;
        if let Some(name) = raw.targets.keys().find(|n| n.starts_with(prefix)) {
            return Err(invalid(format!(
                "target `{name}`: names starting with `{prefix}` are reserved for generated targets when [discovery] file is set"
            )));
        }
    }
    Ok(())
}

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
    if health.max_retry_after_secs == 0 || health.max_retry_after_secs > MAX_COOLDOWN_SECS {
        return Err(invalid(
            "health.max_retry_after_secs must be between 1 and 86400",
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
            if let Some(price) = &endpoint.price {
                let tier_rates = price
                    .tiers
                    .iter()
                    .flat_map(|t| [Some(t.input), Some(t.output), t.cached_input]);
                let valid = [Some(price.input), Some(price.output), price.cached_input]
                    .into_iter()
                    .chain(tier_rates)
                    .flatten()
                    .all(finite_non_negative);
                if !valid {
                    return Err(invalid(format!(
                        "target `{id}`: endpoint prices must not be negative"
                    )));
                }
                let mut previous = 0;
                for tier in &price.tiers {
                    if tier.above_tokens <= previous {
                        return Err(invalid(format!(
                            "target `{id}`: price tiers need `above_tokens` above 0 and in increasing order"
                        )));
                    }
                    previous = tier.above_tokens;
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

/// The quota sources the gateway can poll while serving.
pub const QUOTA_SOURCES: [&str; 1] = ["opencode-go"];

/// A quota source must be known, and its provider must be reachable over https (plain http only
/// to a loopback host, so tests can serve a fixture): the provider's key goes with every poll.
fn validate_quota_source(provider: &str, source: &str, base_url: &str) -> Result<(), ConfigError> {
    if !QUOTA_SOURCES.contains(&source) {
        return Err(invalid(format!(
            "providers.{provider}.quota_source: `{source}` is not a known quota source; the available source is `opencode-go`"
        )));
    }
    let parsed = reqwest::Url::parse(base_url)
        .map_err(|e| invalid(format!("providers.{provider}.base_url is not a URL: {e}")))?;
    let loopback = matches!(parsed.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
    if parsed.scheme() == "https" || (parsed.scheme() == "http" && loopback) {
        Ok(())
    } else {
        Err(invalid(format!(
            "providers.{provider}: a quota_source needs an https base_url, because the provider's key is sent with every poll"
        )))
    }
}

/// The kinds of price source `humpyard prices` can read.
const PRICE_SOURCE_KINDS: [&str; 1] = ["models-dev"];

/// Splits a price source such as `models-dev:opencode-go` into its kind and the source's own
/// provider id; `None` when the kind is unknown or the id is empty.
pub fn parse_price_source(spec: &str) -> Option<(&str, &str)> {
    spec.split_once(':')
        .filter(|(kind, id)| PRICE_SOURCE_KINDS.contains(kind) && !id.is_empty())
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

/// A pool route's tail: only on a pool, at least one target, none of them a pool member or listed twice.
fn validate_tail(
    id: &str,
    tail: &TailSpec,
    members: &[String],
    is_pool: bool,
) -> Result<(), ConfigError> {
    if !is_pool {
        return Err(invalid(format!(
            "route `{id}`: `then` is only for a pool route (one with `pool = {{ ... }}`)"
        )));
    }
    if tail.targets.is_empty() {
        return Err(invalid(format!(
            "route `{id}`: `then.targets` must not be empty"
        )));
    }
    if tail.on.as_ref().is_some_and(Vec::is_empty) {
        return Err(invalid(format!(
            "route `{id}`: `then.on` must list at least one class, or be left out for overflow"
        )));
    }
    for (index, name) in tail.targets.iter().enumerate() {
        if members.contains(name) {
            return Err(invalid(format!(
                "route `{id}`: `then` target `{name}` is also a pool member"
            )));
        }
        if tail.targets[..index].contains(name) {
            return Err(invalid(format!(
                "route `{id}`: `then` target `{name}` is listed twice"
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
        RouteSpec::Passthrough { targets, .. } => non_empty("targets", targets),
        RouteSpec::Fastest {
            targets,
            pool,
            then,
            explore,
            min_samples,
            max_age_secs,
            quality_weight,
            latency_target_ms,
            unknown_quality,
            ..
        } => {
            non_empty("targets", targets)?;
            if let Some(tail) = then {
                validate_tail(id, tail, targets, pool.is_some())?;
            }
            if !(0.0..=1.0).contains(quality_weight) || !(0.0..=1.0).contains(unknown_quality) {
                return Err(invalid(format!(
                    "route `{id}`: `quality_weight` and `unknown_quality` must be between 0 and 1"
                )));
            }
            if *latency_target_ms == 0 {
                return Err(invalid(format!(
                    "route `{id}`: `latency_target_ms` must be at least 1"
                )));
            }
            if !(0.0..=1.0).contains(explore) {
                return Err(invalid(format!(
                    "route `{id}`: `explore` must be between 0 and 1"
                )));
            }
            if *min_samples == 0 || *max_age_secs == 0 {
                return Err(invalid(format!(
                    "route `{id}`: `min_samples` and `max_age_secs` must be at least 1"
                )));
            }
            Ok(())
        }
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
