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
//! runners are too noisy for absolute numbers, so CI relies on the relative isolation test instead.
//!
//! - `OVERHEAD_BASELINE_RPS`: the throughput recorded on this machine at the highest concurrency against the
//!   instant upstream; without it the throughput budget is reported as not checked.
//! - `OVERHEAD_BUDGET_P50_MS`, `OVERHEAD_BUDGET_P99_MS`: replace a recorded limit, to check the failing path with a
//!   tightened one. The recorded values are the constants below, and they change only with the documentation.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test scaffolding
#![allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)] // reporting only

mod common;

use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::Router;
use axum::extract::State;
use axum::response::IntoResponse;
use axum::routing::post;
use common::{chat_completion, chat_request, serve, usage};
use humpyard::config::Config;
use humpyard::server;

struct Mock {
    delay: Duration,
}

async fn completions(State(mock): State<Arc<Mock>>) -> axum::response::Response {
    if !mock.delay.is_zero() {
        tokio::time::sleep(mock.delay).await;
    }
    axum::Json(chat_completion("m", "ok", Some(usage(20, 5)))).into_response()
}

async fn mock_upstream(delay: Duration) -> String {
    let app = Router::new()
        .route("/chat/completions", post(completions))
        .with_state(Arc::new(Mock { delay }));
    format!("http://{}", serve(app).await)
}

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
/// 64 concurrent requests, and throughput no more than 10% below the baseline recorded on the same machine.
const BUDGET_ADDED_P50_MS: f64 = 2.0;
const BUDGET_ADDED_P99_MS: f64 = 10.0;
const BUDGET_THROUGHPUT_FLOOR: f64 = 0.90;
/// The concurrency at which throughput is compared with the baseline.
const THROUGHPUT_CONCURRENCY: usize = 64;

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

/// Compares the measurements with the budgets. The worst cell decides the latency budgets.
fn evaluate(
    cells: &[Cell],
    peak_rps: Option<f64>,
    baseline_rps: Option<f64>,
    (p50_limit, p99_limit): (f64, f64),
) -> Vec<BudgetLine> {
    let worst = |value: fn(&Cell) -> f64| {
        cells
            .iter()
            .max_by(|a, b| value(a).total_cmp(&value(b)))
            .map(|c| (value(c), c.concurrency))
    };
    let latency = |name, limit: f64, found: Option<(f64, usize)>| match found {
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
            verdict: Verdict::NotChecked("the 50 ms cells were not run"),
        },
    };
    let throughput = match (peak_rps, baseline_rps) {
        (Some(rps), Some(baseline)) => {
            let floor = baseline * BUDGET_THROUGHPUT_FLOOR;
            BudgetLine {
                name: "throughput against the recorded baseline",
                limit: format!(">= {floor:.0} req/s (baseline {baseline:.0})"),
                measured: format!("{rps:.0} req/s"),
                verdict: if rps >= floor {
                    Verdict::Met
                } else {
                    Verdict::Exceeded
                },
            }
        }
        (Some(rps), None) => BudgetLine {
            name: "throughput against the recorded baseline",
            limit: "90% of the baseline".into(),
            measured: format!("{rps:.0} req/s"),
            verdict: Verdict::NotChecked(
                "set OVERHEAD_BASELINE_RPS to this machine's recorded figure",
            ),
        },
        (None, _) => BudgetLine {
            name: "throughput against the recorded baseline",
            limit: "90% of the baseline".into(),
            measured: "no measurement".into(),
            verdict: Verdict::NotChecked("the instant-upstream cells were not run"),
        },
    };
    vec![
        latency(
            "gateway-added p50 latency, 50 ms upstream",
            p50_limit,
            worst(|c| c.added_p50),
        ),
        latency(
            "gateway-added p99 latency, 50 ms upstream",
            p99_limit,
            worst(|c| c.added_p99),
        ),
        throughput,
    ]
}

/// Prints the budget lines and returns one message per exceeded budget, naming the budget and the value.
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
        if line.verdict == Verdict::Exceeded {
            exceeded.push(format!(
                "budget exceeded: {}: measured {}, limit {}",
                line.name, line.measured, line.limit
            ));
        }
    }
    if !exceeded.is_empty() && !enforced {
        println!(
            "(not enforced: set HUMPYARD_BUDGET_ENFORCE=1 to fail the run on an exceeded budget)"
        );
    }
    exceeded
}

struct Run {
    p50: f64,
    p90: f64,
    p99: f64,
    rps: f64,
}

/// The `percent`th percentile (nearest rank) of an ascending list.
fn percentile(sorted: &[f64], percent: usize) -> f64 {
    let index = ((sorted.len() - 1) * percent + 50) / 100;
    sorted[index.min(sorted.len() - 1)]
}

/// `requests` requests spread over `concurrency` workers; per-request latency in milliseconds.
async fn run(client: &reqwest::Client, url: &str, concurrency: usize, requests: usize) -> Run {
    let body = chat_request("r", false);
    let per_worker = requests.div_ceil(concurrency);
    let started = Instant::now();
    let workers: Vec<_> = (0..concurrency)
        .map(|_| {
            let (client, url, body) = (client.clone(), url.to_string(), body.clone());
            tokio::spawn(async move {
                let mut latencies = Vec::with_capacity(per_worker);
                for _ in 0..per_worker {
                    let t = Instant::now();
                    let resp = client.post(&url).json(&body).send().await.unwrap();
                    assert_eq!(resp.status(), 200);
                    let _ = resp.bytes().await.unwrap();
                    latencies.push(t.elapsed().as_secs_f64() * 1000.0);
                }
                latencies
            })
        })
        .collect();
    let mut all = Vec::new();
    for worker in workers {
        all.extend(worker.await.unwrap());
    }
    let elapsed = started.elapsed().as_secs_f64();
    all.sort_by(f64::total_cmp);
    Run {
        p50: percentile(&all, 50),
        p90: percentile(&all, 90),
        p99: percentile(&all, 99),
        rps: all.len() as f64 / elapsed,
    }
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
    let mut peak_rps = None;
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
            if delay_ms == 0 && concurrency == THROUGHPUT_CONCURRENCY {
                peak_rps = Some(through.rps);
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

    let baseline = std::env::var("OVERHEAD_BASELINE_RPS")
        .ok()
        .and_then(|v| v.parse().ok());
    let limits = (
        limit_from_env("OVERHEAD_BUDGET_P50_MS", BUDGET_ADDED_P50_MS),
        limit_from_env("OVERHEAD_BUDGET_P99_MS", BUDGET_ADDED_P99_MS),
    );
    let enforced = enforcing(std::env::var("HUMPYARD_BUDGET_ENFORCE").ok().as_deref());
    let exceeded = report(&evaluate(&cells, peak_rps, baseline, limits), enforced);
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
    let lines = evaluate(&cells, Some(48_437.0), Some(46_000.0), RECORDED);
    assert!(lines.iter().all(|l| l.verdict == Verdict::Met));
}

#[test]
fn an_exceeded_budget_names_the_budget_the_value_and_the_worst_cell() {
    let cells = [cell(1, 0.5, 0.6), cell(64, 2.5, 4.0)];
    let lines = evaluate(&cells, None, None, RECORDED);
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
    let lines = evaluate(&cells, None, None, (1.0, 5.0));
    assert_eq!(lines[0].verdict, Verdict::Exceeded);
    assert_eq!(lines[1].verdict, Verdict::Exceeded);
}

#[test]
fn throughput_is_judged_against_ninety_percent_of_the_baseline_or_not_at_all() {
    let cells = [cell(64, 0.1, 0.1)];
    let below = evaluate(&cells, Some(40_000.0), Some(46_000.0), RECORDED);
    assert_eq!(below[2].verdict, Verdict::Exceeded);
    let at_floor = evaluate(&cells, Some(41_400.0), Some(46_000.0), RECORDED);
    assert_eq!(at_floor[2].verdict, Verdict::Met);
    let unchecked = evaluate(&cells, Some(1.0), None, RECORDED);
    assert!(matches!(unchecked[2].verdict, Verdict::NotChecked(_)));
}

#[test]
fn only_the_value_one_enforces() {
    assert!(enforcing(Some("1")));
    for other in [None, Some(""), Some("0"), Some("true"), Some("yes")] {
        assert!(!enforcing(other), "{other:?}");
    }
}
