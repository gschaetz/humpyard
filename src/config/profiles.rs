//! Key profiles (ADR 0025, `add-key-profiles`). A profile bundles the narrowable settings of a key: its routes,
//! budgets, over-budget mode and rate limits. A key that names one inherits every setting it leaves unset and
//! may set the others only to something no wider. The merge is done once, when the configuration loads or
//! reloads, and written into the raw key, so the rest of the gateway reads an ordinary key and nothing is
//! decided per request. A profile never carries a privilege (`read_config`, `manage_overrides`).

use std::fmt::Display;

use super::schema::{OverBudget, RawConfig, RawKey, RawProfile};
use super::validate::{MAX_REQUESTS_PER_MINUTE, key_limit};
use super::{ConfigError, invalid};

/// Checks every profile, then resolves every key that names one against it.
pub(super) fn resolve(raw: &mut RawConfig) -> Result<(), ConfigError> {
    for (id, profile) in &raw.profiles {
        check(id, profile, raw)?;
    }
    for (id, key) in &mut raw.keys {
        let Some(name) = key.profile.clone() else {
            continue;
        };
        let profile = raw.profiles.get(&name).ok_or_else(|| {
            invalid(format!(
                "keys.{id}.profile names unknown profile `{name}`; define it as [profiles.{name}]"
            ))
        })?;
        narrow_key(id, &name, key, profile)?;
    }
    Ok(())
}

/// A profile's values are valid by the rules a key's are, naming the profile.
fn check(id: &str, profile: &RawProfile, raw: &RawConfig) -> Result<(), ConfigError> {
    for (field, value) in [
        ("daily_usd", profile.daily_usd),
        ("monthly_usd", profile.monthly_usd),
    ] {
        if value.is_some_and(|v| !(v.is_finite() && v >= 0.0)) {
            return Err(invalid(format!(
                "profiles.{id}.{field} must not be negative"
            )));
        }
    }
    if profile
        .max_request_usd
        .is_some_and(|v| !(v.is_finite() && v > 0.0))
    {
        return Err(invalid(format!(
            "profiles.{id}.max_request_usd must be a positive number of dollars"
        )));
    }
    let owner = format!("profiles.{id}");
    limits(
        &owner,
        profile.max_requests_per_minute.as_ref(),
        profile.max_concurrent.as_ref(),
    )?;
    for route in profile.allowed_routes.iter().flatten() {
        if !raw.targets.contains_key(route) && !raw.routes.contains_key(route) {
            return Err(invalid(format!(
                "profiles.{id}.allowed_routes names unknown route or target `{route}`"
            )));
        }
    }
    Ok(())
}

/// Validates the two limit values of `owner`.
fn limits(
    owner: &str,
    rate: Option<&toml::Value>,
    concurrent: Option<&toml::Value>,
) -> Result<(Option<u32>, Option<u32>), ConfigError> {
    Ok((
        key_limit(
            owner,
            "max_requests_per_minute",
            rate,
            MAX_REQUESTS_PER_MINUTE,
        )?,
        key_limit(owner, "max_concurrent", concurrent, u32::MAX.into())?,
    ))
}

/// Gives `key` the profile's settings it lacks and refuses any it sets wider than the profile's.
fn narrow_key(
    id: &str,
    name: &str,
    key: &mut RawKey,
    profile: &RawProfile,
) -> Result<(), ConfigError> {
    let wider = |field: &str, detail: String| {
        invalid(format!(
            "keys.{id}.{field} is wider than profile `{name}` allows ({detail})"
        ))
    };
    for (field, own, bound) in [
        ("daily_usd", &mut key.daily_usd, profile.daily_usd),
        ("monthly_usd", &mut key.monthly_usd, profile.monthly_usd),
        (
            "max_request_usd",
            &mut key.max_request_usd,
            profile.max_request_usd,
        ),
    ] {
        narrow(own, bound).map_err(|(k, p)| wider(field, format!("{k} against {p}")))?;
    }
    for (field, own, bound) in [
        ("daily_tokens", &mut key.daily_tokens, profile.daily_tokens),
        (
            "monthly_tokens",
            &mut key.monthly_tokens,
            profile.monthly_tokens,
        ),
    ] {
        narrow(own, bound).map_err(|(k, p)| wider(field, format!("{k} against {p}")))?;
    }
    narrow_limits(id, name, key, profile)?;
    narrow_routes(key, profile).map_err(|route| {
        wider(
            "allowed_routes",
            format!("`{route}` is not in the profile's routes"),
        )
    })?;
    narrow_over_budget(key, profile)
        .map_err(|()| wider("over_budget", "free_only under a block profile".to_string()))
}

/// Sets `own` from `bound` when unset; refuses (returning both values) when `own` is larger.
fn narrow<T: PartialOrd + Copy + Display>(
    own: &mut Option<T>,
    bound: Option<T>,
) -> Result<(), (T, T)> {
    match (*own, bound) {
        (Some(k), Some(p)) if k > p => Err((k, p)),
        (None, Some(p)) => {
            *own = Some(p);
            Ok(())
        }
        _ => Ok(()),
    }
}

/// The rate and concurrency limits are validated, compared as numbers and written back.
fn narrow_limits(
    id: &str,
    name: &str,
    key: &mut RawKey,
    profile: &RawProfile,
) -> Result<(), ConfigError> {
    let (own_rate, own_conc) = limits(
        &format!("keys.{id}"),
        key.max_requests_per_minute.as_ref(),
        key.max_concurrent.as_ref(),
    )?;
    let (bound_rate, bound_conc) = limits(
        &format!("profiles.{name}"),
        profile.max_requests_per_minute.as_ref(),
        profile.max_concurrent.as_ref(),
    )?;
    for (field, own, bound, slot) in [
        (
            "max_requests_per_minute",
            own_rate,
            bound_rate,
            &mut key.max_requests_per_minute,
        ),
        (
            "max_concurrent",
            own_conc,
            bound_conc,
            &mut key.max_concurrent,
        ),
    ] {
        let mut value = own;
        narrow(&mut value, bound).map_err(|(k, p)| {
            invalid(format!(
                "keys.{id}.{field} is wider than profile `{name}` allows ({k} against {p})"
            ))
        })?;
        *slot = value.map(|n| toml::Value::Integer(i64::from(n)));
    }
    Ok(())
}

/// A key lists only routes its profile lists; unset inherits the profile's list.
fn narrow_routes(key: &mut RawKey, profile: &RawProfile) -> Result<(), String> {
    match (&key.allowed_routes, &profile.allowed_routes) {
        (Some(own), Some(bound)) => match own.iter().find(|r| !bound.contains(r)) {
            Some(route) => Err(route.clone()),
            None => Ok(()),
        },
        (None, Some(bound)) => {
            key.allowed_routes = Some(bound.clone());
            Ok(())
        }
        _ => Ok(()),
    }
}

/// `block` may replace `free_only`, never the reverse; unset inherits.
fn narrow_over_budget(key: &mut RawKey, profile: &RawProfile) -> Result<(), ()> {
    match (key.over_budget, profile.over_budget) {
        (Some(OverBudget::FreeOnly), Some(OverBudget::Block)) => Err(()),
        (None, Some(bound)) => {
            key.over_budget = Some(bound);
            Ok(())
        }
        _ => Ok(()),
    }
}
