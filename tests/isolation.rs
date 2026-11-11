//! Isolation test (spec: `performance`, ADR 0036). Not part of the default run:
//!
//! ```text
//! cargo test --release --test isolation -- --ignored --nocapture
//! ```
//!
//! Administrative statistics reads must not move the request path. The gateway runs on a runtime with ONE worker
//! thread, the worst case (a 500m container can leave a runtime with one), in a thread of its own, so only the
//! gateway is constrained. Its ledger is seeded past the statistics row cap. The same request traffic is then
//! measured with and without two clients reading statistics and session views back to back, in alternating rounds
//! so that drift in the machine reaches both halves, and the verdict is on the median p99 of the rounds.
//!
//! `ISOLATION_WORKERS` (default 1) changes the gateway's worker count, to see how much of a result depends on it;
//! the recorded verdict is always the default.
//!
//! It fails when the loaded p99 is more than `max(10%, 1 ms)` above the unloaded one, when the ledger dropped an
//! entry, or when the load made no successful read (so a gate that refuses everything cannot pass). Before any of
//! that, an unloaded-against-unloaded check shows the machine is quiet enough to decide: if two unloaded halves
//! differ by more than the threshold the test says it is too noisy, after three attempts, instead of passing.
//!
//! The load asks for a different window each time, so a cache of identical reads cannot make it cheap.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test scaffolding
#![allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)] // reporting only

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use axum::Router;
use axum::response::IntoResponse;
use axum::routing::post;
use common::load::{Run, run};
use common::seed::{Rng, make_row};
use common::{chat_completion, serve, usage};
use humpyard::config::Config;
use humpyard::ledger::Ledger;
use humpyard::server;

const DAY_MS: i64 = 86_400_000;
/// Request traffic: 16 concurrent requests against a 5 ms upstream, so a stalled worker shows as a large p99.
const CONCURRENCY: usize = 16;
const UPSTREAM_DELAY: Duration = Duration::from_millis(5);
/// The share, and the floor in milliseconds, by which the loaded p99 may exceed the unloaded one.
const ALLOWED_RISE_FRACTION: f64 = 0.10;
const ALLOWED_RISE_FLOOR_MS: f64 = 1.0;
const NOISE_ATTEMPTS: usize = 3;

fn env_usize(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

/// How far above `base` a p99 may rise before the request path counts as moved.
fn allowed_rise(base: f64) -> f64 {
    (base * ALLOWED_RISE_FRACTION).max(ALLOWED_RISE_FLOOR_MS)
}

fn moved(base: f64, other: f64) -> bool {
    other > base + allowed_rise(base)
}

fn median(values: &[f64]) -> f64 {
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let mid = sorted.len() / 2;
    if sorted.len() % 2 == 1 {
        sorted[mid]
    } else {
        f64::midpoint(sorted[mid - 1], sorted[mid])
    }
}

async fn completions() -> axum::response::Response {
    tokio::time::sleep(UPSTREAM_DELAY).await;
    axum::Json(chat_completion("m", "ok", Some(usage(20, 5)))).into_response()
}

async fn mock_upstream() -> String {
    let app = Router::new().route("/chat/completions", post(completions));
    format!("http://{}", serve(app).await)
}

/// Fills a ledger past the statistics row cap through its real write path: `rows` usage rows and the decisions
/// that go with two of every three, spread over the last 29 days.
async fn seed_ledger(path: &std::path::Path, rows: u64) {
    let now_ms = i64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis(),
    )
    .unwrap();
    let days = 29;
    let start_ms = now_ms - days * DAY_MS;
    let ledger = Ledger::open_with(path, 100_000, 0).await.unwrap();
    let mut rng = Rng(1);
    for i in 0..rows {
        let ts =
            start_ms + (i64::try_from(i).unwrap() * days * DAY_MS) / i64::try_from(rows).unwrap();
        let (entry, decision) = make_row(&mut rng, i, ts);
        ledger.record(entry);
        if i % 3 != 2 {
            ledger.record_decision(decision);
        }
        if ledger.queue_depth() > 60_000 {
            ledger.flush().await;
        }
    }
    ledger.flush().await;
    assert_eq!(ledger.dropped(), 0, "seeding must not drop rows");
    ledger.shutdown().await;
}

/// Starts the gateway on a runtime with `workers` worker threads, in a thread of its own, and returns its address.
fn start_gateway(toml: String, workers: usize) -> String {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(workers)
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async move {
            let config = Config::from_toml(&toml, |n| std::env::var(n).ok()).unwrap();
            let app = server::router(config).await.unwrap();
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            tx.send(listener.local_addr().unwrap()).unwrap();
            axum::serve(listener, app).await.unwrap();
        });
    });
    format!("http://{}", rx.recv().unwrap())
}

/// Two clients reading statistics and session views back to back until stopped, each read for a different window.
struct Load {
    stop: Arc<AtomicBool>,
    ok: Arc<AtomicU64>,
    refused: Arc<AtomicU64>,
    failed: Arc<AtomicU64>,
    tasks: Vec<tokio::task::JoinHandle<()>>,
}

struct LoadCounts {
    ok: u64,
    refused: u64,
    failed: u64,
}

fn target_query(n: u64) -> String {
    format!("/admin/v1/stats?group_by=target&window={}h", 700 + n % 24)
}

fn session_query(n: u64) -> String {
    format!("/admin/v1/stats?group_by=session&window={}h", 700 + n % 24)
}

fn start_load(client: &reqwest::Client, base: &str, key: &str) -> Load {
    let load = Load {
        stop: Arc::new(AtomicBool::new(false)),
        ok: Arc::new(AtomicU64::new(0)),
        refused: Arc::new(AtomicU64::new(0)),
        failed: Arc::new(AtomicU64::new(0)),
        tasks: Vec::new(),
    };
    let mut load = load;
    for query in [target_query as fn(u64) -> String, session_query] {
        let (client, base, key) = (client.clone(), base.to_string(), key.to_string());
        let (stop, ok, refused, failed) = (
            load.stop.clone(),
            load.ok.clone(),
            load.refused.clone(),
            load.failed.clone(),
        );
        load.tasks.push(tokio::spawn(async move {
            let mut n = 0;
            while !stop.load(Ordering::Relaxed) {
                let response = client
                    .get(format!("{base}{}", query(n)))
                    .bearer_auth(&key)
                    .send()
                    .await;
                n += 1;
                match response {
                    Ok(r) => {
                        let status = r.status().as_u16();
                        let _ = r.bytes().await;
                        match status {
                            200 => ok.fetch_add(1, Ordering::Relaxed),
                            503 => refused.fetch_add(1, Ordering::Relaxed),
                            _ => failed.fetch_add(1, Ordering::Relaxed),
                        };
                    }
                    Err(_) => {
                        failed.fetch_add(1, Ordering::Relaxed);
                    }
                }
            }
        }));
    }
    load
}

impl Load {
    async fn finish(self) -> LoadCounts {
        self.stop.store(true, Ordering::Relaxed);
        for task in self.tasks {
            task.await.unwrap();
        }
        LoadCounts {
            ok: self.ok.load(Ordering::Relaxed),
            refused: self.refused.load(Ordering::Relaxed),
            failed: self.failed.load(Ordering::Relaxed),
        }
    }
}

async fn unloaded(client: &reqwest::Client, url: &str, requests: usize) -> Run {
    run(client, url, CONCURRENCY, requests).await
}

async fn loaded(
    client: &reqwest::Client,
    url: &str,
    requests: usize,
    base: &str,
    key: &str,
) -> (Run, LoadCounts) {
    let load = start_load(client, base, key);
    // Let the reads get under way before measuring.
    tokio::time::sleep(Duration::from_millis(300)).await;
    let measured = run(client, url, CONCURRENCY, requests).await;
    let counts = load.finish().await;
    // And let the gateway finish them before the next unloaded half.
    tokio::time::sleep(Duration::from_millis(300)).await;
    (measured, counts)
}

/// The ledger's dropped-entry count, from the metrics endpoint.
async fn dropped(client: &reqwest::Client, base: &str, metrics_key: &str) -> u64 {
    let text = client
        .get(format!("{base}/metrics"))
        .bearer_auth(metrics_key)
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    text.lines()
        .find(|l| l.starts_with("humpyard_ledger_dropped_total"))
        .and_then(|l| l.split_whitespace().last())
        .and_then(|v| v.parse().ok())
        .unwrap_or_else(|| panic!("no humpyard_ledger_dropped_total in the metrics:\n{text}"))
}

fn gateway_toml(
    ledger: &std::path::Path,
    upstream: &str,
    admin_hash: &str,
    metrics_hash: &str,
) -> String {
    format!(
        r#"listen = "127.0.0.1:0"
[ledger]
path = "{}"
[metrics]
enabled = true
bearer_sha256 = "{metrics_hash}"
[providers.p]
base_url = "{upstream}"
api_key_env = "PATH"
max_retries = 0
[targets.t]
endpoints = [{{ provider = "p", model = "m", price = {{ input = 0.0, output = 0.0 }} }}]
[routes.r]
type = "passthrough"
targets = ["t"]
[admin.credentials.gary]
sha256 = "{admin_hash}"
scopes = ["read"]
"#,
        ledger.display()
    )
}

/// Is the machine quiet enough to decide? Two unloaded halves must agree within the threshold, in one of
/// `NOISE_ATTEMPTS` tries.
async fn assert_quiet(client: &reqwest::Client, url: &str, rounds: usize, requests: usize) {
    for attempt in 1..=NOISE_ATTEMPTS {
        let (mut first, mut second) = (Vec::new(), Vec::new());
        for _ in 0..rounds {
            first.push(unloaded(client, url, requests).await.p99);
            second.push(unloaded(client, url, requests).await.p99);
        }
        let (a, b) = (median(&first), median(&second));
        println!("noise check {attempt}: unloaded p99 {a:.2} ms against {b:.2} ms");
        if !moved(a, b) && !moved(b, a) {
            return;
        }
    }
    panic!(
        "the environment is too noisy to decide: two unloaded halves differ by more than the threshold"
    );
}

/// What the run found, as the reasons it fails (none when it passes).
struct Findings {
    unloaded_p99: f64,
    loaded_p99: f64,
    dropped: u64,
    every_round_read: bool,
    failed_reads: u64,
}

fn failures(f: &Findings) -> Vec<String> {
    let mut out = Vec::new();
    if moved(f.unloaded_p99, f.loaded_p99) {
        out.push(format!(
            "request p99 rose from {:.2} ms to {:.2} ms under admin reads (allowed rise {:.2} ms)",
            f.unloaded_p99,
            f.loaded_p99,
            allowed_rise(f.unloaded_p99)
        ));
    }
    if f.dropped > 0 {
        out.push(format!(
            "{} ledger entries were dropped during the run",
            f.dropped
        ));
    }
    if !f.every_round_read {
        out.push(
            "a loaded round made no successful admin read, so it did not load the gateway".into(),
        );
    }
    if f.failed_reads > 0 {
        out.push(format!(
            "{} admin reads failed (not 200 or 503); the load is misconfigured or the gateway broke",
            f.failed_reads
        ));
    }
    out
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "isolation test; run explicitly with --ignored (CI runs it in release mode)"]
async fn admin_reads_do_not_move_the_request_path() {
    let rows = env_usize("ISOLATION_ROWS", 100_000) as u64;
    let requests = env_usize("ISOLATION_REQUESTS", 4_000);
    let rounds = env_usize("ISOLATION_ROUNDS", 5);

    let dir = common::scratch().join("isolation");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let ledger_path = dir.join("ledger.db");
    let started = std::time::Instant::now();
    seed_ledger(&ledger_path, rows).await;
    println!(
        "\nseeded {rows} usage rows and about {} decisions in {:.1} s",
        rows * 2 / 3,
        started.elapsed().as_secs_f64()
    );

    let (admin_key, admin_hash) = humpyard::auth::generate_key().unwrap();
    let (metrics_key, metrics_hash) = humpyard::auth::generate_key().unwrap();
    let upstream = mock_upstream().await;
    let workers = env_usize("ISOLATION_WORKERS", 1);
    println!("gateway runtime: {workers} worker thread(s)");
    let base = start_gateway(
        gateway_toml(&ledger_path, &upstream, &admin_hash, &metrics_hash),
        workers,
    );
    let url = format!("{base}/v1/chat/completions");
    let client = reqwest::Client::builder()
        .pool_max_idle_per_host(256)
        .build()
        .unwrap();
    unloaded(&client, &url, 600).await; // warm the connections, the ledger writer and the caches

    assert_quiet(&client, &url, rounds, requests).await;

    println!(
        "\n{:<7}{:>16}{:>16}{:>12}{:>10}{:>8}",
        "round", "unloaded p99 ms", "loaded p99 ms", "reads ok", "refused", "failed"
    );
    let (mut a99, mut b99) = (Vec::new(), Vec::new());
    let (mut ok_each_round, mut failed_total) = (true, 0);
    for round in 1..=rounds {
        let a = unloaded(&client, &url, requests).await;
        let (b, counts) = loaded(&client, &url, requests, &base, &admin_key).await;
        println!(
            "{round:<7}{:>16.2}{:>16.2}{:>12}{:>10}{:>8}",
            a.p99, b.p99, counts.ok, counts.refused, counts.failed
        );
        ok_each_round &= counts.ok > 0;
        failed_total += counts.failed;
        a99.push(a.p99);
        b99.push(b.p99);
    }
    let (base_p99, loaded_p99) = (median(&a99), median(&b99));
    let dropped = dropped(&client, &base, &metrics_key).await;
    println!(
        "\nmedian p99: {base_p99:.2} ms unloaded, {loaded_p99:.2} ms loaded (allowed rise {:.2} ms); dropped entries {dropped}",
        allowed_rise(base_p99)
    );
    let _ = std::fs::remove_dir_all(&dir);

    let failures = failures(&Findings {
        unloaded_p99: base_p99,
        loaded_p99,
        dropped,
        every_round_read: ok_each_round,
        failed_reads: failed_total,
    });
    assert!(
        failures.is_empty(),
        "isolation failed:\n  {}",
        failures.join("\n  ")
    );
}

#[test]
fn the_allowed_rise_is_a_tenth_with_a_floor_of_a_millisecond() {
    assert!(
        (allowed_rise(2.0) - 1.0).abs() < 1e-9,
        "the floor decides at small p99"
    );
    assert!(
        (allowed_rise(50.0) - 5.0).abs() < 1e-9,
        "a tenth decides at large p99"
    );
}

#[test]
fn a_rise_is_judged_against_the_threshold_exactly() {
    assert!(!moved(10.0, 10.0));
    assert!(
        !moved(10.0, 11.0),
        "a rise of exactly the allowance is allowed"
    );
    assert!(moved(10.0, 11.01));
    assert!(!moved(10.0, 5.0), "a fall is never a move");
    assert!(moved(100.0, 110.5));
}

#[test]
fn the_median_of_an_odd_and_an_even_count() {
    assert!((median(&[3.0, 1.0, 2.0]) - 2.0).abs() < 1e-9);
    assert!((median(&[4.0, 1.0, 3.0, 2.0]) - 2.5).abs() < 1e-9);
    assert!((median(&[7.0]) - 7.0).abs() < 1e-9);
}

#[test]
fn the_load_asks_for_a_different_window_each_time() {
    let seen: std::collections::HashSet<String> = (0..24).map(target_query).collect();
    assert_eq!(seen.len(), 24);
    assert!(target_query(0).contains("group_by=target"));
    assert!(session_query(0).contains("group_by=session"));
}

fn passing() -> Findings {
    Findings {
        unloaded_p99: 8.0,
        loaded_p99: 8.5,
        dropped: 0,
        every_round_read: true,
        failed_reads: 0,
    }
}

#[test]
fn a_quiet_run_with_no_loss_and_real_reads_passes() {
    assert_eq!(failures(&passing()), Vec::<String>::new());
}

#[test]
fn each_way_to_fail_is_reported_on_its_own() {
    let slow = failures(&Findings {
        loaded_p99: 40.0,
        ..passing()
    });
    assert_eq!(slow.len(), 1);
    assert!(slow[0].contains("request p99 rose"), "{slow:?}");
    let lost = failures(&Findings {
        dropped: 3,
        ..passing()
    });
    assert!(lost.len() == 1 && lost[0].contains("dropped"), "{lost:?}");
    let idle = failures(&Findings {
        every_round_read: false,
        ..passing()
    });
    assert!(
        idle.len() == 1 && idle[0].contains("no successful admin read"),
        "{idle:?}"
    );
    let broken = failures(&Findings {
        failed_reads: 2,
        ..passing()
    });
    assert!(
        broken.len() == 1 && broken[0].contains("failed"),
        "{broken:?}"
    );
}
