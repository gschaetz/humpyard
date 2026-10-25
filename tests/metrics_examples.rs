//! The shipped dashboard and alert examples only use metrics the gateway really exposes, so a
//! renamed metric breaks the build instead of a dashboard (ADR 0016).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test scaffolding

mod common;

use std::collections::BTreeMap;

use common::serve;
use humpyard::config::Config;
use humpyard::server;

/// Metric names (with their TYPE) from a live scrape of a gateway that has a ledger.
async fn exposed() -> BTreeMap<String, String> {
    let dir = tempfile::tempdir().unwrap();
    let (credential, hash) = humpyard::auth::generate_key().unwrap();
    let (_, admin_hash) = humpyard::auth::generate_key().unwrap();
    let toml = format!(
        r#"listen = "127.0.0.1:0"
[metrics]
enabled = true
bearer_sha256 = "{hash}"
[ledger]
path = "{db}"
[admin.credentials.ops]
sha256 = "{admin_hash}"
scopes = ["read"]
[providers.p]
base_url = "http://127.0.0.1:1/v1"
api_key_env = "K"
[targets.a]
endpoints = [{{ provider = "p", model = "m" }}]
"#,
        db = dir.path().join("l.db").display()
    );
    let config = Config::from_toml(&toml, |_| Some("k".into())).unwrap();
    let addr = serve(server::router(config).await.unwrap()).await;
    let text = reqwest::Client::new()
        .get(format!("http://{addr}/metrics"))
        .bearer_auth(credential)
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    text.lines()
        .filter_map(|l| l.strip_prefix("# TYPE "))
        .map(|rest| {
            let (name, kind) = rest.split_once(' ').unwrap();
            (name.to_string(), kind.to_string())
        })
        .collect()
}

/// Every `humpyard_*` identifier in `text`.
fn metric_names(text: &str) -> Vec<String> {
    let mut names = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0;
    while let Some(at) = text[i..].find("humpyard_") {
        let start = i + at;
        let mut end = start;
        while end < bytes.len()
            && (bytes[end].is_ascii_lowercase()
                || bytes[end].is_ascii_digit()
                || bytes[end] == b'_')
        {
            end += 1;
        }
        names.push(text[start..end].to_string());
        i = end;
    }
    names
}

#[tokio::test]
async fn every_metric_in_the_examples_exists() {
    let exposed = exposed().await;
    assert!(exposed.contains_key("humpyard_requests_total"));
    let mut checked = 0;
    for file in [
        "grafana-dashboard.json",
        "grafana-summary-dashboard.json",
        "alert-rules.yaml",
        "prometheusrule.yaml",
    ] {
        let text = std::fs::read_to_string(format!("examples/metrics/{file}")).unwrap();
        for name in metric_names(&text) {
            let base = ["_bucket", "_sum", "_count"]
                .iter()
                .find_map(|suffix| {
                    name.strip_suffix(suffix)
                        .filter(|b| exposed.get(*b).is_some_and(|k| k == "histogram"))
                })
                .unwrap_or(&name);
            assert!(
                exposed.contains_key(base),
                "{file} uses `{name}`, which the gateway does not expose; known: {:?}",
                exposed.keys().collect::<Vec<_>>()
            );
            checked += 1;
        }
    }
    assert!(
        checked > 20,
        "the examples reference the metrics ({checked})"
    );
}

#[test]
fn the_example_manifests_name_the_metrics_port() {
    let monitor = std::fs::read_to_string("examples/metrics/servicemonitor.yaml").unwrap();
    assert!(monitor.contains("port: metrics"));
    let policy = std::fs::read_to_string("examples/metrics/networkpolicy.yaml").unwrap();
    assert!(policy.contains("port: 9100"));
}
