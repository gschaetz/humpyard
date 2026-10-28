//! Ledger scale harness (spec: `performance`). Not part of the default run:
//!
//! ```text
//! LEDGER_SCALE_ROWS=1000000 cargo test --release --test ledger_scale -- --ignored --nocapture
//! ```
//!
//! Fills a ledger through its real write path with seeded synthetic rows, then times the operations the gateway
//! performs against it. Results are recorded in `docs/performance.md`.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test scaffolding
#![allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)] // reporting only
#![allow(clippy::too_many_lines)] // one linear script: fill, measure, report

use std::path::{Path, PathBuf};
use std::time::Instant;

use humpyard::ledger::{
    Attempt, Decision, DecisionQuery, Entry, GroupBy, Kind, Ledger, QuotaSample, SessionQuery,
    StatsQuery,
};

const DAY_MS: i64 = 86_400_000;
/// The row cap the server uses for `/v1/stats` and the session view.
const SERVER_ROW_CAP: u32 = 50_000;

fn env_u64(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

/// A small seeded generator (`SplitMix64`), so a run is reproducible without a dependency.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
}

const PROVIDERS: [&str; 8] = [
    "kilocode",
    "openrouter",
    "opencode-zen",
    "codestral",
    "groq",
    "nvidia",
    "ollama",
    "opencode",
];
const ROUTES: [&str; 6] = [
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
fn make_row(rng: &mut Rng, i: u64, ts_ms: i64) -> (Entry, Decision) {
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

fn size_of(path: &Path) -> u64 {
    ["", "-wal", "-shm"]
        .iter()
        .filter_map(|suffix| {
            let mut name = path.as_os_str().to_owned();
            name.push(suffix);
            std::fs::metadata(PathBuf::from(name)).ok()
        })
        .map(|m| m.len())
        .sum()
}

struct Report(Vec<(String, f64, String)>);

impl Report {
    fn time<T>(&mut self, label: &str, note: impl Into<String>, started: Instant, value: T) -> T {
        self.0.push((
            label.to_string(),
            started.elapsed().as_secs_f64() * 1000.0,
            note.into(),
        ));
        value
    }
}

async fn count(ledger: &Ledger, table: &str) -> i64 {
    let query = match table {
        "usage" => sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM usage"),
        "decisions" => sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM decisions"),
        "quota_samples" => sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM quota_samples"),
        other => panic!("unknown table {other}"),
    };
    query.fetch_one(ledger.pool()).await.unwrap()
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "scale harness; run explicitly with --ignored"]
async fn ledger_scale() {
    let rows = env_u64("LEDGER_SCALE_ROWS", 1_000_000);
    let days = i64::try_from(env_u64("LEDGER_SCALE_DAYS", 30)).unwrap();
    let seed = env_u64("LEDGER_SCALE_SEED", 1);
    let dir = std::env::temp_dir().join(format!("humpyard-ledger-scale-{rows}-{seed}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("ledger.db");
    let now_ms = i64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis(),
    )
    .unwrap();
    let start_ms = now_ms - days * DAY_MS;
    let mut report = Report(Vec::new());

    // Fill through the real queue and writer. Retention 0 keeps everything; a flush every batch keeps the
    // queue from dropping, which also makes this the sustained-write measurement.
    let ledger = Ledger::open_with(&path, 100_000, 0).await.unwrap();
    let mut rng = Rng(seed);
    let started = Instant::now();
    for i in 0..rows {
        let ts =
            start_ms + (i64::try_from(i).unwrap() * days * DAY_MS) / i64::try_from(rows).unwrap();
        let (entry, decision) = make_row(&mut rng, i, ts);
        ledger.record(entry);
        if i % 3 != 2 {
            ledger.record_decision(decision);
        }
        if i % 1_000 == 0 {
            ledger.record_quota(vec![QuotaSample {
                ts_ms: ts,
                provider: PROVIDERS[(i / 1_000 % 8) as usize].to_string(),
                window: "weekly".to_string(),
                percent: (i % 100) as f64,
                status: "ok".to_string(),
                resets_at: None,
            }]);
        }
        if ledger.queue_depth() > 60_000 {
            ledger.flush().await;
        }
    }
    ledger.flush().await;
    let fill = started.elapsed().as_secs_f64();
    assert_eq!(ledger.dropped(), 0, "the harness must not drop rows");
    let usage = count(&ledger, "usage").await;
    let decisions = count(&ledger, "decisions").await;
    let quota = count(&ledger, "quota_samples").await;
    let bytes = size_of(&path);
    ledger.shutdown().await;

    // Reopen: what a restart costs.
    let started = Instant::now();
    let ledger = Ledger::open_with(&path, 1_000, 0).await.unwrap();
    report.time("reopen the ledger", "", started, ());

    let day = now_ms - DAY_MS;
    let month = now_ms - days * DAY_MS;
    let started = Instant::now();
    let spend = ledger.spend_since(day).await.unwrap();
    report.time(
        "spend_since, last 1 day",
        format!("{} keys", spend.len()),
        started,
        (),
    );
    let started = Instant::now();
    let spend = ledger.spend_since(month).await.unwrap();
    report.time(
        "spend_since, whole window (budget hydration)",
        format!("{} keys", spend.len()),
        started,
        (),
    );

    for (group, name) in [
        (GroupBy::Route, "route"),
        (GroupBy::Target, "target"),
        (GroupBy::Endpoint, "endpoint"),
    ] {
        for (since, window) in [(day, "1 day"), (month, "whole window")] {
            let query = StatsQuery {
                key: None,
                since_ms: since,
                group_by: group,
                route: None,
                target: None,
                row_cap: SERVER_ROW_CAP,
            };
            let started = Instant::now();
            let stats = ledger.stats(&query).await.unwrap();
            report.time(
                &format!("stats by {name}, {window}"),
                format!("{} groups", stats.groups.len()),
                started,
                (),
            );
        }
    }
    for (since, window) in [(day, "1 day"), (month, "whole window")] {
        let started = Instant::now();
        let scan = ledger
            .session_scan(&SessionQuery {
                key: None,
                since_ms: since,
                row_cap: SERVER_ROW_CAP,
            })
            .await
            .unwrap();
        report.time(
            &format!("session scan, {window}"),
            format!("{} sessions", scan.sessions.len()),
            started,
            (),
        );
    }
    let started = Instant::now();
    let recent = ledger.entries_since(day).await.unwrap();
    report.time(
        "entries_since, last 1 day",
        format!("{} rows", recent.len()),
        started,
        (),
    );

    let newest = DecisionQuery {
        limit: 100,
        ..DecisionQuery::default()
    };
    let started = Instant::now();
    let found = ledger.decisions(&newest).await.unwrap();
    report.time(
        "decisions, newest 100",
        format!("{} rows", found.len()),
        started,
        (),
    );
    let by_route = DecisionQuery {
        limit: 100,
        route: Some("fast-thinker-public".to_string()),
        ..DecisionQuery::default()
    };
    let started = Instant::now();
    let found = ledger.decisions(&by_route).await.unwrap();
    report.time(
        "decisions, newest 100 of one route",
        format!("{} rows", found.len()),
        started,
        (),
    );
    let by_request = DecisionQuery {
        limit: 1,
        request_id: Some(format!("req-{}", (rows / 2) / 3 * 3)),
        ..DecisionQuery::default()
    };
    let started = Instant::now();
    let found = ledger.decisions(&by_request).await.unwrap();
    report.time(
        "decisions, one request id",
        format!("{} rows", found.len()),
        started,
        (),
    );

    // The prune the writer runs: decisions past retention (here, older than 7 days).
    let cutoff = now_ms - 7 * DAY_MS;
    let started = Instant::now();
    let removed = sqlx::query("DELETE FROM decisions WHERE ts_ms < ?")
        .bind(cutoff)
        .execute(ledger.pool())
        .await
        .unwrap()
        .rows_affected();
    report.time(
        "prune decisions older than 7 days",
        format!("{removed} rows"),
        started,
        (),
    );
    ledger.shutdown().await;

    println!("\n=== ledger scale: {rows} usage rows over {days} days, seed {seed} ===");
    println!("rows: usage {usage}, decisions {decisions}, quota_samples {quota}");
    println!(
        "sustained write through the ledger queue: {:.0} usage+decision rows/s ({fill:.1} s)",
        (usage + decisions) as f64 / fill
    );
    println!(
        "database size (db + wal + shm): {:.1} MiB",
        bytes as f64 / 1_048_576.0
    );
    println!("{:<50} {:>12}  note", "operation", "ms");
    for (label, ms, note) in &report.0 {
        println!("{label:<50} {ms:>12.1}  {note}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}
