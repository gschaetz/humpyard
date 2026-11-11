//! Gateway memory harness (spec: `performance`). Not part of the default run:
//!
//! ```text
//! cargo test --release --test memory -- --ignored --nocapture
//! ```
//!
//! Runs the real `humpyard serve` binary as a child process (so its memory is the gateway's alone, not the test's)
//! against a mock upstream in this process, and samples the child's resident set size with `ps` while load runs:
//! idle after start, at 64 concurrent requests (plain, then streamed, then plain again), and after each phase once
//! it has settled. The second plain phase against the first shows whether memory keeps growing under the same load.
//!
//! `HUMPYARD_BUDGET_ENFORCE=1` fails the run on an exceeded budget (the release check on the reference machine).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test scaffolding
#![allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)] // reporting only

mod common;

use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use common::load::{run, run_stream};
use common::mock::mock_upstream;

/// Concurrency of every loaded phase: the highest of the overhead harness.
const CONCURRENCY: usize = 64;

/// Budgets of `docs/performance.md`, in MiB of resident memory: the peak of the cluster's request for the gateway
/// container (64 MiB; the measured peak was 37 to 40 MiB), and a few times the growth measured between two loads.
const BUDGET_PEAK_MIB: f64 = 64.0;
/// How far the resident memory after the second plain phase may exceed the one after the first.
const BUDGET_GROWTH_MIB: f64 = 8.0;

/// The child process, killed when it goes out of scope so a failed run leaves nothing behind.
struct Gateway {
    child: Child,
}

impl Drop for Gateway {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// The resident set size of `pid` in KiB, from `ps` (the same on Linux and macOS).
fn rss_kib(pid: u32) -> u64 {
    let out = Command::new("ps")
        .args(["-o", "rss=", "-p", &pid.to_string()])
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse()
        .unwrap_or(0)
}

fn mib(kib: u64) -> f64 {
    kib as f64 / 1024.0
}

/// Samples the resident memory of `pid` every 25 ms until stopped, keeping the peak.
struct Sampler {
    stop: Arc<AtomicBool>,
    peak: Arc<AtomicU64>,
    thread: std::thread::JoinHandle<()>,
}

impl Sampler {
    fn start(pid: u32) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let peak = Arc::new(AtomicU64::new(0));
        let (flag, top) = (stop.clone(), peak.clone());
        let thread = std::thread::spawn(move || {
            while !flag.load(Ordering::Relaxed) {
                top.fetch_max(rss_kib(pid), Ordering::Relaxed);
                std::thread::sleep(Duration::from_millis(25));
            }
        });
        Self { stop, peak, thread }
    }

    /// Stops sampling and returns the peak in KiB.
    fn finish(self) -> u64 {
        self.stop.store(true, Ordering::Relaxed);
        self.thread.join().unwrap();
        self.peak.load(Ordering::Relaxed)
    }
}

async fn start_gateway(dir: &std::path::Path, upstream: &str) -> (Gateway, String) {
    // A free port: bind to port 0, note it, and let go of it for the child.
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let config = dir.join("config.toml");
    std::fs::write(
        &config,
        format!(
            r#"listen = "127.0.0.1:{port}"
[ledger]
path = "{}"
[providers.p]
base_url = "{upstream}"
api_key_env = "MEMORY_TEST_KEY"
max_retries = 0
[targets.t]
endpoints = [{{ provider = "p", model = "m", price = {{ input = 0.0, output = 0.0 }} }}]
[routes.r]
type = "passthrough"
targets = ["t"]
"#,
            dir.join("ledger.db").display()
        ),
    )
    .unwrap();
    let child = Command::new(env!("CARGO_BIN_EXE_humpyard"))
        .args(["serve", "--config", config.to_str().unwrap()])
        .env("MEMORY_TEST_KEY", "k")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let base = format!("http://127.0.0.1:{port}");
    let client = reqwest::Client::new();
    let started = Instant::now();
    while client.get(format!("{base}/healthz")).send().await.is_err() {
        assert!(
            started.elapsed() < Duration::from_secs(15),
            "the gateway did not start"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    (Gateway { child }, base)
}

/// One phase: the load runs while the child's memory is sampled; returns the peak, then (after the gateway has had
/// time to settle) the resident memory at rest, both in KiB.
async fn phase<F: std::future::Future<Output = ()>>(pid: u32, load: F) -> (u64, u64) {
    let sampler = Sampler::start(pid);
    load.await;
    let peak = sampler.finish();
    tokio::time::sleep(Duration::from_secs(2)).await;
    (peak, rss_kib(pid))
}

struct Findings {
    peak_mib: f64,
    /// Memory at rest after the second plain phase less that after the first.
    growth_mib: f64,
}

fn failures(f: &Findings) -> Vec<String> {
    let mut out = Vec::new();
    if f.peak_mib > BUDGET_PEAK_MIB {
        out.push(format!(
            "budget exceeded: peak resident memory under load: measured {:.1} MiB, limit {BUDGET_PEAK_MIB:.0} MiB",
            f.peak_mib
        ));
    }
    if f.growth_mib > BUDGET_GROWTH_MIB {
        out.push(format!(
            "budget exceeded: growth between two identical loads: measured {:.1} MiB, limit {BUDGET_GROWTH_MIB:.0} MiB",
            f.growth_mib
        ));
    }
    out
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "memory harness; run explicitly with --ignored"]
async fn gateway_memory_under_load() {
    let requests: usize = std::env::var("MEMORY_REQUESTS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(100_000);
    let streams: usize = std::env::var("MEMORY_STREAMS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(3_200);
    let dir = common::scratch().join("memory");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let upstream = mock_upstream(Duration::ZERO).await;
    let (gateway, base) = start_gateway(&dir, &upstream).await;
    let pid = gateway.child.id();
    let url = format!("{base}/v1/chat/completions");
    let client = reqwest::Client::builder()
        .pool_max_idle_per_host(256)
        .build()
        .unwrap();

    tokio::time::sleep(Duration::from_secs(2)).await;
    let idle = rss_kib(pid);
    let plain = |label: &'static str| {
        let (client, url) = (client.clone(), url.clone());
        async move {
            let result = phase(pid, async {
                run(&client, &url, CONCURRENCY, requests).await;
            })
            .await;
            (label, result)
        }
    };
    let first = plain("plain, 64 concurrent").await;
    let (streamed_label, streamed) = (
        "streamed, 64 concurrent",
        phase(pid, async {
            run_stream(&client, &url, CONCURRENCY, streams).await;
        })
        .await,
    );
    let second = plain("plain again").await;

    println!("\n=== gateway memory (resident set of the humpyard process) ===");
    println!("{:<28}{:>14}{:>16}", "phase", "peak (MiB)", "at rest (MiB)");
    println!("{:<28}{:>14}{:>16.1}", "idle after start", "", mib(idle));
    for (label, (peak, rest)) in [first, (streamed_label, streamed), second] {
        println!("{label:<28}{:>14.1}{:>16.1}", mib(peak), mib(rest));
    }
    let findings = Findings {
        peak_mib: mib(first.1.0.max(streamed.0).max(second.1.0)),
        growth_mib: mib(second.1.1) - mib(first.1.1),
    };
    println!(
        "\npeak {:.1} MiB (limit {BUDGET_PEAK_MIB:.0}); at rest after the second plain phase {:+.1} MiB against the first (limit {BUDGET_GROWTH_MIB:.0})",
        findings.peak_mib, findings.growth_mib
    );
    drop(gateway);
    let _ = std::fs::remove_dir_all(&dir);

    let messages = failures(&findings);
    if std::env::var("HUMPYARD_BUDGET_ENFORCE").ok().as_deref() == Some("1") {
        assert!(messages.is_empty(), "{}", messages.join("\n"));
    } else if !messages.is_empty() {
        println!(
            "(not enforced: set HUMPYARD_BUDGET_ENFORCE=1 to fail the run)\n{}",
            messages.join("\n")
        );
    }
}

#[test]
fn a_run_within_both_budgets_has_no_failures() {
    assert_eq!(
        failures(&Findings {
            peak_mib: 60.0,
            growth_mib: 2.0
        }),
        Vec::<String>::new()
    );
}

#[test]
fn each_exceeded_memory_budget_is_named_with_its_value() {
    let both = failures(&Findings {
        peak_mib: 200.0,
        growth_mib: 30.0,
    });
    assert_eq!(both.len(), 2);
    assert!(
        both[0].contains("peak resident memory") && both[0].contains("200.0 MiB"),
        "{both:?}"
    );
    assert!(
        both[1].contains("growth between two identical loads") && both[1].contains("30.0 MiB"),
        "{both:?}"
    );
}
