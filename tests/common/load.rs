//! Request traffic for the harnesses: a number of requests spread over a number of concurrent workers, with the
//! per-request latency summarised. Shared by the overhead harness and the isolation tests.
#![allow(clippy::cast_precision_loss)] // reporting only

use std::time::Instant;

use futures::StreamExt;

use super::chat_request;

pub struct Run {
    pub p50: f64,
    pub p90: f64,
    pub p99: f64,
    pub rps: f64,
}

/// The `percent`th percentile (nearest rank) of an ascending list.
pub fn percentile(sorted: &[f64], percent: usize) -> f64 {
    let index = ((sorted.len() - 1) * percent + 50) / 100;
    sorted[index.min(sorted.len() - 1)]
}

/// `requests` requests spread over `concurrency` workers; per-request latency in milliseconds.
pub async fn run(client: &reqwest::Client, url: &str, concurrency: usize, requests: usize) -> Run {
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

/// What a streaming run found: time to the first body chunk (what a client sees as the first token) and time to the
/// end of the stream, in milliseconds, and streams per second.
pub struct StreamRun {
    pub ttfb_p50: f64,
    pub ttfb_p99: f64,
    pub total_p50: f64,
    pub total_p99: f64,
    pub rps: f64,
}

/// `requests` streaming requests spread over `concurrency` workers, each read to the end of its stream.
pub async fn run_stream(
    client: &reqwest::Client,
    url: &str,
    concurrency: usize,
    requests: usize,
) -> StreamRun {
    let body = chat_request("r", true);
    let per_worker = requests.div_ceil(concurrency);
    let started = Instant::now();
    let workers: Vec<_> = (0..concurrency)
        .map(|_| {
            let (client, url, body) = (client.clone(), url.to_string(), body.clone());
            tokio::spawn(async move {
                let mut times = Vec::with_capacity(per_worker);
                for _ in 0..per_worker {
                    let t = Instant::now();
                    let resp = client.post(&url).json(&body).send().await.unwrap();
                    assert_eq!(resp.status(), 200);
                    let mut stream = resp.bytes_stream();
                    let first = stream.next().await;
                    assert!(first.is_some(), "the stream was empty");
                    let ttfb = t.elapsed().as_secs_f64() * 1000.0;
                    while stream.next().await.is_some() {}
                    times.push((ttfb, t.elapsed().as_secs_f64() * 1000.0));
                }
                times
            })
        })
        .collect();
    let mut all = Vec::new();
    for worker in workers {
        all.extend(worker.await.unwrap());
    }
    let elapsed = started.elapsed().as_secs_f64();
    let mut ttfb: Vec<f64> = all.iter().map(|t| t.0).collect();
    let mut total: Vec<f64> = all.iter().map(|t| t.1).collect();
    ttfb.sort_by(f64::total_cmp);
    total.sort_by(f64::total_cmp);
    StreamRun {
        ttfb_p50: percentile(&ttfb, 50),
        ttfb_p99: percentile(&ttfb, 99),
        total_p50: percentile(&total, 50),
        total_p99: percentile(&total, 99),
        rps: all.len() as f64 / elapsed,
    }
}

/// How a harness's request traffic authenticates (the environment variable it reads says which): `actor`, a client
/// key that names an agent actor owned by a human, so every request goes through the key lookup, its revocation check
/// and the actor and owner written on its decision; `plain`, a key with no actor; or `none`, no keys, an open
/// gateway. The three differ only in what each request does with its key, which is how the cost of an actor is seen
/// apart from the cost of a key.
pub struct LoadKey {
    /// The name of the mode, for the report.
    pub mode: String,
    /// The configuration for it: a `[keys.load]` table, with its actors when it names one.
    pub toml: String,
    /// The secret the traffic sends, if any.
    pub secret: Option<String>,
}

/// The mode named by environment variable `var` (`default` when it is not set), and the key it implies.
pub fn load_key(var: &str, default: &str) -> LoadKey {
    let mode = std::env::var(var).unwrap_or_else(|_| default.to_string());
    if mode == "none" {
        return LoadKey {
            mode,
            toml: String::new(),
            secret: None,
        };
    }
    let (secret, hash) = humpyard::auth::generate_key().unwrap();
    let toml = match mode.as_str() {
        "plain" => format!("[keys.load]\nsha256 = \"{hash}\"\n"),
        "actor" => format!(
            "[actors.gary]\nkind = \"human\"\n[actors.load-agent]\nkind = \"agent\"\nowner = \"gary\"\n\
             [keys.load]\nsha256 = \"{hash}\"\nactor = \"load-agent\"\n"
        ),
        other => panic!("{var} must be actor, plain or none, not `{other}`"),
    };
    LoadKey {
        mode,
        toml,
        secret: Some(secret),
    }
}

/// A client that sends the load key (when there is one) on every request. A request that sets its own credential, as
/// the administrative reads and the metrics scrape do, replaces it.
pub fn client_for(load: &LoadKey, idle: Option<usize>) -> reqwest::Client {
    let mut builder = reqwest::Client::builder();
    if let Some(idle) = idle {
        builder = builder.pool_max_idle_per_host(idle);
    }
    if let Some(secret) = &load.secret {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            reqwest::header::AUTHORIZATION,
            format!("Bearer {secret}").parse().unwrap(),
        );
        builder = builder.default_headers(headers);
    }
    builder.build().unwrap()
}
