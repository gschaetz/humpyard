//! Seeded synthetic ledger rows, shared by the scale harness and the isolation tests. A run is reproducible from
//! its seed without a dependency: the same seed and count give the same rows, apart from their timestamps, which
//! the caller chooses.
#![allow(clippy::cast_possible_truncation)] // indices into short constant tables

use humpyard::ledger::{Attempt, Decision, Entry, Kind};

/// A small seeded generator (`SplitMix64`), so a run is reproducible without a dependency.
pub struct Rng(pub u64);

impl Rng {
    pub fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    pub fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
}

pub const PROVIDERS: [&str; 8] = [
    "kilocode",
    "openrouter",
    "opencode-zen",
    "codestral",
    "groq",
    "nvidia",
    "ollama",
    "opencode",
];
pub const ROUTES: [&str; 6] = [
    "fast-thinker-public",
    "deep-thinker-public",
    "fast-thinker-paid",
    "deep-thinker-paid",
    "coding-paid",
    "fast-thinker-private",
];

fn outcome(rng: &mut Rng) -> &'static str {
    match rng.below(100) {
        0..=84 => "ok",
        85..=92 => "unavailable",
        93..=96 => "rate_limit",
        _ => "error",
    }
}

/// One synthetic request: its usage entry and its decision, chronological by `ts_ms`.
pub fn make_row(rng: &mut Rng, i: u64, ts_ms: i64) -> (Entry, Decision) {
    let provider = PROVIDERS[rng.below(PROVIDERS.len() as u64) as usize];
    let target = format!("{provider}-model-{}", rng.below(8));
    let route = ROUTES[rng.below(ROUTES.len() as u64) as usize];
    let key = format!("key-{:02}", rng.below(30));
    let session = (rng.below(100) < 70).then(|| format!("sess-{}", rng.below(20_000)));
    let result = outcome(rng);
    let request_id = format!("req-{i}");
    let input = 200 + rng.below(8_000);
    let output = 20 + rng.below(1_500);
    let entry = Entry {
        ts_ms,
        key_id: Some(key.clone()),
        session_id: session.clone(),
        route: route.to_string(),
        target: target.clone(),
        provider: provider.to_string(),
        model: format!("model-{}", rng.below(8)),
        kind: Kind::Answer,
        input_tokens: input,
        cached_input_tokens: rng.below(input),
        cache_creation_tokens: 0,
        output_tokens: output,
        reasoning_tokens: rng.below(output),
        cost_micro_usd: if provider == "opencode" {
            rng.below(5_000)
        } else {
            0
        },
        outcome: if result == "ok" { "ok" } else { "error" }.to_string(),
        usage_missing: false,
        request_id: Some(request_id.clone()),
        signals: None,
    };
    let attempts = (0..=rng.below(2))
        .map(|n| Attempt {
            target: format!("{provider}-model-{n}"),
            provider: provider.to_string(),
            model: format!("model-{n}"),
            outcome: if n == 0 { result } else { "ok" }.to_string(),
            ms: 100 + rng.below(4_000),
            class: Some("public".to_string()),
            retry_after_ms: None,
            message: None,
        })
        .collect();
    let decision = Decision {
        request_id,
        ts_ms,
        key_id: Some(key),
        session_id: session,
        format: "openai_chat".to_string(),
        requested_model: route.to_string(),
        route: Some(route.to_string()),
        source: Some("requested_model".to_string()),
        eligible_targets: (0..6).map(|n| format!("{provider}-model-{n}")).collect(),
        attempts,
        status: 200,
        outcome: "ok".to_string(),
        served_target: Some(target),
        served_provider: Some(provider.to_string()),
        latency_ms: 200 + rng.below(6_000),
        ..Decision::default()
    };
    (entry, decision)
}
