//! Operational metrics: one in-process registry, many views (ADR 0016).
//!
//! Handlers record into the registry; encoders read it. The Prometheus text format is the first
//! view ([`encode`]); a status page later reads the same [`Metrics::snapshot`]. Label values come
//! only from the loaded configuration or from fixed sets: a string a client sent (the requested
//! model, a header, a tag) never becomes a label, so a hostile client cannot grow the series or
//! put content into them. Recording never blocks on I/O and never fails a request.

use std::collections::{BTreeMap, HashSet};
use std::fmt::Write as _;
use std::sync::atomic::{AtomicU64, Ordering};

use parking_lot::RwLock;

use crate::config::Config;
use crate::ledger::{Decision, Entry};

/// Prefix of every metric name.
const PREFIX: &str = "humpyard_";

/// Upper bounds of the duration histogram, in milliseconds. Model calls are slow, so the range
/// runs from 50 ms to two minutes.
const BUCKETS_MS: [u64; 12] = [
    50, 100, 250, 500, 1_000, 2_500, 5_000, 10_000, 20_000, 30_000, 60_000, 120_000,
];

/// Why a target was removed for capability (ADR 0019).
const CAPABILITY_REASONS: [&str; 3] = ["tools", "vision", "context"];

/// Client protocols a request can arrive on (the decision's `format`).
const PROTOCOLS: [&str; 3] = ["openai_chat", "openai_responses", "anthropic_messages"];

/// Outcomes that mean the gateway refused the request before any upstream call.
const REFUSALS: [&str; 8] = [
    "no_capable_target",
    "override_no_target",
    "unauthorized",
    "forbidden",
    "budget_exhausted",
    "no_eligible_target",
    "unknown_model",
    "bad_request",
];

/// What a metric measures.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Counter,
    Gauge,
    Histogram,
}

impl Kind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Counter => "counter",
            Self::Gauge => "gauge",
            Self::Histogram => "histogram",
        }
    }
}

/// A sample's value.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Int(u64),
    Float(f64),
}

/// One line of a metric: a name suffix (`_bucket`, `_sum`, `_count` or empty), labels, a value.
#[derive(Clone, Debug, PartialEq)]
pub struct Sample {
    pub suffix: &'static str,
    pub labels: Vec<(String, String)>,
    pub value: Value,
}

/// A metric with all its samples, ready for any view.
#[derive(Clone, Debug, PartialEq)]
pub struct Family {
    /// The full name, with the `humpyard_` prefix.
    pub name: String,
    pub help: &'static str,
    pub kind: Kind,
    pub samples: Vec<Sample>,
}

/// Builds a family for a value that is read at scrape time (a breaker state, a queue depth, ...).
/// `name` is given without the prefix.
pub fn gauge(name: &str, help: &'static str, samples: Vec<(Vec<(&str, String)>, Value)>) -> Family {
    scraped(name, help, Kind::Gauge, samples)
}

/// Like [`gauge`], for a monotonic value that is owned elsewhere (the ledger's drop count).
pub fn counter(
    name: &str,
    help: &'static str,
    samples: Vec<(Vec<(&str, String)>, Value)>,
) -> Family {
    scraped(name, help, Kind::Counter, samples)
}

fn scraped(
    name: &str,
    help: &'static str,
    kind: Kind,
    samples: Vec<(Vec<(&str, String)>, Value)>,
) -> Family {
    Family {
        name: format!("{PREFIX}{name}"),
        help,
        kind,
        samples: samples
            .into_iter()
            .map(|(labels, value)| Sample {
                suffix: "",
                labels: labels
                    .into_iter()
                    .map(|(k, v)| (k.to_string(), v))
                    .collect(),
                value,
            })
            .collect(),
    }
}

/// Settings fixed at startup.
#[derive(Clone, Copy, Debug)]
pub struct Settings {
    /// Add a `key` label to the four per-request counters.
    pub key_labels: bool,
    /// Most distinct `key` label values; keys past it are counted under `other`.
    pub key_label_limit: usize,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            key_labels: false,
            key_label_limit: 100,
        }
    }
}

/// The names the configuration defines; the only values a label may take besides fixed sets.
#[derive(Default)]
struct Vocabulary {
    routes: HashSet<String>,
    targets: HashSet<String>,
    providers: HashSet<String>,
    keys: HashSet<String>,
}

/// A bucketed duration series.
struct Histogram {
    /// Counts per bucket, not cumulative; the last slot is above every bound.
    buckets: [AtomicU64; BUCKETS_MS.len() + 1],
    sum_ms: AtomicU64,
}

impl Histogram {
    fn new() -> Self {
        Self {
            buckets: std::array::from_fn(|_| AtomicU64::new(0)),
            sum_ms: AtomicU64::new(0),
        }
    }

    fn observe(&self, ms: u64) {
        let slot = BUCKETS_MS
            .iter()
            .position(|bound| ms <= *bound)
            .unwrap_or(BUCKETS_MS.len());
        self.buckets[slot].fetch_add(1, Ordering::Relaxed);
        self.sum_ms.fetch_add(ms, Ordering::Relaxed);
    }
}

enum Cell {
    Count(AtomicU64),
    Hist(Histogram),
}

/// A metric the registry owns: its definition and one cell per label combination.
struct Owned {
    name: &'static str,
    help: &'static str,
    kind: Kind,
    labels: &'static [&'static str],
    /// Carries a trailing `key` label when [`Settings::key_labels`] is on.
    keyed: bool,
    series: RwLock<BTreeMap<Vec<String>, Cell>>,
}

impl Owned {
    const fn new(
        name: &'static str,
        help: &'static str,
        kind: Kind,
        labels: &'static [&'static str],
        keyed: bool,
    ) -> Self {
        Self {
            name,
            help,
            kind,
            labels,
            keyed,
            series: RwLock::new(BTreeMap::new()),
        }
    }

    fn add(&self, labels: &[String], n: u64) {
        self.with(labels, |cell| {
            if let Cell::Count(c) = cell {
                c.fetch_add(n, Ordering::Relaxed);
            }
        });
    }

    fn observe_ms(&self, labels: &[String], ms: u64) {
        self.with(labels, |cell| {
            if let Cell::Hist(h) = cell {
                h.observe(ms);
            }
        });
    }

    /// Runs `f` on the cell for `labels`, creating it on first sight. The common case takes only
    /// a shared lock; creation takes the exclusive lock for one insert.
    fn with(&self, labels: &[String], f: impl Fn(&Cell)) {
        if let Some(cell) = self.series.read().get(labels) {
            f(cell);
            return;
        }
        let mut series = self.series.write();
        let cell = series.entry(labels.to_vec()).or_insert_with(|| {
            if self.kind == Kind::Histogram {
                Cell::Hist(Histogram::new())
            } else {
                Cell::Count(AtomicU64::new(0))
            }
        });
        f(cell);
    }

    fn label_names(&self, with_key: bool) -> Vec<&'static str> {
        let mut names = self.labels.to_vec();
        if self.keyed && with_key {
            names.push("key");
        }
        names
    }

    fn snapshot(&self, with_key: bool) -> Family {
        let names = self.label_names(with_key);
        let mut samples = Vec::new();
        for (values, cell) in self.series.read().iter() {
            let labels: Vec<(String, String)> = names
                .iter()
                .zip(values)
                .map(|(k, v)| ((*k).to_string(), v.clone()))
                .collect();
            match cell {
                Cell::Count(c) => samples.push(Sample {
                    suffix: "",
                    labels,
                    value: Value::Int(c.load(Ordering::Relaxed)),
                }),
                Cell::Hist(h) => histogram_samples(h, &labels, &mut samples),
            }
        }
        Family {
            name: format!("{PREFIX}{}", self.name),
            help: self.help,
            kind: self.kind,
            samples,
        }
    }
}

fn histogram_samples(h: &Histogram, labels: &[(String, String)], out: &mut Vec<Sample>) {
    let mut cumulative = 0_u64;
    for (slot, count) in h.buckets.iter().enumerate() {
        cumulative += count.load(Ordering::Relaxed);
        let le = BUCKETS_MS
            .get(slot)
            .map_or_else(|| "+Inf".to_string(), |ms| seconds(*ms));
        let mut with_le = labels.to_vec();
        with_le.push(("le".to_string(), le));
        out.push(Sample {
            suffix: "_bucket",
            labels: with_le,
            value: Value::Int(cumulative),
        });
    }
    out.push(Sample {
        suffix: "_sum",
        labels: labels.to_vec(),
        value: Value::Float(seconds_f64(h.sum_ms.load(Ordering::Relaxed))),
    });
    out.push(Sample {
        suffix: "_count",
        labels: labels.to_vec(),
        value: Value::Int(cumulative),
    });
}

/// Milliseconds as a decimal number of seconds, exactly (no float): `1500` is `1.5`.
fn seconds(ms: u64) -> String {
    let (whole, frac) = (ms / 1000, ms % 1000);
    if frac == 0 {
        whole.to_string()
    } else {
        format!("{whole}.{frac:03}")
            .trim_end_matches('0')
            .to_string()
    }
}

fn seconds_f64(ms: u64) -> f64 {
    seconds(ms).parse().unwrap_or(0.0)
}

/// The registry.
pub struct Metrics {
    settings: Settings,
    vocab: RwLock<Vocabulary>,
    /// Key ids already given their own `key` label value.
    seen_keys: RwLock<HashSet<String>>,
    requests: Owned,
    duration: Owned,
    tokens: Owned,
    cost: Owned,
    failovers: Owned,
    blocked: Owned,
    skips: Owned,
    retry_after: Owned,
    capability: Owned,
    refusals: Owned,
    reloads: Owned,
}

impl Default for Metrics {
    fn default() -> Self {
        Self::new(Settings::default())
    }
}

impl Metrics {
    pub fn new(settings: Settings) -> Self {
        Self {
            settings,
            vocab: RwLock::new(Vocabulary::default()),
            seen_keys: RwLock::new(HashSet::new()),
            requests: Owned::new(
                "requests_total",
                "Requests handled, by protocol, route, serving target and provider, and status class.",
                Kind::Counter,
                &["protocol", "route", "target", "provider", "status_class"],
                true,
            ),
            duration: Owned::new(
                "request_duration_seconds",
                "Time from a request's arrival to its response (headers, for streams), for requests that reached routing.",
                Kind::Histogram,
                &["route", "target"],
                false,
            ),
            tokens: Owned::new(
                "tokens_total",
                "Tokens reported by providers, by direction: input (fresh), output, cached_input and cache_creation.",
                Kind::Counter,
                &["route", "target", "direction"],
                true,
            ),
            cost: Owned::new(
                "cost_micro_usd_total",
                "Cost of provider calls in millionths of a US dollar, at the configured prices.",
                Kind::Counter,
                &["route", "target"],
                true,
            ),
            failovers: Owned::new(
                "failovers_total",
                "Attempts that failed and were followed by another attempt.",
                Kind::Counter,
                &["route", "from_target", "to_target"],
                false,
            ),
            blocked: Owned::new(
                "fallback_blocked_total",
                "Requests that failed upstream on a strict route that had excluded targets: a fallback the route's allowed_classes did not permit.",
                Kind::Counter,
                &["route"],
                false,
            ),
            skips: Owned::new(
                "health_skips_total",
                "Requests in which a target was left out of the plan because every endpoint was cooling down and another target remained.",
                Kind::Counter,
                &["route", "target"],
                false,
            ),
            retry_after: Owned::new(
                "retry_after_honored_total",
                "Failures whose stated retry time cooled the endpoint for exactly that long, by provider and where the time was read from (body now, header once the engine exposes it).",
                Kind::Counter,
                &["provider", "source"],
                false,
            ),
            capability: Owned::new(
                "capability_skips_total",
                "Requests in which a target was removed from the plan because none of its endpoints could serve the request, by reason (tools, vision, context).",
                Kind::Counter,
                &["route", "target", "reason"],
                false,
            ),
            refusals: Owned::new(
                "refusals_total",
                "Requests the gateway refused before any upstream call, by reason.",
                Kind::Counter,
                &["reason"],
                true,
            ),
            reloads: Owned::new(
                "config_reload_total",
                "Configuration reload attempts, by result.",
                Kind::Counter,
                &["result"],
                false,
            ),
        }
    }

    /// Replaces the names labels may take with the ones `config` defines. Series already recorded
    /// are kept, because a counter must not reset on a reload.
    pub fn set_vocabulary(&self, config: &Config) {
        let mut vocab = self.vocab.write();
        vocab.routes = config.routes.keys().cloned().collect();
        vocab.targets = config.targets.keys().cloned().collect();
        vocab.providers = config.providers.keys().cloned().collect();
        vocab.keys = config.keys.keys().cloned().collect();
    }

    /// Records a finished request: the one place a request's route, serving target, status and
    /// attempts are all known.
    pub fn observe_decision(&self, d: &Decision) {
        let (route, target, provider) = {
            let vocab = self.vocab.read();
            (
                pick(&vocab.routes, d.route.as_deref()),
                pick(&vocab.targets, d.served_target.as_deref()),
                pick(&vocab.providers, d.served_provider.as_deref()),
            )
        };
        let protocol = fixed(&PROTOCOLS, &d.format);
        let key = self.key_label(d.key_id.as_deref());

        let mut labels = vec![
            protocol,
            route.clone(),
            target.clone(),
            provider,
            status_class(d.status),
        ];
        labels.extend(key.clone());
        self.requests.add(&labels, 1);

        if d.route.is_some() {
            self.duration
                .observe_ms(&[route.clone(), target], d.latency_ms);
        }
        self.observe_failovers(&route, d);
        self.observe_health_skips(&route, d);
        self.observe_retry_after(d);
        self.observe_capability_skips(&route, d);
        // A strict route that failed after calling its allowed targets: the excluded ones are the
        // fallback it did not take.
        if !d.excluded_targets.is_empty()
            && !d.attempts.is_empty()
            && d.outcome != "ok"
            && d.outcome != "cancelled"
        {
            self.blocked.add(std::slice::from_ref(&route), 1);
        }
        if REFUSALS.contains(&d.outcome.as_str()) {
            let mut labels = vec![d.outcome.clone()];
            labels.extend(key);
            self.refusals.add(&labels, 1);
        }
    }

    fn observe_capability_skips(&self, route: &str, d: &Decision) {
        let vocab = self.vocab.read();
        for incapable in &d.incapable_targets {
            self.capability.add(
                &[
                    route.to_string(),
                    pick(&vocab.targets, Some(&incapable.target)),
                    fixed(&CAPABILITY_REASONS, &incapable.reason),
                ],
                1,
            );
        }
    }

    /// Honored retry times, by provider. The time is read from the error body today; the label is
    /// there so the header can be told apart when the engine exposes it.
    fn observe_retry_after(&self, d: &Decision) {
        let vocab = self.vocab.read();
        for attempt in d.attempts.iter().filter(|a| a.retry_after_ms.is_some()) {
            self.retry_after.add(
                &[
                    pick(&vocab.providers, Some(&attempt.provider)),
                    "body".to_string(),
                ],
                1,
            );
        }
    }

    fn observe_health_skips(&self, route: &str, d: &Decision) {
        let vocab = self.vocab.read();
        for target in &d.cold_targets {
            self.skips
                .add(&[route.to_string(), pick(&vocab.targets, Some(target))], 1);
        }
    }

    fn observe_failovers(&self, route: &str, d: &Decision) {
        let vocab = self.vocab.read();
        for pair in d.attempts.windows(2) {
            if pair[0].outcome != "ok" {
                self.failovers.add(
                    &[
                        route.to_string(),
                        pick(&vocab.targets, Some(&pair[0].target)),
                        pick(&vocab.targets, Some(&pair[1].target)),
                    ],
                    1,
                );
            }
        }
    }

    /// Records one provider call's tokens and cost (the same row the ledger stores).
    pub fn observe_usage(&self, e: &Entry) {
        let (route, target) = {
            let vocab = self.vocab.read();
            (
                pick(&vocab.routes, Some(&e.route)),
                pick(&vocab.targets, Some(&e.target)),
            )
        };
        let key = self.key_label(e.key_id.as_deref());
        // `input` is the fresh part of the prompt; the cached and cache-creation parts are separate
        // directions, so a dashboard can derive the cache-hit ratio (ADR 0016: fixed label values).
        for (direction, n) in [
            ("input", e.input_tokens),
            ("output", e.output_tokens),
            ("cached_input", e.cached_input_tokens),
            ("cache_creation", e.cache_creation_tokens),
        ] {
            let mut labels = vec![route.clone(), target.clone(), direction.to_string()];
            labels.extend(key.clone());
            self.tokens.add(&labels, n);
        }
        let mut labels = vec![route, target];
        labels.extend(key);
        self.cost.add(&labels, e.cost_micro_usd);
    }

    /// Records a configuration reload attempt.
    pub fn observe_reload(&self, ok: bool) {
        self.reloads
            .add(&[if ok { "ok" } else { "rejected" }.to_string()], 1);
    }

    /// The `key` label value for a request, or `None` when key labels are off.
    fn key_label(&self, key: Option<&str>) -> Option<String> {
        if !self.settings.key_labels {
            return None;
        }
        let Some(key) = key else {
            return Some("none".to_string());
        };
        if !self.vocab.read().keys.contains(key) {
            return Some("other".to_string());
        }
        if self.seen_keys.read().contains(key) {
            return Some(key.to_string());
        }
        let mut seen = self.seen_keys.write();
        if seen.contains(key) || seen.len() < self.settings.key_label_limit {
            seen.insert(key.to_string());
            Some(key.to_string())
        } else {
            Some("other".to_string())
        }
    }

    /// Every metric with its samples: the registry's own, then `scraped` (values read at scrape
    /// time from their owners). This is what every view renders from.
    pub fn snapshot(&self, scraped: Vec<Family>) -> Vec<Family> {
        let with_key = self.settings.key_labels;
        let mut families: Vec<Family> = [
            &self.requests,
            &self.duration,
            &self.tokens,
            &self.cost,
            &self.failovers,
            &self.blocked,
            &self.skips,
            &self.retry_after,
            &self.capability,
            &self.refusals,
            &self.reloads,
        ]
        .iter()
        .map(|m| m.snapshot(with_key))
        .collect();
        families.extend(scraped);
        families
    }

    /// The Prometheus text exposition of [`Metrics::snapshot`].
    pub fn render(&self, scraped: Vec<Family>) -> String {
        encode(&self.snapshot(scraped))
    }
}

/// `value` if the configuration defines it, `none` when absent, `other` when unknown.
fn pick(known: &HashSet<String>, value: Option<&str>) -> String {
    match value {
        None => "none".to_string(),
        Some(v) if known.contains(v) => v.to_string(),
        Some(_) => "other".to_string(),
    }
}

/// `value` if it is one of the fixed set, else `other`.
fn fixed(set: &[&str], value: &str) -> String {
    if set.contains(&value) {
        value.to_string()
    } else {
        "other".to_string()
    }
}

fn status_class(status: u16) -> String {
    match status / 100 {
        class @ 1..=5 => format!("{class}xx"),
        _ => "other".to_string(),
    }
}

/// Prometheus text exposition format 0.0.4.
pub fn encode(families: &[Family]) -> String {
    let mut out = String::new();
    for family in families {
        let _ = writeln!(out, "# HELP {} {}", family.name, escape_help(family.help));
        let _ = writeln!(out, "# TYPE {} {}", family.name, family.kind.as_str());
        for sample in &family.samples {
            let _ = write!(out, "{}{}", family.name, sample.suffix);
            if !sample.labels.is_empty() {
                out.push('{');
                for (i, (k, v)) in sample.labels.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    let _ = write!(out, "{k}=\"{}\"", escape_label(v));
                }
                out.push('}');
            }
            match &sample.value {
                Value::Int(n) => {
                    let _ = writeln!(out, " {n}");
                }
                Value::Float(f) => {
                    let _ = writeln!(out, " {f}");
                }
            }
        }
    }
    out
}

fn escape_help(text: &str) -> String {
    text.replace('\\', "\\\\").replace('\n', "\\n")
}

fn escape_label(text: &str) -> String {
    text.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
}
