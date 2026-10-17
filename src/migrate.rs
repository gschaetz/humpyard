//! `humpyard migrate-modelrelay`: turns a ModelRelay setup into a humpyard configuration fragment.
//!
//! The facts come from ModelRelay's own admin API (`/api/config` for providers, `/api/models` for models),
//! so no catalog has to be copied from another project, and no API key value is ever requested: the
//! endpoint that returns key values is never called, and the fragment names only environment variables.
//! The output is a suggestion to review and merge (ADR 0014), never an applied change. Only the `migrate`
//! command (`main`) may use this module, like the price sources, so `serve` can never fetch it.

#![allow(clippy::doc_markdown)] // "ModelRelay" is a product name, written in prose throughout

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::time::Duration;

use serde::Deserialize;

use crate::config::Config;

const FETCH_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_BODY_BYTES: usize = 32 * 1024 * 1024;
/// The strengths ModelRelay's tags share with humpyard's (ADR 0021).
const VOCABULARY: [&str; 5] = ["coding", "reasoning", "general", "fast", "agentic"];
/// Added to every generated target so one pool can mean "any of them" (`auto-fastest`).
const CATCH_ALL: &str = "all";

/// Providers whose chat endpoint is OpenAI-compatible, with the base URL to use. ModelRelay's own
/// provider keys; compiled from each provider's public API documentation.
const KNOWN_PROVIDERS: [(&str, &str); 9] = [
    ("nvidia", "https://integrate.api.nvidia.com/v1"),
    ("groq", "https://api.groq.com/openai/v1"),
    ("cerebras", "https://api.cerebras.ai/v1"),
    ("opencode", "https://opencode.ai/zen/v1"),
    ("openrouter", "https://openrouter.ai/api/v1"),
    ("codestral", "https://codestral.mistral.ai/v1"),
    ("scaleway", "https://api.scaleway.ai/v1"),
    ("kilocode", "https://api.kilo.ai/api/gateway"),
    (
        "googleai",
        "https://generativelanguage.googleapis.com/v1beta/openai",
    ),
];
const OPENAI_COMPATIBLE_PREFIX: &str = "openai-compatible:";

#[derive(Debug, thiserror::Error)]
pub enum MigrateError {
    #[error("{0}")]
    Source(String),
    #[error("{0}")]
    Parse(String),
}

/// One provider row of `/api/config`. Unknown fields are ignored; key values are never present (the
/// API masks them), and only whether a key exists is read.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderRow {
    pub key: String,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub has_key: bool,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub has_multiple_keys: bool,
    #[serde(default)]
    pub api_key_pool: Vec<serde_json::Value>,
}

/// One model row of `/api/models`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelRow {
    pub model_id: String,
    pub provider_key: String,
    /// The window as ModelRelay prints it: a number, or a string such as `"128k"` or `"1M"`.
    #[serde(default)]
    pub ctx: Option<serde_json::Value>,
    /// What ModelRelay actually routes with, when it reports it.
    #[serde(default)]
    pub ctx_info: Option<CtxInfo>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub hidden: bool,
    /// ModelRelay's quality score for the model, 0 to 1.
    #[serde(default)]
    pub intell: Option<f64>,
    /// Whether that score is an estimate (from metadata) and not a measured benchmark; absent means unknown,
    /// which is treated as an estimate.
    #[serde(default)]
    pub is_estimated_score: Option<bool>,
}

/// ModelRelay's `ctxInfo`: the context window it uses for `min_ctx` routing.
#[derive(Debug, Deserialize)]
pub struct CtxInfo {
    #[serde(default)]
    pub tokens: Option<u64>,
    /// False when the figure is only a model maximum that no provider is known to serve.
    #[serde(default)]
    pub usable: Option<bool>,
}

impl ModelRow {
    /// The context window to declare: what ModelRelay routes with (`ctxInfo.tokens` when usable,
    /// including a window it capped by observed quota), else the printed `ctx` for older versions
    /// that report no `ctxInfo`. A model maximum that is not usable is not a window.
    pub fn window(&self) -> Option<u64> {
        if let Some(info) = &self.ctx_info {
            return if info.usable == Some(false) {
                None
            } else {
                info.tokens.filter(|t| *t > 0)
            };
        }
        match self.ctx.as_ref()? {
            serde_json::Value::Number(n) => n.as_u64().filter(|t| *t > 0),
            serde_json::Value::String(text) => parse_size(text),
            _ => None,
        }
    }
}

impl ModelRow {
    /// `(declared, estimate_only)`: a score ModelRelay measured is declared as the target's quality; an
    /// estimate is declared only when `include_estimates`, and otherwise returned for a comment.
    fn quality(&self, include_estimates: bool) -> (Option<f64>, Option<f64>) {
        let Some(score) = self
            .intell
            .filter(|s| s.is_finite() && (0.0..=1.0).contains(s))
        else {
            return (None, None);
        };
        // Round to three places: the score is a benchmark index, not a measurement to the last digit.
        let score = (score * 1000.0).round() / 1000.0;
        let measured = self.is_estimated_score == Some(false);
        if measured || include_estimates {
            (Some(score), None)
        } else {
            (None, Some(score))
        }
    }
}

/// `"128k"` is 128,000, `"1M"` is 1,000,000 (ModelRelay's own arithmetic), and a bare number is itself.
fn parse_size(text: &str) -> Option<u64> {
    let text = text.trim();
    let (digits, factor) = match text.chars().last()? {
        'k' | 'K' => (&text[..text.len() - 1], 1_000_u64),
        'm' | 'M' => (&text[..text.len() - 1], 1_000_000),
        _ => (text, 1),
    };
    let number: f64 = digits.trim().parse().ok()?;
    if !(number.is_finite() && number > 0.0 && number <= 1e9) {
        return None;
    }
    let value = crate::num::u64_from_f64_rounded(number * crate::num::f64_from_u64(factor));
    (value > 0).then_some(value)
}

#[derive(Deserialize)]
struct ModelsEnvelope {
    models: Vec<ModelRow>,
}

pub fn parse_config(text: &str) -> Result<Vec<ProviderRow>, MigrateError> {
    serde_json::from_str(text).map_err(|e| {
        MigrateError::Parse(format!(
            "the config response is not the provider list ModelRelay's /api/config returns: {e}"
        ))
    })
}

pub fn parse_models(text: &str) -> Result<Vec<ModelRow>, MigrateError> {
    serde_json::from_str::<ModelsEnvelope>(text)
        .map(|envelope| envelope.models)
        .or_else(|_| serde_json::from_str::<Vec<ModelRow>>(text))
        .map_err(|e| {
            MigrateError::Parse(format!(
                "the models response is not what ModelRelay's /api/models returns: {e}"
            ))
        })
}

/// What the operator chose.
#[derive(Clone, Debug, Default)]
pub struct Options {
    /// The `class` given to every generated target.
    pub class: String,
    /// Minimum context windows to also generate `+min_ctx:N` routes for.
    pub min_ctx: Vec<u64>,
    /// Declare ModelRelay's estimated scores as `quality` too (by default only measured ones are).
    pub estimated_quality: bool,
    /// ModelRelay provider key to the name used in the fragment (`--rename`).
    pub rename: BTreeMap<String, String>,
    /// Written as `quality_weight` on every generated `fastest` route (`--quality-weight`).
    pub quality_weight: Option<f64>,
}

/// A name a provider can be given: lowercase letters, digits and dashes, 1 to 32 characters.
fn valid_provider_name(name: &str) -> bool {
    (1..=32).contains(&name.len())
        && !name.starts_with('-')
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// Checks the options against the providers before anything is printed: a weight in range, renames that name
/// a real ModelRelay provider with a valid new name, and no two providers ending up with the same name.
pub fn check_options(providers: &[ProviderRow], options: &Options) -> Result<(), String> {
    if let Some(weight) = options.quality_weight
        && !(0.0..=1.0).contains(&weight)
    {
        return Err("--quality-weight must be between 0 and 1".to_string());
    }
    for (from, to) in &options.rename {
        if !providers.iter().any(|p| &p.key == from) {
            return Err(format!("--rename: ModelRelay has no provider `{from}`"));
        }
        if !valid_provider_name(to) {
            return Err(format!(
                "--rename {from}={to}: the new name must be lowercase letters, digits and dashes (1 to 32 characters)"
            ));
        }
    }
    let mut ignored = Report::default();
    let used = select_providers(providers, options, &mut ignored);
    let mut names: BTreeMap<&str, &str> = BTreeMap::new();
    for (key, (resolved, _)) in &used {
        if let Some(other) = names.insert(resolved.name.as_str(), key.as_str()) {
            return Err(format!(
                "providers `{other}` and `{key}` would both be named `{}`; rename one of them",
                resolved.name
            ));
        }
    }
    Ok(())
}

/// What was carried over and what was not.
#[derive(Debug, Default)]
pub struct Report {
    pub providers: usize,
    pub targets: usize,
    pub routes: usize,
    /// Things that did not carry over, each with the reason.
    pub skipped: Vec<String>,
    /// How many generated targets declare a `quality`.
    pub with_quality: usize,
    /// Things to do by hand after merging.
    pub todo: Vec<String>,
}

#[derive(Debug)]
pub struct Migration {
    pub fragment: String,
    pub report: Report,
}

/// Lowercase letters, digits and single dashes.
fn slug(text: &str) -> String {
    let mut out = String::new();
    for c in text.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_matches('-')
        .chars()
        .take(60)
        .collect::<String>()
        .trim_matches('-')
        .to_string()
}

/// A TOML basic string on one line: quotes, backslashes and control characters are escaped, so text from
/// ModelRelay can never end the string or start a new table.
fn quoted(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if c.is_control() => {
                let _ = write!(out, "\\u{:04X}", u32::from(c));
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Model ids are copied into the fragment, so one with control characters (or an absurd length) is
/// not a model id and is reported instead.
fn usable_model_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 200 && !id.chars().any(char::is_control)
}

fn env_name(provider: &str) -> String {
    let upper: String = provider
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect();
    format!("{upper}_API_KEY")
}

/// A provider that humpyard can call: its humpyard name and base URL.
struct Resolved {
    name: String,
    base_url: String,
    /// Needs no key (a local server); the variable still has to hold some non-empty value.
    keyless_ok: bool,
}

fn resolve_provider(row: &ProviderRow) -> Result<Resolved, String> {
    if let Some((_, url)) = KNOWN_PROVIDERS.iter().find(|(key, _)| *key == row.key) {
        return Ok(Resolved {
            name: slug(&row.key),
            base_url: (*url).to_string(),
            keyless_ok: false,
        });
    }
    let own_url = row
        .base_url
        .as_deref()
        .map(str::trim)
        .filter(|u| !u.is_empty())
        .map(|u| u.trim_end_matches('/'))
        .map(|u| u.strip_suffix("/chat/completions").unwrap_or(u).to_string());
    if row.key == "ollama" {
        return own_url
            .map(|url| Resolved {
                name: "ollama".into(),
                base_url: if url.ends_with("/v1") {
                    url
                } else {
                    format!("{url}/v1")
                },
                keyless_ok: true,
            })
            .ok_or_else(|| "no base URL is set for it".to_string());
    }
    if let Some(id) = row.key.strip_prefix(OPENAI_COMPATIBLE_PREFIX) {
        return own_url
            .map(|url| Resolved {
                name: format!("oai-{}", slug(id)),
                base_url: url,
                keyless_ok: true,
            })
            .ok_or_else(|| "its endpoint has no base URL".to_string());
    }
    Err("humpyard has no OpenAI-compatible base URL for this provider".to_string())
}

struct TargetPlan {
    name: String,
    provider: String,
    model: String,
    ctx: Option<u64>,
    strengths: Vec<String>,
    /// A measured quality, declared on the target.
    quality: Option<f64>,
    /// ModelRelay's estimate, kept as a comment unless `--estimated-quality`.
    estimated: Option<f64>,
}

/// The providers humpyard can call, by ModelRelay key; the flag records that a model used one.
type Used = BTreeMap<String, (Resolved, bool)>;

/// Which providers carry over; the rest are reported with the reason.
fn select_providers(providers: &[ProviderRow], options: &Options, report: &mut Report) -> Used {
    let mut used = Used::new();
    for row in providers {
        if !row.enabled {
            report
                .skipped
                .push(format!("provider `{}`: disabled in ModelRelay", row.key));
            continue;
        }
        let resolved = match resolve_provider(row) {
            Ok(mut resolved) => {
                if let Some(to) = options.rename.get(&row.key) {
                    resolved.name.clone_from(to);
                }
                resolved
            }
            Err(reason) => {
                report
                    .skipped
                    .push(format!("provider `{}`: {reason}", row.key));
                continue;
            }
        };
        if !row.has_key && !resolved.keyless_ok {
            report.skipped.push(format!(
                "provider `{}`: ModelRelay has no key for it",
                row.key
            ));
            continue;
        }
        if row.has_multiple_keys || row.api_key_pool.len() > 1 {
            report.todo.push(format!(
                "provider `{}` has {} keys in ModelRelay; humpyard uses one per provider, so only one \
                 variable ({}) is referenced. Add the others as extra providers by hand if you need them",
                row.key,
                row.api_key_pool.len().max(2),
                env_name(&resolved.name)
            ));
        }
        used.insert(row.key.clone(), (resolved, false));
    }
    used
}

/// One target per usable model, named uniquely, sorted by name.
fn plan_targets(
    models: &[ModelRow],
    used: &mut Used,
    report: &mut Report,
    options: &Options,
) -> Vec<TargetPlan> {
    let mut targets: Vec<TargetPlan> = Vec::new();
    let mut taken: BTreeSet<String> = BTreeSet::new();
    let mut unknown_tags: BTreeSet<String> = BTreeSet::new();
    let mut removed_counts: BTreeMap<(&str, String), usize> = BTreeMap::new();
    for model in models {
        let Some((resolved, seen)) = used.get_mut(&model.provider_key) else {
            continue; // its provider was reported already
        };
        let removed = if model.hidden {
            Some("hidden")
        } else {
            match model.status.as_deref() {
                Some(status @ ("banned" | "disabled" | "excluded")) => Some(status),
                _ => None,
            }
        };
        if let Some(reason) = removed {
            // A ban is a deliberate act on one model, so it is named. The rest come from the operator's
            // filters (an excluded provider, a minimum quality score), so they are counted.
            if reason == "banned" {
                report.skipped.push(format!(
                    "model `{}/{}`: banned in ModelRelay",
                    model.provider_key, model.model_id
                ));
            } else {
                *removed_counts
                    .entry((reason, model.provider_key.clone()))
                    .or_default() += 1;
            }
            continue;
        }
        if !usable_model_id(&model.model_id) {
            report.skipped.push(format!(
                "model on `{}`: its id is empty, too long or has control characters",
                model.provider_key
            ));
            continue;
        }
        *seen = true;
        let mut strengths: Vec<String> = Vec::new();
        for tag in &model.tags {
            let tag = tag.trim().to_ascii_lowercase();
            if VOCABULARY.contains(&tag.as_str()) {
                if !strengths.contains(&tag) {
                    strengths.push(tag);
                }
            } else if !tag.is_empty() {
                unknown_tags.insert(tag);
            }
        }
        strengths.push(CATCH_ALL.to_string());
        let base = format!("{}-{}", resolved.name, slug(&model.model_id));
        let mut name = base.clone();
        let mut n = 2;
        while !taken.insert(name.clone()) {
            name = format!("{base}-{n}");
            n += 1;
        }
        let (quality, estimated) = model.quality(options.estimated_quality);
        targets.push(TargetPlan {
            name,
            provider: resolved.name.clone(),
            model: model.model_id.clone(),
            ctx: model.window(),
            strengths,
            quality,
            estimated,
        });
    }
    let mut by_reason: BTreeMap<&str, Vec<(String, usize)>> = BTreeMap::new();
    for ((reason, provider), count) in removed_counts {
        by_reason.entry(reason).or_default().push((provider, count));
    }
    for (reason, providers) in by_reason {
        let total: usize = providers.iter().map(|(_, n)| n).sum();
        let detail: Vec<String> = providers.iter().map(|(p, n)| format!("{p} {n}")).collect();
        report.skipped.push(format!(
            "{total} model(s) {reason} in ModelRelay: {}",
            detail.join(", ")
        ));
    }
    if !unknown_tags.is_empty() {
        report.skipped.push(format!(
            "tags not in humpyard's strength vocabulary were not carried over: {}",
            unknown_tags.into_iter().collect::<Vec<_>>().join(", ")
        ));
    }
    targets.sort_by(|a, b| a.name.cmp(&b.name));
    targets
}

/// Builds the fragment and the report from the two responses.
pub fn migrate(providers: &[ProviderRow], models: &[ModelRow], options: &Options) -> Migration {
    let mut report = Report::default();
    let mut used = select_providers(providers, options, &mut report);
    let targets = plan_targets(models, &mut used, &mut report, options);

    let mut out = String::new();
    out.push_str(
        "# Generated by `humpyard migrate-modelrelay`. Review it, then merge it into your config.\n",
    );
    out.push_str(
        "# Keys are not copied: set each api_key_env variable in the gateway's environment.\n\n",
    );
    let provider_names: BTreeMap<&str, &Resolved> = used
        .values()
        .filter(|(_, seen)| *seen)
        .map(|(r, _)| (r.name.as_str(), r))
        .collect();
    for (name, resolved) in &provider_names {
        let _ = writeln!(out, "[providers.{name}]");
        let _ = writeln!(out, "base_url = {}", quoted(&resolved.base_url));
        let _ = writeln!(out, "api_key_env = {}", quoted(&env_name(name)));
        if resolved.keyless_ok {
            out.push_str(
                "# ModelRelay has no key for this one: set the variable to its key if the server needs one,\n# or to any non-empty value if it does not (a local server)\n",
            );
        }
        out.push('\n');
    }
    report.providers = provider_names.len();
    for target in &targets {
        render_target(&mut out, target, &options.class);
    }
    report.targets = targets.len();
    report.with_quality = targets.iter().filter(|t| t.quality.is_some()).count();
    report.routes = render_routes(&mut out, &targets, options, &mut report.skipped);
    Migration {
        fragment: out,
        report,
    }
}

fn render_target(out: &mut String, target: &TargetPlan, class: &str) {
    let _ = writeln!(out, "[targets.{}]", target.name);
    let _ = writeln!(out, "class = {}", quoted(class));
    let strengths: Vec<String> = target.strengths.iter().map(|s| quoted(s)).collect();
    let _ = writeln!(out, "strengths = [{}]", strengths.join(", "));
    if let Some(quality) = target.quality {
        let _ = writeln!(out, "quality = {quality}");
    } else if let Some(estimate) = target.estimated {
        let _ = writeln!(
            out,
            "# ModelRelay's estimated quality {estimate} is not declared (its guess, not a measurement); \
             add `quality = {estimate}` by hand or migrate with --estimated-quality"
        );
    }
    let window = target
        .ctx
        .map(|c| format!(", context_window = {c}"))
        .unwrap_or_default();
    let _ = writeln!(
        out,
        "endpoints = [{{ provider = {}, model = {}{window} }}]\n",
        quoted(&target.provider),
        quoted(&target.model)
    );
}

/// `auto-fastest` and `tag:<strength>`, each also with `+min_ctx:N`; routes that would be empty are
/// reported instead of written (an empty pool is a configuration error).
fn render_routes(
    out: &mut String,
    targets: &[TargetPlan],
    options: &Options,
    skipped: &mut Vec<String>,
) -> usize {
    let mut strengths: Vec<&str> = VOCABULARY
        .iter()
        .copied()
        .filter(|s| targets.iter().any(|t| t.strengths.iter().any(|x| x == s)))
        .collect();
    let mut routes: Vec<(String, &str)> = vec![("auto-fastest".to_string(), CATCH_ALL)];
    routes.extend(strengths.drain(..).map(|s| (format!("tag:{s}"), s)));
    let mut windows: Vec<u64> = options.min_ctx.clone();
    windows.sort_unstable();
    windows.dedup();
    let mut written = 0;
    for (name, strength) in routes {
        let candidates = std::iter::once((name.clone(), None)).chain(
            windows
                .iter()
                .map(|n| (format!("{name}+min_ctx:{n}"), Some(*n))),
        );
        for (route, window) in candidates {
            let qualifies = targets.iter().any(|t| {
                t.strengths.iter().any(|s| s == strength)
                    && window.is_none_or(|w| t.ctx.is_some_and(|c| c >= w))
            });
            if !qualifies {
                skipped.push(format!("route `{route}`: no generated target qualifies"));
                continue;
            }
            let min = window
                .map(|w| format!(", min_context_window = {w}"))
                .unwrap_or_default();
            let weight = options
                .quality_weight
                .map(|w| format!("quality_weight = {w:?}\n"))
                .unwrap_or_default();
            let _ = writeln!(
                out,
                "[routes.{}]\ntype = \"fastest\"\n{weight}pool = {{ strength = {}{min} }}\n",
                quoted(&route),
                quoted(strength)
            );
            written += 1;
        }
    }
    written
}

/// Loads the fragment through the real configuration parser (placeholder keys), so a fragment that would
/// not load is reported instead of presented as ready.
pub fn self_check(fragment: &str) -> Result<(), String> {
    let text = format!("listen = \"127.0.0.1:0\"\n\n{fragment}");
    Config::from_toml(&text, |_| Some("placeholder".to_string()))
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// Accepts https, and http only to a loopback host (the admin token must not cross a network in clear).
fn check_url(url: &str) -> Result<reqwest::Url, MigrateError> {
    let parsed = reqwest::Url::parse(url)
        .map_err(|e| MigrateError::Source(format!("`{url}` is not a URL: {e}")))?;
    let loopback = matches!(parsed.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
    if parsed.scheme() == "https" || (parsed.scheme() == "http" && loopback) {
        Ok(parsed)
    } else {
        Err(MigrateError::Source(format!(
            "`{url}`: the admin token is sent with the request, so ModelRelay must be reached over https \
             (or http on this machine; use a port-forward otherwise), or pass saved responses with --config and --models"
        )))
    }
}

/// Reads one admin API path. Only `/api/config` and `/api/models` are ever requested.
pub async fn fetch(base: &str, path: &str, token: Option<&str>) -> Result<String, MigrateError> {
    debug_assert!(matches!(path, "/api/config" | "/api/models"));
    let base = check_url(base)?;
    let url = base
        .join(path)
        .map_err(|e| MigrateError::Source(format!("cannot form the request URL: {e}")))?;
    let client = reqwest::Client::builder()
        .timeout(FETCH_TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| MigrateError::Source(e.to_string()))?;
    let mut request = client.get(url.clone());
    if let Some(token) = token {
        request = request.bearer_auth(token);
    }
    let mut response = request
        .send()
        .await
        .map_err(|e| MigrateError::Source(format!("cannot reach {url}: {e}")))?;
    if !response.status().is_success() {
        return Err(MigrateError::Source(format!(
            "{url} answered {} (a token may be needed: --token-env)",
            response.status()
        )));
    }
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|e| MigrateError::Source(format!("reading {url}: {e}")))?
    {
        if body.len() + chunk.len() > MAX_BODY_BYTES {
            return Err(MigrateError::Source(format!(
                "{url} is larger than 32 MB; refusing it"
            )));
        }
        body.extend_from_slice(&chunk);
    }
    String::from_utf8(body).map_err(|_| MigrateError::Source(format!("{url} is not UTF-8 text")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn providers(json: &str) -> Vec<ProviderRow> {
        parse_config(json).unwrap()
    }
    fn models(json: &str) -> Vec<ModelRow> {
        parse_models(json).unwrap()
    }
    fn options(min_ctx: &[u64]) -> Options {
        Options {
            class: "public".into(),
            min_ctx: min_ctx.to_vec(),
            ..Options::default()
        }
    }

    const CONFIG: &str = r#"[
      {"key":"groq","name":"Groq","enabled":true,"hasKey":true,"apiKeyPool":[{"index":0,"masked":"gsk_...abcd"}]},
      {"key":"cerebras","enabled":true,"hasKey":true,"hasMultipleKeys":true,
       "apiKeyPool":[{"index":0,"masked":"a"},{"index":1,"masked":"b"}]},
      {"key":"nvidia","enabled":false,"hasKey":true},
      {"key":"openrouter","enabled":true,"hasKey":false},
      {"key":"kiro","enabled":true,"hasKey":true},
      {"key":"ollama","enabled":true,"hasKey":false,"baseUrl":"http://ollama.example.net:11434"},
      {"key":"openai-compatible:lab","enabled":true,"hasKey":true,"baseUrl":"https://lab.example/v1/chat/completions"}
    ]"#;

    const MODELS: &str = r#"{"models":[
      {"modelId":"llama-3.3-70b-versatile","providerKey":"groq","ctx":131072,"tags":["general","fast"],"status":"up"},
      {"modelId":"llama-3.3-70b-versatile","providerKey":"cerebras","ctx":65536,"tags":["general","weird"],"status":"pending"},
      {"modelId":"qwen/qwen3-coder","providerKey":"cerebras","ctx":null,"tags":["coding"]},
      {"modelId":"banned-one","providerKey":"groq","ctx":1000,"tags":["general"],"status":"banned"},
      {"modelId":"free:model","providerKey":"openrouter","ctx":32000,"tags":["general"]},
      {"modelId":"x","providerKey":"nvidia","ctx":8000,"tags":["general"]},
      {"modelId":"local-8b","providerKey":"ollama","ctx":8192,"tags":["fast"]},
      {"modelId":"lab/model","providerKey":"openai-compatible:lab","ctx":200000,"tags":["coding","reasoning"]}
    ],"best":null}"#;

    #[test]
    fn a_setup_becomes_providers_targets_and_routes_that_load() {
        let m = migrate(&providers(CONFIG), &models(MODELS), &options(&[32000]));
        assert_eq!(self_check(&m.fragment), Ok(()), "{}", m.fragment);
        assert_eq!(m.report.providers, 4, "{}", m.fragment);
        assert!(m.fragment.contains("[providers.groq]"));
        assert!(
            m.fragment
                .contains("base_url = \"https://api.groq.com/openai/v1\"")
        );
        assert!(m.fragment.contains("api_key_env = \"GROQ_API_KEY\""));
        assert!(m.fragment.contains("[providers.oai-lab]"));
        assert!(
            m.fragment.contains("base_url = \"https://lab.example/v1\""),
            "chat path stripped"
        );
        assert!(
            m.fragment
                .contains("base_url = \"http://ollama.example.net:11434/v1\"")
        );
        assert!(
            m.fragment
                .contains("[targets.groq-llama-3-3-70b-versatile]")
        );
        assert!(m.fragment.contains("context_window = 131072"));
        assert!(
            m.fragment
                .contains(r#"strengths = ["general", "fast", "all"]"#)
        );
        assert!(m.fragment.contains("class = \"public\""));
    }

    #[test]
    fn nothing_that_looks_like_a_key_is_written() {
        let m = migrate(&providers(CONFIG), &models(MODELS), &options(&[]));
        for secret in ["gsk_", "abcd", "masked"] {
            assert!(!m.fragment.contains(secret), "{secret}");
        }
    }

    #[test]
    fn the_same_model_on_two_providers_gets_two_targets_and_collisions_are_numbered() {
        let cfg = providers(r#"[{"key":"groq","enabled":true,"hasKey":true}]"#);
        let mdl = models(
            r#"[{"modelId":"A/B","providerKey":"groq"},{"modelId":"a-b","providerKey":"groq"}]"#,
        );
        let m = migrate(&cfg, &mdl, &options(&[]));
        assert!(m.fragment.contains("[targets.groq-a-b]"));
        assert!(m.fragment.contains("[targets.groq-a-b-2]"));
        assert_eq!(self_check(&m.fragment), Ok(()));
    }

    #[test]
    fn skipped_things_are_reported_with_reasons() {
        let m = migrate(&providers(CONFIG), &models(MODELS), &options(&[]));
        let skipped = m.report.skipped.join("\n");
        assert!(skipped.contains("provider `nvidia`: disabled"), "{skipped}");
        assert!(
            skipped.contains("provider `openrouter`: ModelRelay has no key"),
            "{skipped}"
        );
        assert!(
            skipped.contains("provider `kiro`: humpyard has no OpenAI-compatible base URL"),
            "{skipped}"
        );
        assert!(
            skipped.contains("model `groq/banned-one`: banned"),
            "{skipped}"
        );
        assert!(
            skipped.contains("weird"),
            "unknown tags are named: {skipped}"
        );
        assert!(!m.fragment.contains("banned-one") && !m.fragment.contains("[providers.kiro]"));
        let todo = m.report.todo.join("\n");
        assert!(
            todo.contains("cerebras") && todo.contains("2 keys"),
            "{todo}"
        );
    }

    #[test]
    fn drop_in_routes_cover_auto_fastest_every_strength_and_each_window() {
        let m = migrate(
            &providers(CONFIG),
            &models(MODELS),
            &options(&[32_000, 100_000]),
        );
        for route in [
            "auto-fastest",
            "auto-fastest+min_ctx:32000",
            "auto-fastest+min_ctx:100000",
            "tag:general",
            "tag:general+min_ctx:32000",
            "tag:coding",
            "tag:coding+min_ctx:100000",
            "tag:fast",
            "tag:reasoning",
        ] {
            assert!(
                m.fragment.contains(&format!("[routes.\"{route}\"]")),
                "{route}\n{}",
                m.fragment
            );
        }
        assert_eq!(self_check(&m.fragment), Ok(()), "{}", m.fragment);
    }

    #[test]
    fn a_route_that_would_be_an_empty_pool_is_reported_not_written() {
        let m = migrate(&providers(CONFIG), &models(MODELS), &options(&[500_000]));
        assert!(!m.fragment.contains("+min_ctx:500000"), "{}", m.fragment);
        assert!(
            m.report
                .skipped
                .iter()
                .any(|s| s.contains("tag:general+min_ctx:500000")),
            "{:?}",
            m.report.skipped
        );
        assert_eq!(self_check(&m.fragment), Ok(()), "{}", m.fragment);
    }

    #[test]
    fn untrusted_text_cannot_inject_configuration() {
        let cfg = providers(r#"[{"key":"groq","enabled":true,"hasKey":true}]"#);
        // A newline in a model id is not a model id: skipped and reported, nothing from it is written.
        let junk = models(
            r#"[{"modelId":"x\"\n[keys.evil]\nsha256 = \"0\"","providerKey":"groq","tags":["general"]}]"#,
        );
        let m = migrate(&cfg, &junk, &options(&[]));
        assert!(!m.fragment.contains("evil"), "{}", m.fragment);
        assert!(
            m.report
                .skipped
                .iter()
                .any(|s| s.contains("control characters"))
        );
        // Quotes and backslashes in an otherwise normal id are escaped on one line and still load.
        let quirky = models(r#"[{"modelId":"a\"b\\c","providerKey":"groq","tags":["general"]}]"#);
        let m = migrate(&cfg, &quirky, &options(&[]));
        assert_eq!(self_check(&m.fragment), Ok(()), "{}", m.fragment);
        assert!(
            m.fragment.contains(r#"model = "a\"b\\c""#),
            "{}",
            m.fragment
        );
    }

    #[test]
    fn nothing_to_migrate_does_not_pass_the_self_check() {
        let m = migrate(&[], &[], &options(&[]));
        assert!(self_check(&m.fragment).is_err());
    }

    #[test]
    fn parse_errors_say_what_was_expected() {
        assert!(
            parse_config("{}")
                .unwrap_err()
                .to_string()
                .contains("/api/config")
        );
        assert!(
            parse_models("[1]")
                .unwrap_err()
                .to_string()
                .contains("/api/models")
        );
        assert_eq!(
            parse_models("[]").unwrap().len(),
            0,
            "a bare array is accepted too"
        );
    }

    #[test]
    fn the_admin_token_never_crosses_the_network_in_clear() {
        assert!(check_url("https://modelrelay.example").is_ok());
        assert!(check_url("http://127.0.0.1:7352").is_ok());
        assert!(check_url("http://modelrelay.internal:7352").is_err());
        assert!(check_url("ftp://x").is_err());
    }

    fn window_of(json: &str) -> Option<u64> {
        models(&format!("[{json}]"))[0].window()
    }

    #[test]
    fn windows_follow_what_modelrelay_routes_with() {
        let base = r#""modelId":"m","providerKey":"groq""#;
        // Printed strings use ModelRelay's own arithmetic.
        assert_eq!(
            window_of(&format!(r#"{{{base},"ctx":"128k"}}"#)),
            Some(128_000)
        );
        assert_eq!(
            window_of(&format!(r#"{{{base},"ctx":"1M"}}"#)),
            Some(1_000_000)
        );
        assert_eq!(
            window_of(&format!(r#"{{{base},"ctx":"262144"}}"#)),
            Some(262_144)
        );
        assert_eq!(
            window_of(&format!(r#"{{{base},"ctx":65536}}"#)),
            Some(65_536)
        );
        assert_eq!(window_of(&format!(r#"{{{base},"ctx":null}}"#)), None);
        assert_eq!(window_of(&format!(r#"{{{base},"ctx":"lots"}}"#)), None);
        assert_eq!(window_of(&format!(r#"{{{base},"ctx":"0k"}}"#)), None);
        // `ctxInfo` wins: a usable window, including one ModelRelay capped by observed quota ...
        let capped = r#""ctx":"131072","ctxInfo":{"tokens":8000,"usable":true,"capped":true,"reason":"quota"}"#;
        assert_eq!(window_of(&format!("{{{base},{capped}}}")), Some(8_000));
        // ... and a model maximum that no provider is known to serve is not a window at all.
        let maximum =
            r#""ctx":"512k","ctxInfo":{"tokens":null,"usable":false,"reason":"model-maximum"}"#;
        assert_eq!(window_of(&format!("{{{base},{maximum}}}")), None);
    }

    #[test]
    fn models_excluded_by_the_operators_filters_are_not_migrated_but_down_ones_are() {
        let cfg = providers(r#"[{"key":"groq","enabled":true,"hasKey":true}]"#);
        let mdl = models(
            r#"[
              {"modelId":"filtered","providerKey":"groq","status":"excluded","tags":["general"]},
              {"modelId":"slow-now","providerKey":"groq","status":"down","tags":["general"]},
              {"modelId":"late","providerKey":"groq","status":"timeout","tags":["general"]},
              {"modelId":"fine","providerKey":"groq","status":"up","tags":["general"]}
            ]"#,
        );
        let m = migrate(&cfg, &mdl, &options(&[]));
        assert!(!m.fragment.contains("filtered"), "{}", m.fragment);
        for kept in ["slow-now", "late", "fine"] {
            assert!(
                m.fragment.contains(kept),
                "{kept} is a ping state, not a removal:\n{}",
                m.fragment
            );
        }
        assert!(
            m.report
                .skipped
                .iter()
                .any(|s| s.starts_with("1 model(s) excluded") && s.contains("groq 1")),
            "{:?}",
            m.report.skipped
        );
    }

    fn renamed(pairs: &[(&str, &str)]) -> Options {
        Options {
            rename: pairs
                .iter()
                .map(|(a, b)| ((*a).to_string(), (*b).to_string()))
                .collect(),
            ..options(&[32_000])
        }
    }

    #[test]
    fn a_renamed_provider_is_renamed_everywhere_and_still_loads() {
        let cfg = providers(CONFIG);
        let opts = renamed(&[("groq", "groq-free")]);
        assert_eq!(check_options(&cfg, &opts), Ok(()));
        let m = migrate(&cfg, &models(MODELS), &opts);
        assert!(
            m.fragment.contains("[providers.groq-free]"),
            "{}",
            m.fragment
        );
        assert!(
            m.fragment.contains("api_key_env = \"GROQ_FREE_API_KEY\""),
            "{}",
            m.fragment
        );
        assert!(
            m.fragment
                .contains("[targets.groq-free-llama-3-3-70b-versatile]"),
            "{}",
            m.fragment
        );
        assert!(m.fragment.contains("provider = \"groq-free\""));
        assert!(!m.fragment.contains("[providers.groq]") && !m.fragment.contains("GROQ_API_KEY"));
        assert_eq!(self_check(&m.fragment), Ok(()), "{}", m.fragment);
    }

    #[test]
    fn bad_renames_and_weights_are_refused_before_anything_is_printed() {
        let cfg = providers(CONFIG);
        let refused = |opts: Options| check_options(&cfg, &opts).unwrap_err();
        assert!(refused(renamed(&[("nope", "x")])).contains("no provider `nope`"));
        assert!(refused(renamed(&[("groq", "Has Space")])).contains("lowercase"));
        assert!(refused(renamed(&[("groq", "-lead")])).contains("lowercase"));
        assert!(refused(renamed(&[("groq", "")])).contains("lowercase"));
        // Two providers ending up with the same name, or one taking another's.
        assert!(
            refused(renamed(&[("groq", "same"), ("cerebras", "same")]))
                .contains("both be named `same`")
        );
        assert!(refused(renamed(&[("groq", "cerebras")])).contains("both be named `cerebras`"));
        for weight in [-0.1, 1.5, f64::NAN] {
            let opts = Options {
                quality_weight: Some(weight),
                ..options(&[])
            };
            assert!(
                check_options(&cfg, &opts)
                    .unwrap_err()
                    .contains("--quality-weight"),
                "{weight}"
            );
        }
    }

    #[test]
    fn a_quality_weight_is_written_on_every_route_and_loads() {
        let opts = Options {
            quality_weight: Some(0.5),
            ..options(&[32_000])
        };
        let m = migrate(&providers(CONFIG), &models(MODELS), &opts);
        let routes = m.fragment.matches("type = \"fastest\"").count();
        assert!(routes >= 6, "{routes}");
        assert_eq!(
            m.fragment.matches("quality_weight = 0.5\n").count(),
            routes,
            "{}",
            m.fragment
        );
        assert_eq!(self_check(&m.fragment), Ok(()), "{}", m.fragment);
        // And a whole number is still a float in the file.
        let whole = Options {
            quality_weight: Some(1.0),
            ..options(&[])
        };
        let m = migrate(&providers(CONFIG), &models(MODELS), &whole);
        assert!(
            m.fragment.contains("quality_weight = 1.0\n"),
            "{}",
            m.fragment
        );
        assert_eq!(self_check(&m.fragment), Ok(()));
        // Without the option nothing is written.
        let none = migrate(&providers(CONFIG), &models(MODELS), &options(&[]));
        assert!(!none.fragment.contains("quality_weight"));
    }
}
