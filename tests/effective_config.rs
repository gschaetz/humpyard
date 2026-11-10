//! The effective configuration (spec: `gateway-api`, `metrics`, `cli`): `GET /v1/config` with sources and a
//! fingerprint, no secrets, opt-in per key, the fingerprint metric, `ctl config`, and `check-config`'s fingerprint.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test scaffolding

mod common;

use std::path::PathBuf;

use common::serve;
use humpyard::config::Config;
use humpyard::ctl::{Command, Settings, connect, run};
use humpyard::server;
use serde_json::Value;

const HEADER_VALUE: &str = "header-value-that-must-not-appear";

struct Fixture {
    url: String,
    ops_key: String,
    app_key: String,
    app_hash: String,
    metrics_key: String,
    main_path: PathBuf,
}

fn temp(name: &str) -> PathBuf {
    let dir = common::scratch().join(format!(
        "humpyard-effcfg-{}-{name}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A gateway with a hand-written target, a generated one, a pool route, an operator key and an ordinary key.
async fn fixture(name: &str, with_keys: bool) -> Fixture {
    let dir = temp(name);
    let generated = dir.join("discovered.toml");
    std::fs::write(
        &generated,
        r#"
[targets.auto-free-alpha]
strengths = ["general", "all"]
endpoints = [{ provider = "free", model = "alpha", context_window = 32000, price = { input = 0.0, output = 0.0 } }]
"#,
    )
    .unwrap();
    let (ops_key, ops_hash) = humpyard::auth::generate_key().unwrap();
    let (app_key, app_hash) = humpyard::auth::generate_key().unwrap();
    let (_, member_hash) = humpyard::auth::generate_key().unwrap();
    let (metrics_key, metrics_hash) = humpyard::auth::generate_key().unwrap();
    let keys = if with_keys {
        format!(
            "[keys.ops]\nsha256 = \"{ops_hash}\"\nread_config = true\nmax_requests_per_minute = 60\nmax_concurrent = 4\n[keys.app]\nsha256 = \"{app_hash}\"\n[profiles.team]\nmax_requests_per_minute = 30\nmax_concurrent = 2\n[keys.member]\nsha256 = \"{member_hash}\"\nprofile = \"team\"\nmax_concurrent = 1\n"
        )
    } else {
        String::new()
    };
    let main_toml = format!(
        r#"listen = "127.0.0.1:0"
[metrics]
enabled = true
bearer_sha256 = "{metrics_hash}"
[discovery]
enabled = true
file = "{generated}"
[providers.free]
base_url = "http://127.0.0.1:1"
api_key_env = "PATH"
headers = {{ "x-session" = "{HEADER_VALUE}" }}
discover = {{ strengths = ["general"] }}
price_source = "models-dev:groq"
[providers.lab]
base_url = "http://127.0.0.1:1"
api_key_env = "PATH"
[targets.home]
class = "private"
strengths = ["general"]
endpoints = [{{ provider = "lab", model = "home-model", context_window = 131072, max_request_tokens = 8000 }}]
[routes.pool]
type = "fastest"
pool = {{ strength = "general" }}
{keys}"#,
        generated = generated.display()
    );
    let main_path = dir.join("config.toml");
    std::fs::write(&main_path, &main_toml).unwrap();
    let config = Config::load(&main_path).unwrap();
    let url = format!(
        "http://{}",
        serve(server::router(config).await.unwrap()).await
    );
    Fixture {
        url,
        ops_key,
        app_key,
        app_hash,
        metrics_key,
        main_path,
    }
}

async fn get(url: &str, path: &str, key: Option<&str>) -> (u16, String) {
    let mut request = reqwest::Client::new().get(format!("{url}{path}"));
    if let Some(key) = key {
        request = request.bearer_auth(key);
    }
    let response = request.send().await.unwrap();
    (response.status().as_u16(), response.text().await.unwrap())
}

#[tokio::test]
async fn each_target_says_where_it_came_from_and_pools_show_what_they_resolved_to() {
    let f = fixture("sources", true).await;
    let (status, body) = get(&f.url, "/v1/config", Some(&f.ops_key)).await;
    assert_eq!(status, 200, "{body}");
    let v: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["targets"]["home"]["source"], "file");
    assert_eq!(v["targets"]["auto-free-alpha"]["source"], "generated");
    assert_eq!(v["routes"]["pool"]["source"], "file");
    let members = v["routes"]["pool"]["resolved"]["members"]
        .as_array()
        .unwrap();
    assert!(members.iter().any(|m| m == "auto-free-alpha") && members.iter().any(|m| m == "home"));
    assert!(
        v["config"]["fingerprint"]
            .as_str()
            .is_some_and(|s| !s.is_empty())
    );
}

#[tokio::test]
async fn a_keys_rate_and_concurrency_limits_are_shown_and_absent_ones_are_null() {
    let f = fixture("key-limits", true).await;
    let (status, body) = get(&f.url, "/v1/config", Some(&f.ops_key)).await;
    assert_eq!(status, 200, "{body}");
    let v: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["keys"]["ops"]["max_requests_per_minute"], 60);
    assert_eq!(v["keys"]["ops"]["max_concurrent"], 4);
    assert!(v["keys"]["app"]["max_requests_per_minute"].is_null());
    assert!(v["keys"]["app"]["max_concurrent"].is_null());
}

#[tokio::test]
async fn no_secret_is_in_the_view() {
    let f = fixture("secrets", true).await;
    let (_, body) = get(&f.url, "/v1/config", Some(&f.ops_key)).await;
    let path_value = std::env::var("PATH").unwrap();
    assert!(!body.contains(&path_value), "the provider's API key value");
    assert!(!body.contains(HEADER_VALUE), "a provider header value");
    assert!(body.contains("x-session"), "header names are shown");
    assert!(
        !body.contains(f.app_hash.trim_start_matches("sha256:")),
        "a key hash"
    );
    assert!(!body.contains(&f.ops_key) && !body.contains(&f.app_key));
}

#[tokio::test]
async fn only_a_key_that_opted_in_may_read_it() {
    let f = fixture("optin", true).await;
    let (status, body) = get(&f.url, "/v1/config", Some(&f.app_key)).await;
    assert_eq!(status, 403);
    let v: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["error"]["code"], "config_not_readable");
    assert!(
        !body.contains("home-model"),
        "nothing of the configuration leaks"
    );
    assert_eq!(get(&f.url, "/v1/config", None).await.0, 401);
    assert_eq!(get(&f.url, "/v1/config", Some(&f.ops_key)).await.0, 200);
}

#[tokio::test]
async fn a_gateway_with_no_keys_is_open_to_the_read() {
    let f = fixture("open", false).await;
    assert_eq!(get(&f.url, "/v1/config", None).await.0, 200);
}

#[tokio::test]
async fn the_fingerprint_is_the_same_in_the_view_the_health_report_and_the_metric() {
    let f = fixture("fingerprint", true).await;
    let view: Value =
        serde_json::from_str(&get(&f.url, "/v1/config", Some(&f.ops_key)).await.1).unwrap();
    let health: Value =
        serde_json::from_str(&get(&f.url, "/v1/health", Some(&f.app_key)).await.1).unwrap();
    let fingerprint = view["config"]["fingerprint"].as_str().unwrap();
    assert_eq!(health["config"]["fingerprint"], fingerprint);
    let (status, metrics) = get(&f.url, "/metrics", Some(&f.metrics_key)).await;
    assert_eq!(status, 200, "{metrics}");
    let line = format!("humpyard_config_info{{fingerprint=\"{fingerprint}\"}} 1");
    assert!(metrics.contains(&line), "{metrics}");
    assert_eq!(
        metrics.matches("humpyard_config_info{").count(),
        1,
        "one series"
    );
}

#[tokio::test]
async fn ctl_config_shows_sources_prints_only_the_fingerprint_on_request_and_is_refused_without_opt_in()
 {
    let f = fixture("ctl", true).await;
    let ctl = |key: String, command: Command, json: bool| {
        let url = f.url.clone();
        async move {
            let env = |n: &str| (n == "CTL_KEY").then(|| key.clone());
            let settings = Settings {
                url: Some(url),
                key_env: "CTL_KEY".into(),
                allow_insecure_http: false,
                json,
            };
            let gateway = connect(&settings, &env).unwrap();
            let mut out: Vec<u8> = Vec::new();
            let result = run(&gateway, command, json, &mut out).await;
            (result, String::from_utf8(out).unwrap())
        }
    };
    let (result, text) = ctl(
        f.ops_key.clone(),
        Command::Config { fingerprint: false },
        false,
    )
    .await;
    result.unwrap();
    assert!(
        text.contains("auto-free-alpha") && text.contains("generated") && text.contains("SOURCE"),
        "{text}"
    );
    let (result, only) = ctl(
        f.ops_key.clone(),
        Command::Config { fingerprint: true },
        false,
    )
    .await;
    result.unwrap();
    let view: Value =
        serde_json::from_str(&get(&f.url, "/v1/config", Some(&f.ops_key)).await.1).unwrap();
    assert_eq!(only.trim(), view["config"]["fingerprint"].as_str().unwrap());
    let (result, _) = ctl(
        f.app_key.clone(),
        Command::Config { fingerprint: false },
        false,
    )
    .await;
    let failure = result.unwrap_err();
    assert_eq!(failure.exit_code(), 1);
    assert!(failure.message().contains("403"));
}

#[tokio::test]
async fn check_config_prints_the_fingerprint_and_it_follows_the_files() {
    let f = fixture("checkconfig", true).await;
    let run_check = || {
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_humpyard"))
            .args(["check-config", f.main_path.to_str().unwrap()])
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout)
            .unwrap()
            .lines()
            .find_map(|l| l.strip_prefix("fingerprint: ").map(String::from))
            .expect("a fingerprint line")
    };
    let first = run_check();
    assert_eq!(run_check(), first, "stable for the same files");
    let config = Config::load(&f.main_path).unwrap();
    assert_eq!(
        server::config_fingerprint(&f.main_path, &config).unwrap(),
        first
    );
    let mut text = std::fs::read_to_string(&f.main_path).unwrap();
    text.push_str("\n# a change\n");
    std::fs::write(&f.main_path, text).unwrap();
    assert_ne!(run_check(), first, "it changes when the file does");
}

#[tokio::test]
async fn a_key_shows_its_profile_and_the_values_it_ended_up_with_and_profiles_are_listed() {
    let f = fixture("key-profiles", true).await;
    let (status, body) = get(&f.url, "/v1/config", Some(&f.ops_key)).await;
    assert_eq!(status, 200, "{body}");
    let v: Value = serde_json::from_str(&body).unwrap();
    let member = &v["keys"]["member"];
    assert_eq!(member["profile"], "team");
    assert_eq!(member["max_requests_per_minute"], 30, "inherited");
    assert_eq!(
        member["max_concurrent"], 1,
        "its own, narrower than the profile's"
    );
    assert!(v["keys"]["app"]["profile"].is_null());
    assert_eq!(v["profiles"]["team"]["max_requests_per_minute"], 30);
    assert_eq!(v["profiles"]["team"]["max_concurrent"], 2);
    assert!(v["profiles"]["team"]["daily_usd"].is_null());
    assert!(!body.contains("sha256"), "no hash in the view");
}

#[tokio::test]
async fn an_endpoint_shows_its_request_cap_and_omits_it_when_unset() {
    let f = fixture("request-cap", true).await;
    let (status, body) = get(&f.url, "/v1/config", Some(&f.ops_key)).await;
    assert_eq!(status, 200, "{body}");
    let v: Value = serde_json::from_str(&body).unwrap();
    let capped = &v["targets"]["home"]["endpoints"][0];
    assert_eq!(capped["context_window"], 131_072);
    assert_eq!(capped["max_request_tokens"], 8000);
    let uncapped = &v["targets"]["auto-free-alpha"]["endpoints"][0];
    assert!(uncapped.get("max_request_tokens").is_none(), "{uncapped}");
}
