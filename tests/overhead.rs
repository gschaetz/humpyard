//! Gateway overhead harness (spec: `performance`). Not part of the default run:
//!
//! ```text
//! cargo test --release --test overhead -- --ignored --nocapture
//! ```
//!
//! Sends the same request straight to a mock upstream and through the gateway, at several concurrencies and with
//! and without a fixed upstream delay, and reports the latency percentiles, the difference and requests per second.
//! Everything runs in one process on loopback, so it measures the gateway's own work and not a network.
//!
//! It also reports each absolute budget of `docs/performance.md` as met, exceeded or not checked. A run fails on an
//! exceeded budget only when `HUMPYARD_BUDGET_ENFORCE=1` (the release check on the reference machine); shared CI
//! runners are too noisy for absolute numbers, so CI relies on the relative isolation test instead. A budget that
//! could not be checked fails an enforced run rather than pass without having been enforced.
//!
//! Throughput is not a budget of this harness: one run compared with a recorded figure cannot resolve a 10%
//! tolerance (`docs/performance.md`, Run 3). `scripts/compare-throughput.sh` compares it with the previous
//! release's harness in alternating runs, reading the gateway's requests per second from the table this prints.
//!
//! - `OVERHEAD_BUDGET_P50_MS`, `OVERHEAD_BUDGET_P99_MS`: replace a recorded limit, to check the failing path with a
//!   tightened one. The recorded values are the constants below, and they change only with the documentation.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test scaffolding
#![allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)] // reporting only

mod common;

use std::time::Duration;

use common::load::{run, run_stream};
use common::mock::{STREAM_CHUNKS, STREAM_GAP, mock_upstream};
use common::serve;
use humpyard::config::Config;
use humpyard::server;

async fn gateway(upstream: &str, dir: &std::path::Path) -> String {
    let toml = format!(
        r#"listen = "127.0.0.1:0"
[ledger]
path = "{}"
[providers.p]
base_url = "{upstream}"
api_key_env = "PATH"
max_retries = 0
[targets.t]
endpoints = [{{ provider = "p", model = "m", price = {{ input = 0.0, output = 0.0 }} }}]
[routes.r]
type = "passthrough"
targets = ["t"]
"#,
        dir.join("ledger.db").display()
    );
    let config = Config::from_toml(&toml, |n| std::env::var(n).ok()).unwrap();
    format!(
        "http://{}",
        serve(server::router(config).await.unwrap()).await
    )
}

/// Budgets of `docs/performance.md`: the gateway's added latency over a direct call with a 50 ms upstream at up to
/// 64 concurrent requests.
const BUDGET_ADDED_P50_MS: f64 = 2.0;
const BUDGET_ADDED_P99_MS: f64 = 10.0;
/// Streaming (20 chunks 5 ms apart, up to 64 concurrent streams): what the gateway adds to the time to the first
/// chunk, and to the whole stream. The whole-stream figures are noisy (Run 7 of `docs/performance.md`), so their
/// limits are looser.
const BUDGET_STREAM_FIRST_P50_MS: f64 = 2.0;
const BUDGET_STREAM_FIRST_P99_MS: f64 = 10.0;
const BUDGET_STREAM_TOTAL_P50_MS: f64 = 15.0;
const BUDGET_STREAM_TOTAL_P99_MS: f64 = 30.0;

/// What one cell of the 50 ms table added over the direct call.
struct Cell {
    concurrency: usize,
    added_p50: f64,
    added_p99: f64,
}

#[derive(Debug, PartialEq)]
enum Verdict {
    Met,
    Exceeded,
    NotChecked(&'static str),
}

struct BudgetLine {
    name: &'static str,
    limit: String,
    measured: String,
    verdict: Verdict,
}

/// A limit from the environment (to check the failing path), else the recorded one.
fn limit_from_env(name: &str, recorded: f64) -> f64 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(recorded)
}

/// Whether an exceeded budget fails the run: only with `HUMPYARD_BUDGET_ENFORCE=1`.
fn enforcing(value: Option<&str>) -> bool {
    value == Some("1")
}

/// One latency budget against the worst cell found (its value and concurrency), if any cell was run.
fn latency_line(
    name: &'static str,
    limit: f64,
    found: Option<(f64, usize)>,
    missing: &'static str,
) -> BudgetLine {
    match found {
        Some((ms, concurrency)) => BudgetLine {
            name,
            limit: format!("{limit:.2} ms"),
            measured: format!("{ms:.2} ms (worst, at concurrency {concurrency})"),
            verdict: if ms <= limit {
                Verdict::Met
            } else {
                Verdict::Exceeded
            },
        },
        None => BudgetLine {
            name,
            limit: format!("{limit:.2} ms"),
            measured: "no measurement".into(),
            verdict: Verdict::NotChecked(missing),
        },
    }
}

/// The largest of `value` over the cells, with the concurrency it came at.
fn worst<T>(
    cells: &[T],
    value: fn(&T) -> f64,
    concurrency: fn(&T) -> usize,
) -> Option<(f64, usize)> {
    cells
        .iter()
        .max_by(|a, b| value(a).total_cmp(&value(b)))
        .map(|c| (value(c), concurrency(c)))
}

/// Compares the measurements with the budgets. The worst cell decides the latency budgets.
fn evaluate(cells: &[Cell], (p50_limit, p99_limit): (f64, f64)) -> Vec<BudgetLine> {
    let missing = "the 50 ms cells were not run";
    vec![
        latency_line(
            "gateway-added p50 latency, 50 ms upstream",
            p50_limit,
            worst(cells, |c| c.added_p50, |c| c.concurrency),
            missing,
        ),
        latency_line(
            "gateway-added p99 latency, 50 ms upstream",
            p99_limit,
            worst(cells, |c| c.added_p99, |c| c.concurrency),
            missing,
        ),
    ]
}

/// What one cell of the streaming table added over the direct stream, in milliseconds.
struct StreamCell {
    concurrency: usize,
    first_p50: f64,
    first_p99: f64,
    total_p50: f64,
    total_p99: f64,
}

/// The four streaming budgets against the worst cell of each.
fn evaluate_stream(cells: &[StreamCell], limits: [f64; 4]) -> Vec<BudgetLine> {
    let missing = "the streaming cells were not run";
    vec![
        latency_line(
            "gateway-added time to the first chunk, p50",
            limits[0],
            worst(cells, |c| c.first_p50, |c| c.concurrency),
            missing,
        ),
        latency_line(
            "gateway-added time to the first chunk, p99",
            limits[1],
            worst(cells, |c| c.first_p99, |c| c.concurrency),
            missing,
        ),
        latency_line(
            "gateway-added time to the whole stream, p50",
            limits[2],
            worst(cells, |c| c.total_p50, |c| c.concurrency),
            missing,
        ),
        latency_line(
            "gateway-added time to the whole stream, p99",
            limits[3],
            worst(cells, |c| c.total_p99, |c| c.concurrency),
            missing,
        ),
    ]
}

/// Prints the budget lines and returns one message per budget that fails the run: an exceeded one always (the
/// caller fails the run only when enforcing), and, when enforcing, one that could not be checked, because an
/// enforced run that skipped a budget would pass without having enforced it.
fn report(lines: &[BudgetLine], enforced: bool) -> Vec<String> {
    println!("\n=== budgets (docs/performance.md) ===");
    let mut exceeded = Vec::new();
    for line in lines {
        let verdict = match &line.verdict {
            Verdict::Met => "met".to_string(),
            Verdict::Exceeded => "EXCEEDED".to_string(),
            Verdict::NotChecked(why) => format!("not checked: {why}"),
        };
        println!(
            "{:<46} limit {:<32} measured {:<44} {verdict}",
            line.name, line.limit, line.measured
        );
        match &line.verdict {
            Verdict::Exceeded => exceeded.push(format!(
                "budget exceeded: {}: measured {}, limit {}",
                line.name, line.measured, line.limit
            )),
            Verdict::NotChecked(why) if enforced => {
                exceeded.push(format!("budget not checked: {}: {why}", line.name));
            }
            _ => {}
        }
    }
    if !exceeded.is_empty() && !enforced {
        println!(
            "(not enforced: set HUMPYARD_BUDGET_ENFORCE=1 to fail the run on an exceeded budget)"
        );
    }
    exceeded
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "overhead harness; run explicitly with --ignored"]
async fn gateway_overhead() {
    let dir = std::env::temp_dir().join(format!("humpyard-overhead-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let client = reqwest::Client::builder()
        .pool_max_idle_per_host(256)
        .build()
        .unwrap();
    let base_requests: usize = std::env::var("OVERHEAD_REQUESTS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(5_000);

    println!("\n=== gateway overhead: direct to a mock upstream versus through the gateway ===");
    let mut cells = Vec::new();
    println!(
        "{:<12}{:>5}  {:<8}{:>9}{:>9}{:>9}{:>11}   overhead p50 / p99 (ms)",
        "upstream", "conc", "path", "p50 ms", "p90 ms", "p99 ms", "req/s"
    );
    for delay_ms in [0u64, 50] {
        let delay = Duration::from_millis(delay_ms);
        let mock = mock_upstream(delay).await;
        std::fs::create_dir_all(dir.join(format!("d{delay_ms}"))).unwrap();
        let gw = gateway(&mock, &dir.join(format!("d{delay_ms}"))).await;
        let (direct_url, gateway_url) = (
            format!("{mock}/chat/completions"),
            format!("{gw}/v1/chat/completions"),
        );
        // Warm both paths (connections, caches, the ledger writer).
        run(&client, &direct_url, 8, 300).await;
        run(&client, &gateway_url, 8, 300).await;
        for concurrency in [1usize, 16, 64] {
            // A delayed upstream makes each request slow, so fewer are needed for the same precision.
            let requests = if delay_ms == 0 {
                base_requests
            } else {
                100 * concurrency
            };
            let direct = run(&client, &direct_url, concurrency, requests).await;
            let through = run(&client, &gateway_url, concurrency, requests).await;
            if delay_ms == 50 {
                cells.push(Cell {
                    concurrency,
                    added_p50: through.p50 - direct.p50,
                    added_p99: through.p99 - direct.p99,
                });
            }
            let label = if delay_ms == 0 {
                "instant".to_string()
            } else {
                format!("{delay_ms} ms")
            };
            for (path, r) in [("direct", &direct), ("gateway", &through)] {
                let note = if path == "gateway" {
                    format!(
                        "   +{:.2} / +{:.2}",
                        through.p50 - direct.p50,
                        through.p99 - direct.p99
                    )
                } else {
                    String::new()
                };
                println!(
                    "{label:<12}{concurrency:>5}  {path:<8}{:>9.2}{:>9.2}{:>9.2}{:>11.0}{note}",
                    r.p50, r.p90, r.p99, r.rps
                );
            }
        }
    }
    let _ = std::fs::remove_dir_all(&dir);

    let limits = (
        limit_from_env("OVERHEAD_BUDGET_P50_MS", BUDGET_ADDED_P50_MS),
        limit_from_env("OVERHEAD_BUDGET_P99_MS", BUDGET_ADDED_P99_MS),
    );
    let enforced = enforcing(std::env::var("HUMPYARD_BUDGET_ENFORCE").ok().as_deref());
    let exceeded = report(&evaluate(&cells, limits), enforced);
    if enforced {
        assert!(exceeded.is_empty(), "{}", exceeded.join("\n"));
    }
}

/// Streaming overhead: what the gateway adds to the time to the first chunk (what a client sees as the first token)
/// and to the whole stream, against the same mock streaming 20 chunks 5 ms apart.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "overhead harness; run explicitly with --ignored"]
async fn gateway_streaming_overhead() {
    let dir = std::env::temp_dir().join(format!("humpyard-overhead-stream-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let client = reqwest::Client::builder()
        .pool_max_idle_per_host(256)
        .build()
        .unwrap();
    let per_worker: usize = std::env::var("OVERHEAD_STREAMS_PER_WORKER")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(50);
    let mock = mock_upstream(Duration::ZERO).await;
    let gw = gateway(&mock, &dir).await;
    let (direct_url, gateway_url) = (
        format!("{mock}/chat/completions"),
        format!("{gw}/v1/chat/completions"),
    );
    run_stream(&client, &direct_url, 8, 80).await;
    run_stream(&client, &gateway_url, 8, 80).await;
    let mut cells = Vec::new();

    println!(
        "\n=== streaming overhead: {STREAM_CHUNKS} chunks, {} ms apart, direct versus through the gateway ===",
        STREAM_GAP.as_millis()
    );
    println!(
        "{:>5}  {:<8}{:>11}{:>11}{:>12}{:>12}{:>11}   added: first chunk p50 / p99, whole stream p50 / p99 (ms)",
        "conc", "path", "first p50", "first p99", "total p50", "total p99", "streams/s"
    );
    for concurrency in [1usize, 16, 64] {
        let requests = per_worker * concurrency;
        let direct = run_stream(&client, &direct_url, concurrency, requests).await;
        let through = run_stream(&client, &gateway_url, concurrency, requests).await;
        cells.push(StreamCell {
            concurrency,
            first_p50: through.ttfb_p50 - direct.ttfb_p50,
            first_p99: through.ttfb_p99 - direct.ttfb_p99,
            total_p50: through.total_p50 - direct.total_p50,
            total_p99: through.total_p99 - direct.total_p99,
        });
        for (path, r) in [("direct", &direct), ("gateway", &through)] {
            let note = if path == "gateway" {
                format!(
                    "   +{:.2} / +{:.2}, +{:.2} / +{:.2}",
                    through.ttfb_p50 - direct.ttfb_p50,
                    through.ttfb_p99 - direct.ttfb_p99,
                    through.total_p50 - direct.total_p50,
                    through.total_p99 - direct.total_p99
                )
            } else {
                String::new()
            };
            println!(
                "{concurrency:>5}  {path:<8}{:>11.2}{:>11.2}{:>12.2}{:>12.2}{:>11.0}{note}",
                r.ttfb_p50, r.ttfb_p99, r.total_p50, r.total_p99, r.rps
            );
        }
    }
    let _ = std::fs::remove_dir_all(&dir);

    let enforced = enforcing(std::env::var("HUMPYARD_BUDGET_ENFORCE").ok().as_deref());
    let limits = [
        BUDGET_STREAM_FIRST_P50_MS,
        BUDGET_STREAM_FIRST_P99_MS,
        BUDGET_STREAM_TOTAL_P50_MS,
        BUDGET_STREAM_TOTAL_P99_MS,
    ];
    let exceeded = report(&evaluate_stream(&cells, limits), enforced);
    if enforced {
        assert!(exceeded.is_empty(), "{}", exceeded.join("\n"));
    }
}

fn cell(concurrency: usize, added_p50: f64, added_p99: f64) -> Cell {
    Cell {
        concurrency,
        added_p50,
        added_p99,
    }
}

const RECORDED: (f64, f64) = (BUDGET_ADDED_P50_MS, BUDGET_ADDED_P99_MS);

#[test]
fn the_recorded_run_meets_every_latency_budget() {
    // Run 2 of docs/performance.md: the 50 ms cells at concurrency 1, 16 and 64.
    let cells = [
        cell(1, 0.57, 0.59),
        cell(16, 0.41, 0.64),
        cell(64, 1.22, 5.72),
    ];
    let lines = evaluate(&cells, RECORDED);
    assert!(lines.iter().all(|l| l.verdict == Verdict::Met));
}

#[test]
fn an_exceeded_budget_names_the_budget_the_value_and_the_worst_cell() {
    let cells = [cell(1, 0.5, 0.6), cell(64, 2.5, 4.0)];
    let lines = evaluate(&cells, RECORDED);
    assert_eq!(lines[0].verdict, Verdict::Exceeded);
    assert_eq!(lines[1].verdict, Verdict::Met);
    let messages = report(&lines, true);
    assert_eq!(messages.len(), 1);
    assert!(
        messages[0].contains("gateway-added p50 latency"),
        "{}",
        messages[0]
    );
    assert!(messages[0].contains("2.50 ms"), "{}", messages[0]);
    assert!(messages[0].contains("concurrency 64"), "{}", messages[0]);
    assert!(messages[0].contains("2.00 ms"), "{}", messages[0]);
}

#[test]
fn a_tightened_limit_is_exceeded_by_the_recorded_run() {
    let cells = [cell(64, 1.22, 5.72)];
    let lines = evaluate(&cells, (1.0, 5.0));
    assert_eq!(lines[0].verdict, Verdict::Exceeded);
    assert_eq!(lines[1].verdict, Verdict::Exceeded);
}

#[test]
fn an_unchecked_budget_fails_an_enforced_run_and_only_an_enforced_run() {
    // No 50 ms cells were run, so neither latency budget could be checked.
    let lines = evaluate(&[], RECORDED);
    assert!(
        lines
            .iter()
            .all(|l| matches!(l.verdict, Verdict::NotChecked(_)))
    );
    assert_eq!(report(&lines, false), Vec::<String>::new());
    let messages = report(&lines, true);
    assert_eq!(messages.len(), 2);
    assert!(
        messages[0].contains("budget not checked"),
        "{}",
        messages[0]
    );
    assert!(
        messages[0].contains("the 50 ms cells were not run"),
        "{}",
        messages[0]
    );
}

#[test]
fn only_the_value_one_enforces() {
    assert!(enforcing(Some("1")));
    for other in [None, Some(""), Some("0"), Some("true"), Some("yes")] {
        assert!(!enforcing(other), "{other:?}");
    }
}

fn stream_cell(concurrency: usize, first: (f64, f64), total: (f64, f64)) -> StreamCell {
    StreamCell {
        concurrency,
        first_p50: first.0,
        first_p99: first.1,
        total_p50: total.0,
        total_p99: total.1,
    }
}

const STREAM_RECORDED: [f64; 4] = [
    BUDGET_STREAM_FIRST_P50_MS,
    BUDGET_STREAM_FIRST_P99_MS,
    BUDGET_STREAM_TOTAL_P50_MS,
    BUDGET_STREAM_TOTAL_P99_MS,
];

#[test]
fn the_recorded_streaming_runs_meet_every_streaming_budget() {
    // The worst cells of the three runs of Run 7 in docs/performance.md.
    let cells = [
        stream_cell(1, (0.61, 0.75), (1.53, 4.98)),
        stream_cell(16, (0.86, 1.14), (1.92, 15.46)),
        stream_cell(64, (1.00, 3.26), (9.82, 12.27)),
    ];
    let lines = evaluate_stream(&cells, STREAM_RECORDED);
    assert_eq!(lines.len(), 4);
    assert!(
        lines.iter().all(|l| l.verdict == Verdict::Met),
        "{:?}",
        lines.iter().map(|l| &l.verdict).collect::<Vec<_>>()
    );
}

#[test]
fn an_exceeded_streaming_budget_names_the_budget_the_value_and_the_worst_cell() {
    let cells = [
        stream_cell(1, (0.5, 0.7), (1.0, 2.0)),
        stream_cell(64, (2.4, 3.0), (5.0, 6.0)),
    ];
    let lines = evaluate_stream(&cells, STREAM_RECORDED);
    assert_eq!(lines[0].verdict, Verdict::Exceeded);
    assert!(lines[1..].iter().all(|l| l.verdict == Verdict::Met));
    let messages = report(&lines, true);
    assert_eq!(messages.len(), 1);
    assert!(
        messages[0].contains("time to the first chunk, p50"),
        "{}",
        messages[0]
    );
    assert!(messages[0].contains("2.40 ms"), "{}", messages[0]);
    assert!(messages[0].contains("concurrency 64"), "{}", messages[0]);
}

#[test]
fn streaming_budgets_that_were_not_checked_fail_an_enforced_run() {
    let lines = evaluate_stream(&[], STREAM_RECORDED);
    assert_eq!(report(&lines, false), Vec::<String>::new());
    assert_eq!(report(&lines, true).len(), 4);
}
