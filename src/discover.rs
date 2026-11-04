//! Model discovery (ADR 0022): which free models a provider offers that a pool does not have yet, and
//! which it no longer offers. Used only by the `humpyard models` commands; the serving process never calls
//! any of this, it only reads the file `models sync` writes (see `config::discovered`).
//!
//! A model is **added** only when two independent sources agree: the open models.dev dataset says it is a
//! free chat model with a context window, and the provider's own `/models` lists it for the operator's key.
//! A model already in the generated file is **kept** while the provider still lists it and the dataset has not
//! recorded a cost for it, so a dataset that lags or omits a model never removes anything; it is **removed**
//! when the provider stops listing it or the dataset says it has become paid. Both sources are untrusted: every
//! number is validated, the provider key goes only to the provider's own host, and none goes to the dataset.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::config::{Config, DiscoverSpec};

const FETCH_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_BODY_BYTES: usize = 16 * 1024 * 1024;
/// A generated set smaller than this is too small for the removal guard to mean anything.
const GUARD_MIN_PREVIOUS: usize = 4;
/// Every generated target is named with this prefix (see `config::discovered`).
pub const PREFIX: &str = "auto-";
/// The strength added to every generated target so `auto-fastest`-style pools can include them.
pub const CATCH_ALL: &str = "all";

#[derive(Debug, thiserror::Error)]
pub enum DiscoverError {
    #[error("{0}")]
    Source(String),
    #[error("{0}")]
    Config(String),
}

/// A free chat model as the dataset states it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    pub id: String,
    pub context: u64,
    pub tools: bool,
    pub vision: bool,
    pub reasoning: bool,
}

/// A paid chat model as the dataset states it (USD per million tokens).
#[derive(Clone, Debug, PartialEq)]
pub struct PaidModel {
    pub candidate: Candidate,
    pub input: f64,
    pub output: f64,
}

/// What the dataset says about one provider's models.
#[derive(Clone, Debug, Default)]
pub struct SourceModels {
    /// Free chat models with a context window.
    pub free: BTreeMap<String, Candidate>,
    /// Models the dataset records a nonzero cost for.
    pub paid: BTreeSet<String>,
    /// The paid chat models with a context window, for reports only: discovery never adds a paid model.
    pub paid_chat: BTreeMap<String, PaidModel>,
    /// Every model id the dataset lists for the provider.
    pub all: BTreeSet<String>,
}

fn positive(value: &Value) -> Option<f64> {
    value.as_f64().filter(|n| n.is_finite() && *n >= 0.0)
}

fn lists(object: &Value, field: &str, item: &str) -> bool {
    object[field]
        .as_array()
        .is_some_and(|items| items.iter().any(|v| v.as_str() == Some(item)))
}

/// Reads one provider's models out of the models.dev dataset.
pub fn parse_source(body: &str, provider_id: &str) -> Result<SourceModels, DiscoverError> {
    let dataset: Value = serde_json::from_str(body)
        .map_err(|e| DiscoverError::Source(format!("the dataset is not JSON: {e}")))?;
    let models = dataset[provider_id]["models"].as_object().ok_or_else(|| {
        DiscoverError::Source(format!(
            "the dataset has no provider `{provider_id}` with models"
        ))
    })?;
    let mut out = SourceModels::default();
    for (key, model) in models {
        let id = model["id"].as_str().unwrap_or(key).to_string();
        out.all.insert(id.clone());
        let cost = &model["cost"];
        let (input, output) = (positive(&cost["input"]), positive(&cost["output"]));
        let context = model["limit"]["context"].as_u64().unwrap_or(0);
        let chat = lists(&model["modalities"], "input", "text")
            && lists(&model["modalities"], "output", "text");
        let candidate = Candidate {
            id: id.clone(),
            context,
            tools: model["tool_call"].as_bool().unwrap_or(false),
            vision: lists(&model["modalities"], "input", "image"),
            reasoning: model["reasoning"].as_bool().unwrap_or(false),
        };
        match (input, output) {
            (Some(i), Some(o)) if i == 0.0 && o == 0.0 => {}
            (Some(i), Some(o)) => {
                out.paid.insert(id.clone());
                if chat && context > 0 {
                    out.paid_chat.insert(
                        id,
                        PaidModel {
                            candidate,
                            input: i,
                            output: o,
                        },
                    );
                }
                continue;
            }
            (Some(_), None) | (None, Some(_)) => {
                out.paid.insert(id);
                continue;
            }
            // No cost stated: not known to be free.
            (None, None) => continue,
        }
        if context == 0 || !chat {
            continue;
        }
        out.free.insert(id, candidate);
    }
    Ok(out)
}

/// The model ids an OpenAI-compatible `/models` response lists.
pub fn parse_provider_list(body: &str) -> Result<BTreeSet<String>, DiscoverError> {
    let value: Value = serde_json::from_str(body)
        .map_err(|e| DiscoverError::Source(format!("`/models` did not return JSON: {e}")))?;
    let items = value["data"]
        .as_array()
        .or_else(|| value["models"].as_array())
        .or_else(|| value.as_array())
        .ok_or_else(|| DiscoverError::Source("`/models` has no list of models".into()))?;
    Ok(items
        .iter()
        .filter_map(|item| {
            item.as_str()
                .or_else(|| item["id"].as_str())
                .or_else(|| item["name"].as_str())
                .map(str::to_string)
        })
        .collect())
}

/// A provider's own model list, fetched with its key and only from its own base URL. Redirects are not
/// followed, so the key cannot be carried to another host.
pub async fn fetch_provider_models(
    base_url: &str,
    api_key: &str,
) -> Result<BTreeSet<String>, DiscoverError> {
    fetch_provider_models_within(base_url, api_key, FETCH_TIMEOUT).await
}

/// [`fetch_provider_models`] with the time limit given (a test uses a short one).
pub async fn fetch_provider_models_within(
    base_url: &str,
    api_key: &str,
    timeout: Duration,
) -> Result<BTreeSet<String>, DiscoverError> {
    let url = format!("{}/models", base_url.trim_end_matches('/'));
    let client = reqwest::Client::builder()
        .timeout(timeout)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| DiscoverError::Source(e.to_string()))?;
    let mut response = client
        .get(&url)
        .bearer_auth(api_key)
        .send()
        .await
        .map_err(|e| DiscoverError::Source(format!("cannot reach {url}: {e}")))?;
    if !response.status().is_success() {
        return Err(DiscoverError::Source(format!(
            "{url} answered {}",
            response.status()
        )));
    }
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|e| DiscoverError::Source(format!("reading {url}: {e}")))?
    {
        if body.len() + chunk.len() > MAX_BODY_BYTES {
            return Err(DiscoverError::Source(format!("{url} is larger than 16 MB")));
        }
        body.extend_from_slice(&chunk);
    }
    let text = String::from_utf8(body)
        .map_err(|_| DiscoverError::Source(format!("{url} is not UTF-8 text")))?;
    parse_provider_list(&text)
}

/// One generated target, as it is written to the file.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Generated {
    pub strengths: Vec<String>,
    pub endpoints: Vec<GeneratedEndpoint>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GeneratedEndpoint {
    pub provider: String,
    pub model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_window: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vision: Option<bool>,
    pub price: GeneratedPrice,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GeneratedPrice {
    pub input: f64,
    pub output: f64,
}

#[derive(Serialize, Deserialize)]
struct FileShape {
    #[serde(default)]
    targets: BTreeMap<String, Generated>,
}

/// The targets of an existing generated file, leniently read (a file the gateway would ignore is not trusted
/// to say what to keep, so an unreadable one counts as empty).
pub fn read_generated(text: &str) -> BTreeMap<String, Generated> {
    toml::from_str::<FileShape>(text)
        .map(|file| file.targets)
        .unwrap_or_default()
}

/// A target name from a provider and a model id: lowercase letters, digits and single dashes.
pub fn target_name(provider: &str, model: &str) -> String {
    let mut slug = String::new();
    for c in model.chars() {
        if c.is_ascii_alphanumeric() {
            slug.push(c.to_ascii_lowercase());
        } else if !slug.ends_with('-') {
            slug.push('-');
        }
    }
    let slug: String = slug.trim_matches('-').chars().take(60).collect();
    format!("{PREFIX}{provider}-{}", slug.trim_matches('-'))
}

/// What the sources said about one opted-in provider.
pub struct ProviderView {
    pub name: String,
    pub spec: DiscoverSpec,
    pub source: SourceModels,
    /// The provider's own list; `None` when `confirm = false`.
    pub listed: Option<BTreeSet<String>>,
    /// Why the provider's list could not be used, when it could not.
    pub unusable: Option<String>,
}

/// A window the configuration declares that differs from the dataset's.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WindowDrift {
    pub target: String,
    pub model: String,
    pub declared: u64,
    pub source: u64,
}

/// What discovery would do, before any guardrail.
#[derive(Debug, Default)]
pub struct Plan {
    /// The whole desired generated set, by target name.
    pub desired: BTreeMap<String, Generated>,
    pub added: Vec<(String, Candidate)>,
    pub removed: Vec<String>,
    pub kept: usize,
    /// Free in the dataset but not listed by the provider (only when it confirms).
    pub unconfirmed: Vec<(String, String)>,
    /// Hand-written or generated endpoints on opted-in providers that the dataset no longer lists.
    pub gone: Vec<(String, String)>,
    pub window_drift: Vec<WindowDrift>,
    /// New paid chat models (only when asked for); never added, only reported.
    pub paid_new: Vec<(String, PaidModel)>,
    /// Providers left alone this run, and why.
    pub skipped: Vec<(String, String)>,
}

impl Plan {
    /// Whether there is anything for an operator to review (what `models check` exits 1 for).
    #[must_use]
    pub fn needs_review(&self) -> bool {
        !self.added.is_empty()
            || !self.paid_new.is_empty()
            || !self.removed.is_empty()
            || !self.gone.is_empty()
            || !self.window_drift.is_empty()
    }
}

fn generated_target(
    view: &ProviderView,
    model: &str,
    candidate: Option<&Candidate>,
    previous: Option<&GeneratedEndpoint>,
) -> Generated {
    let mut strengths = view.spec.strengths.clone();
    if candidate.is_some_and(|c| c.reasoning) && !strengths.iter().any(|s| s == "reasoning") {
        strengths.push("reasoning".to_string());
    }
    strengths.push(CATCH_ALL.to_string());
    strengths.dedup();
    let carried = |f: fn(&GeneratedEndpoint) -> Option<bool>| previous.and_then(f);
    Generated {
        strengths,
        endpoints: vec![GeneratedEndpoint {
            provider: view.name.clone(),
            model: model.to_string(),
            context_window: candidate
                .map(|c| c.context)
                .or_else(|| previous.and_then(|p| p.context_window)),
            tools: candidate.map(|c| c.tools).or_else(|| carried(|p| p.tools)),
            vision: candidate
                .map(|c| c.vision)
                .or_else(|| carried(|p| p.vision)),
            price: GeneratedPrice {
                input: 0.0,
                output: 0.0,
            },
        }],
    }
}

/// Works out the desired generated set from the sources, the configuration and the previous generated file.
pub fn plan(
    config: &Config,
    views: &[ProviderView],
    previous: &BTreeMap<String, Generated>,
    include_paid: bool,
) -> Plan {
    let hand_written: BTreeSet<(&str, &str)> = config
        .targets
        .iter()
        .filter(|(name, _)| !config.generated_targets.contains(*name))
        .flat_map(|(_, endpoints)| endpoints)
        .map(|e| (e.provider.as_str(), e.model.as_str()))
        .collect();
    let excluded: BTreeSet<&str> = config
        .discovery
        .exclude
        .iter()
        .map(String::as_str)
        .collect();
    let mut plan = Plan::default();
    let handled: BTreeSet<&str> = views.iter().map(|v| v.name.as_str()).collect();
    // Targets of providers that are no longer opted in are dropped; the loader would refuse them anyway.
    for (name, target) in previous {
        let provider = target.endpoints.first().map(|e| e.provider.as_str());
        if provider.is_some_and(|p| !handled.contains(p)) {
            plan.removed.push(name.clone());
        }
    }
    for view in views {
        plan_provider(config, view, previous, &hand_written, &excluded, &mut plan);
        if include_paid {
            paid_new(config, view, &excluded, &mut plan);
        }
    }
    plan
}

fn plan_provider(
    config: &Config,
    view: &ProviderView,
    previous: &BTreeMap<String, Generated>,
    hand_written: &BTreeSet<(&str, &str)>,
    excluded: &BTreeSet<&str>,
    plan: &mut Plan,
) {
    let mine: Vec<(&String, &Generated)> = previous
        .iter()
        .filter(|(_, t)| t.endpoints.first().is_some_and(|e| e.provider == view.name))
        .collect();
    if let Some(reason) = &view.unusable {
        // The provider's list could not be read (or came back empty): leave its generated targets alone.
        plan.skipped.push((view.name.clone(), reason.clone()));
        for (name, target) in mine {
            plan.kept += 1;
            plan.desired.insert(name.clone(), target.clone());
        }
        return;
    }
    let authority = |model: &str| match &view.listed {
        Some(listed) => listed.contains(model),
        None => view.source.all.contains(model),
    };
    let mut present: BTreeSet<&str> = BTreeSet::new();
    for (name, target) in &mine {
        let Some(endpoint) = target.endpoints.first() else {
            continue;
        };
        let model = endpoint.model.as_str();
        let id = format!("{}/{model}", view.name);
        if authority(model) && !view.source.paid.contains(model) && !excluded.contains(id.as_str())
        {
            present.insert(model);
            plan.kept += 1;
            let refreshed =
                generated_target(view, model, view.source.free.get(model), Some(endpoint));
            plan.desired.insert((*name).clone(), refreshed);
        } else {
            plan.removed.push((*name).clone());
        }
    }
    for (model, candidate) in &view.source.free {
        let id = format!("{}/{model}", view.name);
        if present.contains(model.as_str())
            || hand_written.contains(&(view.name.as_str(), model.as_str()))
            || excluded.contains(id.as_str())
        {
            continue;
        }
        if view
            .listed
            .as_ref()
            .is_some_and(|listed| !listed.contains(model))
        {
            plan.unconfirmed.push((view.name.clone(), model.clone()));
            continue;
        }
        let name = target_name(&view.name, model);
        plan.added.push((name.clone(), candidate.clone()));
        plan.desired
            .insert(name, generated_target(view, model, Some(candidate), None));
    }
    configured_drift(config, view, plan);
}

/// Paid chat models the provider offers that no target uses. Reported for review, never added: a paid model
/// costs money, so a person adds it by hand (with a price from `humpyard prices import`).
fn paid_new(config: &Config, view: &ProviderView, excluded: &BTreeSet<&str>, plan: &mut Plan) {
    let used: BTreeSet<&str> = config
        .targets
        .values()
        .flatten()
        .filter(|e| e.provider == view.name)
        .map(|e| e.model.as_str())
        .collect();
    for (model, paid) in &view.source.paid_chat {
        let listed = view.listed.as_ref().is_none_or(|l| l.contains(model));
        let id = format!("{}/{model}", view.name);
        if listed && !used.contains(model.as_str()) && !excluded.contains(id.as_str()) {
            plan.paid_new.push((view.name.clone(), paid.clone()));
        }
    }
}

/// Configured endpoints on this provider that the dataset no longer lists or whose window differs.
fn configured_drift(config: &Config, view: &ProviderView, plan: &mut Plan) {
    for (target, endpoints) in &config.targets {
        for endpoint in endpoints.iter().filter(|e| e.provider == view.name) {
            if !view.source.all.contains(&endpoint.model) {
                plan.gone.push((target.clone(), endpoint.model.clone()));
                continue;
            }
            let source = view.source.free.get(&endpoint.model).map(|c| c.context);
            if let (Some(declared), Some(source)) = (endpoint.context_window, source)
                && declared != source
            {
                plan.window_drift.push(WindowDrift {
                    target: target.clone(),
                    model: endpoint.model.clone(),
                    declared,
                    source,
                });
            }
        }
    }
}

/// Why a run must not write, when a guardrail trips.
pub fn guardrail(
    config: &Config,
    plan: &Plan,
    previous: &BTreeMap<String, Generated>,
) -> Option<String> {
    let limit = config.discovery.max_added_per_run as usize;
    if plan.added.len() > limit {
        return Some(format!(
            "{} models would be added, more than max_added_per_run = {limit}; nothing was written (raise the limit once you have looked at them with `humpyard models check`)",
            plan.added.len()
        ));
    }
    if previous.len() >= GUARD_MIN_PREVIOUS {
        let count = |n: usize| crate::num::f64_from_u64(u64::try_from(n).unwrap_or(u64::MAX));
        let share = count(plan.removed.len()) / count(previous.len());
        if share > config.discovery.max_removed_fraction {
            return Some(format!(
                "{} of {} generated models would be removed, more than max_removed_fraction = {}; nothing was written (a source may be returning a truncated list)",
                plan.removed.len(),
                previous.len(),
                config.discovery.max_removed_fraction
            ));
        }
    }
    None
}

/// The text of the generated file for a desired set.
pub fn render(desired: &BTreeMap<String, Generated>) -> Result<String, DiscoverError> {
    let body = toml::to_string(&FileShape {
        targets: desired.clone(),
    })
    .map_err(|e| DiscoverError::Config(format!("cannot write the generated file: {e}")))?;
    Ok(format!(
        "# Generated by `humpyard models sync` (ADR 0022). Do not edit: it is rebuilt on every run.\n\
         # To label or change a model, write a target for the same provider and model by hand.\n\n{body}"
    ))
}

/// Everything one run needs: the loaded configuration, the previous generated file, and the plan.
pub struct Gathered {
    pub config: Config,
    pub config_text: String,
    pub file: Option<std::path::PathBuf>,
    pub previous_text: Option<String>,
    pub previous: BTreeMap<String, Generated>,
    pub plan: Plan,
}

fn read_file(path: &std::path::Path) -> Option<String> {
    std::fs::read_to_string(path).ok()
}

/// Loads the configuration at `path` (with its current generated file), reads the sources, and plans.
/// `only` limits the run to one provider; `source_url` is where the dataset is read (a test can change it).
pub async fn gather(
    path: &std::path::Path,
    source_url: &str,
    only: Option<&str>,
    include_paid: bool,
) -> Result<Gathered, DiscoverError> {
    let config_text = std::fs::read_to_string(path)
        .map_err(|e| DiscoverError::Config(format!("cannot read {}: {e}", path.display())))?;
    let config = Config::from_toml_with(&config_text, |n| std::env::var(n).ok(), &read_file)
        .map_err(|e| DiscoverError::Config(e.to_string()))?;
    let opted: Vec<(&String, &crate::config::Provider)> = config
        .providers
        .iter()
        .filter(|(name, p)| {
            p.discover.as_ref().is_some_and(|d| d.enabled)
                && only.is_none_or(|wanted| wanted == name.as_str())
        })
        .collect();
    if opted.is_empty() {
        return Err(DiscoverError::Config(match only {
            Some(name) => format!("provider `{name}` has no `discover` setting"),
            None => "no provider has `discover` set and enabled".to_string(),
        }));
    }
    let dataset = crate::prices::fetch(source_url)
        .await
        .map_err(|e| DiscoverError::Source(e.to_string()))?;
    let mut views = Vec::new();
    for (name, provider) in opted {
        let Some(spec) = provider.discover.clone() else {
            continue;
        };
        views.push(view_of(name, provider, spec, &dataset).await?);
    }
    let file = config.discovery.file.clone();
    let previous_text = file.as_deref().and_then(read_file);
    let previous = previous_text
        .as_deref()
        .map(read_generated)
        .unwrap_or_default();
    let plan = plan(&config, &views, &previous, include_paid);
    Ok(Gathered {
        config,
        config_text,
        file,
        previous_text,
        previous,
        plan,
    })
}

async fn view_of(
    name: &str,
    provider: &crate::config::Provider,
    spec: DiscoverSpec,
    dataset: &str,
) -> Result<ProviderView, DiscoverError> {
    let source_spec = provider.price_source.as_deref().unwrap_or_default();
    let dataset_id = crate::prices::parse_source(source_spec)
        .map_err(|e| DiscoverError::Config(format!("provider `{name}`: {e}")))?;
    let mut view = ProviderView {
        name: name.to_string(),
        spec: spec.clone(),
        source: SourceModels::default(),
        listed: None,
        unusable: None,
    };
    match parse_source(dataset, dataset_id) {
        Ok(source) if !source.all.is_empty() => view.source = source,
        Ok(_) => view.unusable = Some("the dataset lists no models for it".into()),
        Err(e) => view.unusable = Some(e.to_string()),
    }
    if view.unusable.is_none() && spec.confirm {
        match fetch_provider_models(&provider.base_url, &provider.api_key).await {
            Ok(listed) if !listed.is_empty() => view.listed = Some(listed),
            Ok(_) => view.unusable = Some("its own /models list is empty".into()),
            Err(e) => view.unusable = Some(e.to_string()),
        }
    }
    Ok(view)
}

/// What `models sync` did.
#[derive(Debug, PartialEq, Eq)]
pub enum SyncOutcome {
    /// The file already says this.
    Unchanged,
    /// The file was rewritten.
    Written,
    /// A guardrail stopped the run; nothing was written.
    Refused(String),
}

/// Applies a plan: checks the guardrails, checks the new file would load, and writes it atomically.
pub fn apply(g: &Gathered) -> Result<SyncOutcome, DiscoverError> {
    let Some(file) = &g.file else {
        return Err(DiscoverError::Config(
            "set `[discovery] file` so there is somewhere to write".into(),
        ));
    };
    if !g.config.discovery.enabled {
        return Err(DiscoverError::Config(
            "discovery is switched off: set `[discovery] enabled = true` to let `models sync` write the file".into(),
        ));
    }
    if let Some(reason) = guardrail(&g.config, &g.plan, &g.previous) {
        return Ok(SyncOutcome::Refused(reason));
    }
    let text = render(&g.plan.desired)?;
    if g.previous_text.as_deref() == Some(text.as_str()) {
        return Ok(SyncOutcome::Unchanged);
    }
    // Never write a file the gateway would ignore.
    let candidate = Config::from_toml_with(&g.config_text, |n| std::env::var(n).ok(), &|path| {
        if path == file.as_path() {
            Some(text.clone())
        } else {
            read_file(path)
        }
    })
    .map_err(|e| DiscoverError::Config(e.to_string()))?;
    if candidate.discovery_state.loaded() != g.plan.desired.len() {
        return Ok(SyncOutcome::Refused(format!(
            "the new file would not load ({:?}); nothing was written",
            candidate.discovery_state
        )));
    }
    write_atomically(file, &text, g.previous_text.is_some())?;
    Ok(SyncOutcome::Written)
}

fn write_atomically(
    file: &std::path::Path,
    text: &str,
    had_previous: bool,
) -> Result<(), DiscoverError> {
    let io = |e: std::io::Error| DiscoverError::Config(format!("{}: {e}", file.display()));
    if let Some(dir) = file.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).map_err(io)?;
    }
    let mut temp = file.as_os_str().to_owned();
    temp.push(".tmp");
    let temp = std::path::PathBuf::from(temp);
    std::fs::write(&temp, text).map_err(io)?;
    if had_previous {
        let mut prev = file.as_os_str().to_owned();
        prev.push(".prev");
        std::fs::copy(file, std::path::PathBuf::from(prev)).map_err(io)?;
    }
    std::fs::rename(&temp, file).map_err(io)
}

/// `"90s"`, `"30m"`, `"6h"`, `"1d"` as a duration, for `--every`.
pub fn parse_every(text: &str) -> Result<Duration, DiscoverError> {
    let text = text.trim();
    let (digits, unit) = text.split_at(text.len().saturating_sub(1));
    let n: u64 = digits
        .parse()
        .map_err(|_| DiscoverError::Config(format!("`{text}` is not a duration like 30m or 6h")))?;
    let secs = match unit {
        "s" => n,
        "m" => n.saturating_mul(60),
        "h" => n.saturating_mul(3600),
        "d" => n.saturating_mul(86_400),
        _ => {
            return Err(DiscoverError::Config(format!(
                "`{text}` is not a duration like 30m or 6h"
            )));
        }
    };
    if secs == 0 {
        return Err(DiscoverError::Config(
            "--every must be more than zero".into(),
        ));
    }
    Ok(Duration::from_secs(secs))
}

/// A human report of what discovery found.
#[must_use]
pub fn render_report(plan: &Plan) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    for (provider, reason) in &plan.skipped {
        let _ = writeln!(out, "{provider}: left alone ({reason})");
    }
    for (name, candidate) in &plan.added {
        let _ = writeln!(
            out,
            "new:        {name}  window {}{}{}  (free chat model, no target uses it)",
            candidate.context,
            if candidate.tools { ", tools" } else { "" },
            if candidate.reasoning {
                ", reasoning"
            } else {
                ""
            },
        );
    }
    for name in &plan.removed {
        let _ = writeln!(
            out,
            "removed:    {name}  (the provider no longer lists it, or it is no longer free)"
        );
    }
    for (target, model) in &plan.gone {
        let _ = writeln!(
            out,
            "gone:       {target} uses {model}, which the dataset no longer lists"
        );
    }
    for drift in &plan.window_drift {
        let _ = writeln!(
            out,
            "window:     {} ({}) declares {}, the dataset says {}",
            drift.target, drift.model, drift.declared, drift.source
        );
    }
    for (provider, paid) in &plan.paid_new {
        let _ = writeln!(
            out,
            "new (paid): {provider}/{}  window {}  ${} in / ${} out per million  (not added; write a target by hand)",
            paid.candidate.id, paid.candidate.context, paid.input, paid.output
        );
    }
    for (provider, model) in &plan.unconfirmed {
        let _ = writeln!(
            out,
            "unconfirmed: {provider}/{model} is free in the dataset but the provider does not list it; not proposed"
        );
    }
    if !plan.needs_review() {
        out.push_str("nothing to review\n");
    }
    out
}

/// The same report as JSON, with a top-level `drift` flag.
#[must_use]
pub fn report_json(plan: &Plan) -> Value {
    serde_json::json!({
        "drift": plan.needs_review(),
        "new": plan.added.iter().map(|(name, c)| serde_json::json!({
            "target": name, "model": c.id, "context_window": c.context,
            "tools": c.tools, "vision": c.vision, "reasoning": c.reasoning,
        })).collect::<Vec<_>>(),
        "new_paid": plan.paid_new.iter().map(|(p, m)| serde_json::json!({
            "provider": p, "model": m.candidate.id, "context_window": m.candidate.context,
            "input": m.input, "output": m.output,
        })).collect::<Vec<_>>(),
        "removed": plan.removed,
        "gone": plan.gone.iter().map(|(t, m)| serde_json::json!({"target": t, "model": m})).collect::<Vec<_>>(),
        "window_drift": plan.window_drift.iter().map(|d| serde_json::json!({
            "target": d.target, "model": d.model, "declared": d.declared, "source": d.source,
        })).collect::<Vec<_>>(),
        "unconfirmed": plan.unconfirmed.iter().map(|(p, m)| serde_json::json!({"provider": p, "model": m})).collect::<Vec<_>>(),
        "skipped": plan.skipped.iter().map(|(p, r)| serde_json::json!({"provider": p, "reason": r})).collect::<Vec<_>>(),
    })
}
