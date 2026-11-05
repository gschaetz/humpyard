//! `humpyard models check --retired`: configured endpoints whose model the provider no longer lists.
//!
//! Each provider's own `/models` list is the authority for "is this model still offered to this key": it is
//! read the way discovery reads it (the key goes only to the provider's own host, no redirects, bounded size and
//! time) and compared with every configured endpoint, hand-written or generated. The check only reports. A
//! provider can serve a model it does not list, so a report is evidence for the operator, not a verdict.
//!
//! A provider whose list is missing is one of three things, told apart because they mean different things: it has
//! no list (nothing to compare with), the check could not reach it (a laptop that sleeps, a dead host), or it
//! answered with something unusable (a rejected key, a body that is not a list). Only the last makes the check
//! incomplete in a way worth a non-zero exit.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Value, json};

use crate::config::Config;
use crate::discover::{DiscoverError, ListFailure, fetch_provider_list};

/// A configured endpoint whose model its provider's list does not include.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Retired {
    pub target: String,
    pub provider: String,
    pub model: String,
}

/// What a provider's own list gave.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Listing {
    /// The model ids the provider lists.
    Listed(BTreeSet<String>),
    /// The provider has no model list (its `/models` answers 404, 405 or 501), with the reason.
    NoList(String),
    /// The check could not reach the provider (connection failed, timed out, 502, 503 or 504), with the reason.
    Unreachable(String),
    /// The provider answered but the answer cannot be used, with the reason.
    Unusable(String),
    /// The provider has no key, so it was not asked.
    NoKey,
}

impl From<ListFailure> for Listing {
    fn from(failure: ListFailure) -> Self {
        match failure {
            ListFailure::NoList(reason) => Self::NoList(reason),
            ListFailure::Unreachable(reason) => Self::Unreachable(reason),
            ListFailure::Unusable(reason) => Self::Unusable(reason),
        }
    }
}

/// The result of one check.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Report {
    /// Configured endpoints compared with a provider's list.
    pub checked: usize,
    pub retired: Vec<Retired>,
    /// Providers that have no model list, with the reason. They do not affect the exit code.
    pub no_list: Vec<(String, String)>,
    /// Providers the check could not reach, with the reason. They do not affect the exit code.
    pub unreachable: Vec<(String, String)>,
    /// Providers that answered with something unusable, with the reason.
    pub unusable: Vec<(String, String)>,
    /// Providers that were not asked because they have no key.
    pub skipped: Vec<String>,
}

impl Report {
    /// 2 when a provider answered with something unusable (the check is incomplete for a reason worth looking
    /// at), else 1 when it reports a retired endpoint, else 0. A provider with no list, or one that could not be
    /// reached, is in the report but does not change the code.
    #[must_use]
    pub fn exit_code(&self) -> u8 {
        if self.unusable.is_empty() {
            u8::from(!self.retired.is_empty())
        } else {
            2
        }
    }
}

/// Compares every configured endpoint with its provider's list. Pure: the lists are given. Providers are
/// reported in name order and endpoints in target order, so the report is stable.
#[must_use]
pub fn compare(config: &Config, listings: &BTreeMap<String, Listing>) -> Report {
    let mut report = Report::default();
    for (provider, listing) in listings {
        match listing {
            Listing::NoList(reason) => report.no_list.push((provider.clone(), reason.clone())),
            Listing::Unreachable(reason) => {
                report.unreachable.push((provider.clone(), reason.clone()));
            }
            Listing::Unusable(reason) => report.unusable.push((provider.clone(), reason.clone())),
            Listing::NoKey => report.skipped.push(provider.clone()),
            Listing::Listed(_) => {}
        }
    }
    for (target, endpoints) in &config.targets {
        for endpoint in endpoints {
            if let Some(Listing::Listed(listed)) = listings.get(&endpoint.provider) {
                report.checked += 1;
                if !listed.contains(&endpoint.model) {
                    report.retired.push(Retired {
                        target: target.clone(),
                        provider: endpoint.provider.clone(),
                        model: endpoint.model.clone(),
                    });
                }
            }
        }
    }
    report
}

/// Loads the configuration at `path`, asks each provider (or only `only`) for its model list, and compares.
/// One provider's failure never hides the others.
pub async fn check(path: &std::path::Path, only: Option<&str>) -> Result<Report, DiscoverError> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| DiscoverError::Config(format!("cannot read {}: {e}", path.display())))?;
    let config =
        Config::from_toml_with(&text, |n| std::env::var(n).ok(), &|p: &std::path::Path| {
            std::fs::read_to_string(p).ok()
        })
        .map_err(|e| DiscoverError::Config(e.to_string()))?;
    if let Some(name) = only
        && !config.providers.contains_key(name)
    {
        return Err(DiscoverError::Config(format!("no provider named `{name}`")));
    }
    let mut listings = BTreeMap::new();
    for (name, provider) in &config.providers {
        if only.is_some_and(|wanted| wanted != name) {
            continue;
        }
        let listing = if provider.api_key.is_empty() {
            Listing::NoKey
        } else {
            match fetch_provider_list(&provider.base_url, &provider.api_key).await {
                Ok(listed) if listed.is_empty() => {
                    Listing::Unusable("its /models list is empty".into())
                }
                Ok(listed) => Listing::Listed(listed),
                Err(failure) => failure.into(),
            }
        };
        listings.insert(name.clone(), listing);
    }
    Ok(compare(&config, &listings))
}

/// The report as text for a terminal or a job log.
#[must_use]
pub fn render(report: &Report) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    for r in &report.retired {
        let _ = writeln!(
            out,
            "retired:    {} uses {}/{}, which the provider no longer lists",
            r.target, r.provider, r.model
        );
    }
    for (provider, reason) in &report.unusable {
        let _ = writeln!(out, "unusable:   {provider}: {reason}");
    }
    for (provider, reason) in &report.no_list {
        let _ = writeln!(out, "no list:    {provider} has no model list: {reason}");
    }
    for (provider, reason) in &report.unreachable {
        let _ = writeln!(out, "unreachable: {provider} could not be asked: {reason}");
    }
    for provider in &report.skipped {
        let _ = writeln!(out, "skipped:    {provider} has no key to ask with");
    }
    if report.retired.is_empty() && report.unusable.is_empty() {
        let _ = writeln!(
            out,
            "nothing to review: {} endpoint(s) are all listed by their providers",
            report.checked
        );
    }
    out
}

/// The report as JSON.
#[must_use]
pub fn report_json(report: &Report) -> Value {
    let reasons = |pairs: &[(String, String)]| {
        pairs
            .iter()
            .map(|(p, reason)| json!({"provider": p, "reason": reason}))
            .collect::<Vec<_>>()
    };
    json!({
        "checked": report.checked,
        "retired": report.retired.iter().map(|r| json!({
            "target": r.target, "provider": r.provider, "model": r.model,
        })).collect::<Vec<_>>(),
        "unusable": reasons(&report.unusable),
        "no_list": reasons(&report.no_list),
        "unreachable": reasons(&report.unreachable),
        "skipped": report.skipped,
    })
}
