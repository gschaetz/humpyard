//! The admin API's front door (spec: `admin-api`, change `add-admin-api`): only an admin credential opens
//! `/admin/v1`, a client key never does and an admin credential never serves a client endpoint, the namespace
//! is absent without credentials, it can live on its own listener, every refusal looks the same, outcomes are
//! counted, and credentials follow a configuration reload.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test scaffolding

mod common;

use std::time::Duration;

use humpyard::config::Config;
use humpyard::server::{self, Reload};
use serde_json::Value;
use tokio::sync::mpsc;

/// Every gateway gets its own ledger file, so tests that run side by side never share a database.
static NEXT_LEDGER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

struct Secrets {
    admin: String,
    admin_hash: String,
    reader: String,
    reader_hash: String,
    client: String,
    client_hash: String,
    metrics: String,
    metrics_hash: String,
    ledger: std::path::PathBuf,
}

fn secrets() -> Secrets {
    let gen_key = || humpyard::auth::generate_key().unwrap();
    let (admin, admin_hash) = gen_key();
    let (reader, reader_hash) = gen_key();
    let (client, client_hash) = gen_key();
    let (metrics, metrics_hash) = gen_key();
    Secrets {
        admin,
        admin_hash,
        reader,
        reader_hash,
        client,
        client_hash,
        metrics,
        metrics_hash,
        ledger: std::env::temp_dir().join(format!(
            "humpyard-adminapi-{}-{}.db",
            std::process::id(),
            NEXT_LEDGER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        )),
    }
}

/// A client key (`ops`, which may read the configuration and manage overrides, the strongest a client can be)
/// and the metrics credential, plus whatever `[admin]` text is given.
fn config_text(s: &Secrets, admin: &str) -> String {
    format!(
        r#"listen = "127.0.0.1:0"
[metrics]
enabled = true
bearer_sha256 = "{metrics}"
[keys.ops]
sha256 = "{client}"
read_config = true
manage_overrides = true
[providers.p]
base_url = "http://127.0.0.1:1"
api_key_env = "PATH"
max_retries = 0
[targets.t]
endpoints = [{{ provider = "p", model = "m" }}]
[routes.r]
type = "passthrough"
targets = ["t"]
[ledger]
path = "{ledger}"
{admin}
"#,
        metrics = s.metrics_hash,
        client = s.client_hash,
        ledger = s.ledger.display(),
    )
}

fn admin_section(s: &Secrets, extra: &str) -> String {
    format!(
        "{extra}[admin.credentials.gary]\nsha256 = \"{}\"\nscopes = [\"read\", \"overrides\"]\nexpires = 2999-01-01T00:00:00Z\n\
         [admin.credentials.sam]\nsha256 = \"{}\"\nscopes = [\"read\"]\n",
        s.admin_hash, s.reader_hash
    )
}

struct Gateway {
    url: String,
    path: std::path::PathBuf,
    reload: mpsc::Sender<()>,
    stop: mpsc::Sender<()>,
    task: tokio::task::JoinHandle<Result<(), String>>,
    _dir: tempfile::TempDir,
}

async fn start(text: &str) -> Gateway {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("humpyard.toml");
    std::fs::write(&path, text).unwrap();
    let config = Config::from_toml(text, |n| std::env::var(n).ok()).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let (reload, triggers) = mpsc::channel(4);
    let (stop, signals) = mpsc::channel(4);
    let task = tokio::spawn(server::run(
        config,
        listener,
        signals,
        Some(Reload {
            path: path.clone(),
            triggers,
        }),
    ));
    Gateway {
        url,
        path,
        reload,
        stop,
        task,
        _dir: dir,
    }
}

impl Gateway {
    async fn finish(self) {
        self.stop.send(()).await.unwrap();
        self.task.await.unwrap().unwrap();
    }
}

async fn get(
    url: &str,
    path: &str,
    secret: Option<&str>,
) -> (u16, reqwest::header::HeaderMap, Value) {
    let mut request = reqwest::Client::new().get(format!("{url}{path}"));
    if let Some(secret) = secret {
        request = request.bearer_auth(secret);
    }
    let response = request.send().await.unwrap();
    let status = response.status().as_u16();
    let headers = response.headers().clone();
    let body = response.json().await.unwrap_or(Value::Null);
    (status, headers, body)
}

#[tokio::test]
async fn without_credentials_the_namespace_does_not_exist() {
    let s = secrets();
    let gateway = start(&config_text(&s, "")).await;
    for secret in [None, Some(s.client.as_str()), Some(s.admin.as_str())] {
        let (status, _, _) = get(&gateway.url, "/admin/v1/whoami", secret).await;
        assert_eq!(status, 404);
    }
    gateway.finish().await;
}

#[tokio::test]
async fn an_admin_credential_says_who_it_is() {
    let s = secrets();
    let gateway = start(&config_text(&s, &admin_section(&s, ""))).await;
    let (status, _, body) = get(&gateway.url, "/admin/v1/whoami", Some(&s.admin)).await;
    assert_eq!(status, 200);
    assert_eq!(body["id"], "gary");
    assert_eq!(body["kind"], "credential");
    assert_eq!(body["scopes"], serde_json::json!(["read", "overrides"]));
    assert_eq!(body["expires"], "2999-01-01T00:00:00Z");
    let (_, _, sam) = get(&gateway.url, "/admin/v1/whoami", Some(&s.reader)).await;
    assert_eq!(
        (sam["id"].as_str(), sam["expires"].is_null()),
        (Some("sam"), true)
    );
    gateway.finish().await;
}

#[tokio::test]
async fn nothing_but_an_admin_credential_opens_the_admin_api_and_every_refusal_looks_alike() {
    let s = secrets();
    let gateway = start(&config_text(&s, &admin_section(&s, ""))).await;
    // The strongest client key, the metrics credential, nothing, and a made-up secret.
    let mut seen = Vec::new();
    for secret in [
        Some(s.client.as_str()),
        Some(s.metrics.as_str()),
        None,
        Some("not-a-real-credential"),
    ] {
        let (status, headers, body) = get(&gateway.url, "/admin/v1/whoami", secret).await;
        assert_eq!(status, 401, "{secret:?}");
        assert_eq!(headers["www-authenticate"], "Bearer");
        assert_eq!(body["error"]["code"], "admin_credential_invalid");
        seen.push(body);
    }
    assert!(
        seen.windows(2).all(|pair| pair[0] == pair[1]),
        "one shape for every reason"
    );
    gateway.finish().await;
}

#[tokio::test]
async fn an_expired_credential_is_refused_like_any_other() {
    let s = secrets();
    let admin = format!(
        "[admin.credentials.old]\nsha256 = \"{}\"\nscopes = [\"read\"]\nexpires = 2020-01-01T00:00:00Z\n",
        s.admin_hash
    );
    let gateway = start(&config_text(&s, &admin)).await;
    let (status, _, body) = get(&gateway.url, "/admin/v1/whoami", Some(&s.admin)).await;
    assert_eq!(status, 401);
    assert_eq!(body["error"]["code"], "admin_credential_invalid");
    gateway.finish().await;
}

#[tokio::test]
async fn an_admin_credential_does_not_serve_client_endpoints() {
    let s = secrets();
    let gateway = start(&config_text(&s, &admin_section(&s, ""))).await;
    for path in [
        "/v1/models",
        "/v1/key/info",
        "/v1/config",
        "/v1/health",
        "/v1/overrides",
    ] {
        let (status, _, body) = get(&gateway.url, path, Some(&s.admin)).await;
        assert_eq!(status, 401, "{path}");
        assert!(
            body["error"]["message"]
                .as_str()
                .unwrap()
                .contains("invalid API key"),
            "{path}: {body}"
        );
    }
    // And inference is refused before any upstream is touched (the provider here is unreachable).
    let response = reqwest::Client::new()
        .post(format!("{}/v1/chat/completions", gateway.url))
        .bearer_auth(&s.admin)
        .json(&common::chat_request("r", false))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 401);
    gateway.finish().await;
}

#[tokio::test]
async fn the_admin_api_can_have_its_own_listener_and_is_then_off_the_public_port() {
    let s = secrets();
    let spare = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = spare.local_addr().unwrap();
    drop(spare);
    let gateway = start(&config_text(
        &s,
        &admin_section(&s, &format!("[admin]\nlisten = \"{addr}\"\n")),
    ))
    .await;
    wait_listening(addr).await;
    let own = format!("http://{addr}");
    let (status, _, body) = get(&own, "/admin/v1/whoami", Some(&s.admin)).await;
    assert_eq!((status, body["id"].as_str()), (200, Some("gary")));
    let (public, _, _) = get(&gateway.url, "/admin/v1/whoami", Some(&s.admin)).await;
    assert_eq!(public, 404, "never on the public port");
    // The admin listener serves nothing else.
    let (other, _, _) = get(&own, "/v1/models", Some(&s.client)).await;
    assert_eq!(other, 404);
    gateway.finish().await;
}

/// Waits until something accepts connections at `addr`: the own admin listener is bound a moment after the
/// gateway starts, and a fixed pause is not enough on a slow machine.
async fn wait_listening(addr: std::net::SocketAddr) {
    for _ in 0..200 {
        if tokio::net::TcpStream::connect(addr).await.is_ok() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("nothing listened on {addr} within five seconds");
}

async fn metrics(gateway: &Gateway, s: &Secrets) -> String {
    reqwest::Client::new()
        .get(format!("{}/metrics", gateway.url))
        .bearer_auth(&s.metrics)
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap()
}

#[tokio::test]
async fn outcomes_are_counted_without_naming_anyone_and_only_while_the_admin_api_is_on() {
    let s = secrets();
    let off = start(&config_text(&s, "")).await;
    let quiet = metrics(&off, &s).await;
    assert!(!quiet.contains("humpyard_admin_requests_total"));
    assert!(!quiet.contains("humpyard_admin_revocations_active"));
    off.finish().await;

    let gateway = start(&config_text(&s, &admin_section(&s, ""))).await;
    get(&gateway.url, "/admin/v1/whoami", Some(&s.admin)).await;
    get(&gateway.url, "/admin/v1/whoami", Some(&s.admin)).await;
    get(&gateway.url, "/admin/v1/whoami", Some(&s.client)).await;
    let text = metrics(&gateway, &s).await;
    assert!(
        text.contains("humpyard_admin_requests_total{result=\"ok\"} 2"),
        "{text}"
    );
    assert!(
        text.contains("humpyard_admin_requests_total{result=\"unauthorized\"} 1"),
        "{text}"
    );
    for hidden in ["gary", "sam", s.admin.as_str(), s.client.as_str()] {
        let counters: String = text
            .lines()
            .filter(|l| l.contains("admin_requests_total"))
            .collect();
        assert!(!counters.contains(hidden), "the counter names `{hidden}`");
    }
    gateway.finish().await;
}

async fn wait_for(gateway: &Gateway, secret: &str, status: u16) {
    for _ in 0..100 {
        if get(&gateway.url, "/admin/v1/whoami", Some(secret)).await.0 == status {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("a reload did not make `whoami` answer {status}");
}

#[tokio::test]
async fn credentials_follow_a_reload_and_a_rejected_reload_changes_nothing() {
    let s = secrets();
    let gateway = start(&config_text(&s, &admin_section(&s, ""))).await;
    // Narrow gary, remove sam, add nobody.
    let narrowed = format!(
        "[admin.credentials.gary]\nsha256 = \"{}\"\nscopes = [\"read\"]\n",
        s.admin_hash
    );
    std::fs::write(&gateway.path, config_text(&s, &narrowed)).unwrap();
    gateway.reload.send(()).await.unwrap();
    wait_for(&gateway, &s.reader, 401).await;
    let (_, _, gary) = get(&gateway.url, "/admin/v1/whoami", Some(&s.admin)).await;
    assert_eq!(gary["scopes"], serde_json::json!(["read"]));
    assert!(
        gary["expires"].is_null(),
        "the expiry went with the old entry"
    );

    // A file that does not load leaves the credentials in force.
    std::fs::write(&gateway.path, "this is not toml = = =").unwrap();
    gateway.reload.send(()).await.unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;
    let (status, _, body) = get(&gateway.url, "/admin/v1/whoami", Some(&s.admin)).await;
    assert_eq!((status, body["id"].as_str()), (200, Some("gary")));
    assert_eq!(
        get(&gateway.url, "/admin/v1/whoami", Some(&s.reader))
            .await
            .0,
        401
    );

    // Removing every credential turns the API off again.
    std::fs::write(&gateway.path, config_text(&s, "")).unwrap();
    gateway.reload.send(()).await.unwrap();
    wait_for(&gateway, &s.admin, 404).await;
    gateway.finish().await;
}

#[tokio::test]
async fn moving_the_admin_listener_needs_a_restart() {
    let s = secrets();
    let spare = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = spare.local_addr().unwrap();
    drop(spare);
    let gateway = start(&config_text(
        &s,
        &admin_section(&s, &format!("[admin]\nlisten = \"{addr}\"\n")),
    ))
    .await;
    wait_listening(addr).await;
    // The same credentials, but no own listener any more.
    std::fs::write(&gateway.path, config_text(&s, &admin_section(&s, ""))).unwrap();
    gateway.reload.send(()).await.unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;
    // Refused: the own listener keeps serving and the public port still has no admin API.
    let (own, _, _) = get(
        &format!("http://{addr}"),
        "/admin/v1/whoami",
        Some(&s.admin),
    )
    .await;
    assert_eq!(own, 200);
    assert_eq!(
        get(&gateway.url, "/admin/v1/whoami", Some(&s.admin))
            .await
            .0,
        404
    );
    gateway.finish().await;
}
