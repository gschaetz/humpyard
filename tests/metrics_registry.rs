//! The metrics registry: valid exposition, bounded labels, opt-in key labels, monotonic counters.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test scaffolding

use std::collections::BTreeMap;

use humpyard::config::Config;
use humpyard::ledger::{Attempt, Decision, Entry, Kind};
use humpyard::metrics::{self, Metrics, Settings, Value};

const CONFIG: &str = r#"
listen = "127.0.0.1:8080"

[providers.groq]
base_url = "https://api.example.com/v1/"
api_key_env = "GROQ_KEY"

[targets.fast]
endpoints = [{ provider = "groq", model = "llama" }]

[targets.smart]
endpoints = [{ provider = "groq", model = "big" }]

[routes.auto]
type = "stage_router"
efficient = ["fast"]
capable = ["smart"]

[keys.alice]
sha256 = "sha256:0000000000000000000000000000000000000000000000000000000000000001"

[keys.bob]
sha256 = "sha256:0000000000000000000000000000000000000000000000000000000000000002"

[keys.carol]
sha256 = "sha256:0000000000000000000000000000000000000000000000000000000000000003"
"#;

fn config() -> Config {
    Config::from_toml(CONFIG, |_| Some("secret".to_string())).unwrap()
}

fn registry(settings: Settings) -> Metrics {
    let m = Metrics::new(settings);
    m.set_vocabulary(&config());
    m
}

fn keyed() -> Settings {
    Settings {
        key_labels: true,
        ..Settings::default()
    }
}

fn decision(key: Option<&str>) -> Decision {
    Decision {
        request_id: "r1".into(),
        format: "openai_chat".into(),
        key_id: key.map(str::to_string),
        route: Some("auto".into()),
        served_target: Some("fast".into()),
        served_provider: Some("groq".into()),
        status: 200,
        outcome: "ok".into(),
        latency_ms: 120,
        ..Decision::default()
    }
}

fn entry(key: Option<&str>) -> Entry {
    Entry {
        ts_ms: 1,
        key_id: key.map(str::to_string),
        session_id: None,
        route: "auto".into(),
        target: "fast".into(),
        provider: "groq".into(),
        model: "llama".into(),
        kind: Kind::Answer,
        input_tokens: 10,
        cached_input_tokens: 0,
        cache_creation_tokens: 0,
        output_tokens: 5,
        reasoning_tokens: 0,
        cost_micro_usd: 700,
        outcome: "ok".into(),
        usage_missing: false,
        request_id: Some("r1".into()),
        signals: None,
    }
}

/// Splits `name{a="b",c="d"} 12` into the name, its labels and the value; panics on bad syntax.
fn parse_sample(line: &str) -> (String, BTreeMap<String, String>, f64) {
    let (head, value) = line.rsplit_once(' ').expect("a sample has a value");
    let value: f64 = value.parse().expect("the value is a number");
    let (name, labels) = match head.split_once('{') {
        None => (head.to_string(), BTreeMap::new()),
        Some((name, rest)) => {
            let body = rest.strip_suffix('}').expect("labels end with a brace");
            let mut labels = BTreeMap::new();
            let mut chars = body.chars().peekable();
            while chars.peek().is_some() {
                let key: String = chars.by_ref().take_while(|c| *c != '=').collect();
                assert_eq!(chars.next(), Some('"'), "a label value is quoted");
                let mut val = String::new();
                loop {
                    match chars.next().expect("the value is closed") {
                        '\\' => val.push(chars.next().expect("an escaped character")),
                        '"' => break,
                        c => val.push(c),
                    }
                }
                labels.insert(key, val);
                if chars.peek() == Some(&',') {
                    chars.next();
                }
            }
            (name.to_string(), labels)
        }
    };
    (name, labels, value)
}

/// Checks the text format strictly and returns every sample.
fn parse(text: &str) -> Vec<(String, BTreeMap<String, String>, f64)> {
    let mut typed: BTreeMap<String, String> = BTreeMap::new();
    let mut helped = std::collections::BTreeSet::new();
    let mut samples = Vec::new();
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("# HELP ") {
            helped.insert(rest.split(' ').next().unwrap().to_string());
        } else if let Some(rest) = line.strip_prefix("# TYPE ") {
            let (name, kind) = rest.split_once(' ').unwrap();
            assert!(["counter", "gauge", "histogram"].contains(&kind));
            typed.insert(name.to_string(), kind.to_string());
        } else {
            assert!(
                !line.starts_with('#') && !line.is_empty(),
                "stray line {line:?}"
            );
            let sample = parse_sample(line);
            let base = ["_bucket", "_sum", "_count"]
                .iter()
                .find_map(|s| {
                    sample
                        .0
                        .strip_suffix(s)
                        .filter(|b| typed.get(*b).is_some_and(|k| k == "histogram"))
                })
                .unwrap_or(&sample.0)
                .to_string();
            assert!(typed.contains_key(&base), "{base} has a TYPE line first");
            assert!(helped.contains(&base), "{base} has a HELP line");
            samples.push(sample);
        }
    }
    samples
}

fn find<'a>(
    samples: &'a [(String, BTreeMap<String, String>, f64)],
    name: &str,
) -> Vec<&'a (String, BTreeMap<String, String>, f64)> {
    samples.iter().filter(|s| s.0 == name).collect()
}

#[test]
fn the_exposition_parses_strictly_and_histograms_are_cumulative() {
    let m = registry(Settings::default());
    for ms in [10, 60, 400, 3_000, 200_000] {
        let mut d = decision(None);
        d.latency_ms = ms;
        m.observe_decision(&d);
    }
    let samples = parse(&m.render(vec![]));
    let buckets = find(&samples, "humpyard_request_duration_seconds_bucket");
    assert_eq!(buckets.len(), 13, "twelve bounds and +Inf");
    let counts: Vec<f64> = buckets.iter().map(|b| b.2).collect();
    assert!(
        counts.windows(2).all(|w| w[0] <= w[1]),
        "non-decreasing: {counts:?}"
    );
    assert_eq!(buckets.last().unwrap().1["le"], "+Inf");
    let count = find(&samples, "humpyard_request_duration_seconds_count")[0].2;
    assert!((count - 5.0).abs() < f64::EPSILON);
    assert!((counts.last().unwrap() - count).abs() < f64::EPSILON);
    let sum = find(&samples, "humpyard_request_duration_seconds_sum")[0].2;
    assert!(
        (sum - 203.47).abs() < 1e-9,
        "10+60+400+3000+200000 ms = 203.47 s, got {sum}"
    );
}

#[test]
fn a_clients_strings_never_become_labels() {
    let m = registry(Settings::default());
    let hostile = "evil\"model\n{route=\"x\"}\\";
    let mut d = decision(Some(hostile));
    d.format = hostile.into();
    d.requested_model = hostile.into();
    d.route = Some(hostile.into());
    d.served_target = Some(hostile.into());
    d.served_provider = Some(hostile.into());
    d.attempts = vec![
        Attempt {
            target: hostile.into(),
            provider: "groq".into(),
            model: hostile.into(),
            outcome: "rate_limit".into(),
            ms: 1,
            class: None,
            retry_after_ms: None,
            message: None,
        },
        Attempt {
            target: "fast".into(),
            provider: "groq".into(),
            model: "llama".into(),
            outcome: "ok".into(),
            ms: 1,
            class: None,
            retry_after_ms: None,
            message: None,
        },
    ];
    m.observe_decision(&d);
    let mut e = entry(Some(hostile));
    e.route = hostile.into();
    e.target = hostile.into();
    m.observe_usage(&e);

    let text = m.render(vec![]);
    assert!(
        !text.contains("evil"),
        "no client string reaches the output:\n{text}"
    );
    let samples = parse(&text);
    let requests = find(&samples, "humpyard_requests_total");
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].1["route"], "other");
    assert_eq!(requests[0].1["protocol"], "other");
}

#[test]
fn a_request_that_matched_no_route_is_counted_under_none() {
    let m = registry(Settings::default());
    let mut d = decision(None);
    d.route = None;
    d.served_target = None;
    d.served_provider = None;
    d.status = 404;
    d.outcome = "unknown_model".into();
    d.requested_model = "whatever-the-client-sent".into();
    m.observe_decision(&d);
    let text = m.render(vec![]);
    assert!(!text.contains("whatever-the-client-sent"));
    let samples = parse(&text);
    let requests = find(&samples, "humpyard_requests_total");
    assert_eq!(requests[0].1["route"], "none");
    assert_eq!(requests[0].1["status_class"], "4xx");
    assert!(
        find(&samples, "humpyard_request_duration_seconds_count").is_empty(),
        "refusals before routing are not timed"
    );
    let refusals = find(&samples, "humpyard_refusals_total");
    assert_eq!(refusals[0].1["reason"], "unknown_model");
}

#[test]
fn key_labels_are_off_by_default() {
    let m = registry(Settings::default());
    m.observe_decision(&decision(Some("alice")));
    m.observe_usage(&entry(Some("alice")));
    let text = m.render(vec![]);
    assert!(!text.contains("alice") && !text.contains("key="), "{text}");
}

#[test]
fn key_labels_go_on_four_counters_and_never_on_a_histogram() {
    let m = registry(keyed());
    let mut refused = decision(Some("alice"));
    refused.outcome = "budget_exhausted".into();
    refused.status = 402;
    m.observe_decision(&refused);
    m.observe_decision(&decision(Some("alice")));
    m.observe_usage(&entry(Some("alice")));
    let samples = parse(&m.render(vec![]));
    for name in [
        "humpyard_requests_total",
        "humpyard_tokens_total",
        "humpyard_cost_micro_usd_total",
        "humpyard_refusals_total",
    ] {
        let found = find(&samples, name);
        assert!(!found.is_empty(), "{name}");
        assert!(found.iter().all(|s| s.1["key"] == "alice"), "{name}");
    }
    for name in [
        "humpyard_request_duration_seconds_bucket",
        "humpyard_failovers_total",
        "humpyard_config_reload_total",
    ] {
        assert!(
            find(&samples, name)
                .iter()
                .all(|s| !s.1.contains_key("key")),
            "{name}"
        );
    }
}

#[test]
fn distinct_key_labels_are_capped() {
    let m = registry(Settings {
        key_labels: true,
        key_label_limit: 2,
    });
    for key in ["alice", "bob", "carol", "carol", "stranger"] {
        m.observe_decision(&decision(Some(key)));
    }
    let samples = parse(&m.render(vec![]));
    let mut keys: Vec<(String, f64)> = find(&samples, "humpyard_requests_total")
        .iter()
        .map(|s| (s.1["key"].clone(), s.2))
        .collect();
    keys.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(
        keys,
        vec![
            ("alice".to_string(), 1.0),
            ("bob".to_string(), 1.0),
            ("other".to_string(), 3.0)
        ],
        "a third configured key and an unknown one share `other`"
    );
}

#[test]
fn tokens_and_cost_are_counted_per_call() {
    let m = registry(Settings::default());
    m.observe_usage(&entry(None));
    m.observe_usage(&entry(None));
    let samples = parse(&m.render(vec![]));
    let tokens: BTreeMap<String, f64> = find(&samples, "humpyard_tokens_total")
        .iter()
        .map(|s| (s.1["direction"].clone(), s.2))
        .collect();
    assert_eq!(tokens["input"], 20.0);
    assert_eq!(tokens["output"], 10.0);
    assert_eq!(find(&samples, "humpyard_cost_micro_usd_total")[0].2, 1400.0);
}

#[test]
fn cached_and_cache_creation_tokens_are_their_own_directions() {
    let m = registry(Settings::default());
    let mut e = entry(None);
    e.input_tokens = 100;
    e.cached_input_tokens = 700;
    e.cache_creation_tokens = 200;
    e.output_tokens = 50;
    m.observe_usage(&e);
    let samples = parse(&m.render(vec![]));
    let by_direction: BTreeMap<String, f64> = find(&samples, "humpyard_tokens_total")
        .iter()
        .map(|s| (s.1["direction"].clone(), s.2))
        .collect();
    assert_eq!(by_direction["input"], 100.0);
    assert_eq!(by_direction["cached_input"], 700.0);
    assert_eq!(by_direction["cache_creation"], 200.0);
    assert_eq!(by_direction["output"], 50.0);
    // The cache-hit ratio a dashboard derives: cached over input + cached + cache creation.
    let ratio = by_direction["cached_input"]
        / (by_direction["input"] + by_direction["cached_input"] + by_direction["cache_creation"]);
    assert!((ratio - 0.7).abs() < 1e-9);
}

#[test]
fn a_failed_attempt_followed_by_another_is_a_failover() {
    let m = registry(Settings::default());
    let mut d = decision(None);
    d.attempts = vec![
        Attempt {
            target: "smart".into(),
            provider: "groq".into(),
            model: "big".into(),
            outcome: "timeout".into(),
            ms: 5,
            class: None,
            retry_after_ms: None,
            message: None,
        },
        Attempt {
            target: "fast".into(),
            provider: "groq".into(),
            model: "llama".into(),
            outcome: "ok".into(),
            ms: 5,
            class: None,
            retry_after_ms: None,
            message: None,
        },
    ];
    m.observe_decision(&d);
    let samples = parse(&m.render(vec![]));
    let failovers = find(&samples, "humpyard_failovers_total");
    assert_eq!(failovers.len(), 1);
    assert_eq!(failovers[0].1["from_target"], "smart");
    assert_eq!(failovers[0].1["to_target"], "fast");
}

#[test]
fn counters_never_decrease_and_survive_a_vocabulary_change() {
    let m = registry(Settings::default());
    let mut last = 0.0;
    for _ in 0..3 {
        m.observe_decision(&decision(None));
        m.set_vocabulary(&config());
        let samples = parse(&m.render(vec![]));
        let now = find(&samples, "humpyard_requests_total")[0].2;
        assert!(now > last);
        last = now;
    }
    m.observe_reload(true);
    m.observe_reload(false);
    let samples = parse(&m.render(vec![]));
    let reloads: BTreeMap<String, f64> = find(&samples, "humpyard_config_reload_total")
        .iter()
        .map(|s| (s.1["result"].clone(), s.2))
        .collect();
    assert_eq!(reloads["ok"], 1.0);
    assert_eq!(reloads["rejected"], 1.0);
}

#[test]
fn scraped_values_join_the_same_output_and_the_snapshot_equals_the_text() {
    let m = registry(Settings::default());
    m.observe_decision(&decision(None));
    let scraped = || {
        vec![
            metrics::gauge(
                "ledger_queue_depth",
                "Entries waiting for the ledger writer.",
                vec![(vec![], Value::Int(3))],
            ),
            metrics::gauge(
                "quota_percent",
                "Percent of a provider's plan window used.",
                vec![(
                    vec![
                        ("provider", "opencode".to_string()),
                        ("window", "weekly".to_string()),
                    ],
                    Value::Float(42.5),
                )],
            ),
        ]
    };
    let text = m.render(scraped());
    let samples = parse(&text);
    assert_eq!(find(&samples, "humpyard_ledger_queue_depth")[0].2, 3.0);
    assert_eq!(find(&samples, "humpyard_quota_percent")[0].2, 42.5);
    assert_eq!(metrics::encode(&m.snapshot(scraped())), text);
}

fn judged(signals: humpyard::signals::Signals) -> Entry {
    let mut e = entry(None);
    e.signals = Some(signals);
    e
}

fn signal_count(samples: &[(String, BTreeMap<String, String>, f64)], signal: &str) -> Option<f64> {
    find(samples, "humpyard_response_signals_total")
        .into_iter()
        .find(|s| s.1["signal"] == signal)
        .map(|s| s.2)
}

#[test]
fn each_response_signal_raises_its_own_series() {
    use humpyard::signals::Signals;
    let m = registry(Settings::default());
    m.observe_usage(&judged(Signals {
        empty_output: true,
        ..Signals::default()
    }));
    m.observe_usage(&judged(Signals {
        truncated_output: true,
        ..Signals::default()
    }));
    m.observe_usage(&judged(Signals {
        bad_tool_call: true,
        ..Signals::default()
    }));
    m.observe_usage(&judged(Signals {
        cut_stream: true,
        ..Signals::default()
    }));
    let samples = parse(&m.render(vec![]));
    for signal in [
        "empty_output",
        "truncated_output",
        "bad_tool_call",
        "cut_stream",
    ] {
        assert_eq!(signal_count(&samples, signal), Some(1.0), "{signal}");
    }
    assert_eq!(
        find(&samples, "humpyard_response_calls_observed_total")[0].2,
        4.0
    );
}

#[test]
fn a_clean_call_raises_only_the_observed_counter() {
    let m = registry(Settings::default());
    m.observe_usage(&judged(humpyard::signals::Signals::default()));
    let samples = parse(&m.render(vec![]));
    assert_eq!(
        find(&samples, "humpyard_response_calls_observed_total")[0].2,
        1.0
    );
    assert_eq!(find(&samples, "humpyard_response_signals_total").len(), 0);
}

#[test]
fn a_call_that_was_not_judged_is_not_counted() {
    let m = registry(Settings::default());
    m.observe_usage(&entry(None));
    let samples = parse(&m.render(vec![]));
    assert_eq!(
        find(&samples, "humpyard_response_calls_observed_total").len(),
        0
    );
}

#[test]
fn response_signals_use_the_bounded_labels() {
    let m = registry(Settings::default());
    let mut e = judged(humpyard::signals::Signals {
        empty_output: true,
        ..humpyard::signals::Signals::default()
    });
    e.route = "evil-route".into();
    e.target = "evil-target".into();
    m.observe_usage(&e);
    let text = m.render(vec![]);
    assert!(!text.contains("evil"), "{text}");
    let samples = parse(&text);
    let s = &find(&samples, "humpyard_response_signals_total")[0];
    assert_eq!(s.1["route"], "other");
    assert_eq!(s.1["target"], "other");
}
