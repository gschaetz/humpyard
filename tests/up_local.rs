#![cfg(unix)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test scaffolding: fail loudly
//! The `/local` surface of `humpyard up` (spec: `single-machine-up`): the boundary the GUI's service already has,
//! write-only provider keys, the restart that makes a key take effect, and the test request.

mod common;
#[path = "common/up_harness.rs"]
mod harness;

use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use axum::Router;
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use common::{chat_completion, serve, usage};
use harness::{Up, connects, free_port, state};
use serde_json::{Value, json};

const GOOD: &str = "good-key-aaaaaaaa";
const OTHER_GOOD: &str = "good-key-bbbbbbbb";
const BAD: &str = "bad-key-zzzzzzzz";

fn authorised(headers: &HeaderMap) -> bool {
    headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.starts_with("Bearer good-key"))
}

/// A provider that accepts keys starting `good-key`, lists `m`, and answers chat (or fails it when asked).
async fn provider(fail_chat: bool) -> String {
    let app = Router::new()
        .route(
            "/models",
            get(|headers: HeaderMap| async move {
                if authorised(&headers) {
                    axum::Json(json!({"data": [{"id": "m"}]})).into_response()
                } else {
                    StatusCode::UNAUTHORIZED.into_response()
                }
            }),
        )
        .route(
            "/chat/completions",
            post(move |headers: HeaderMap| async move {
                if !authorised(&headers) {
                    StatusCode::UNAUTHORIZED.into_response()
                } else if fail_chat {
                    StatusCode::INTERNAL_SERVER_ERROR.into_response()
                } else {
                    axum::Json(chat_completion("m", "ok", Some(usage(5, 3)))).into_response()
                }
            }),
        );
    format!("http://{}", serve(app).await)
}

fn write_config(dir: &Path, port: u16, base: &str, extra: &str) {
    std::fs::create_dir_all(dir).unwrap();
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700)).unwrap();
    let text = format!(
        r#"listen = "127.0.0.1:{port}"
shutdown_grace_secs = 2
[providers.mock]
base_url = "{base}"
api_key_env = "MOCK_KEY"
max_retries = 0
[targets.fast]
endpoints = [{{ provider = "mock", model = "m" }}]
{extra}"#
    );
    std::fs::write(dir.join("config.toml"), text).unwrap();
}

/// A signed-in client of the GUI's address.
struct Page {
    client: reqwest::Client,
    gui: String,
    cookie: String,
}

impl Page {
    async fn sign_in(up: &mut Up) -> Self {
        let (gui, token) = up.gui();
        let client = reqwest::Client::new();
        let session = client
            .post(format!("{gui}/ui/session"))
            .json(&json!({ "token": token }))
            .send()
            .await
            .unwrap();
        assert_eq!(session.status(), 204);
        let cookie = session.headers()["set-cookie"]
            .to_str()
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .to_string();
        Self {
            client,
            gui,
            cookie,
        }
    }

    async fn send(&self, request: reqwest::RequestBuilder) -> (u16, String) {
        let response = request.header("cookie", &self.cookie).send().await.unwrap();
        (response.status().as_u16(), response.text().await.unwrap())
    }

    async fn get(&self, path: &str) -> (u16, Value) {
        let (status, text) = self
            .send(self.client.get(format!("{}{path}", self.gui)))
            .await;
        (status, serde_json::from_str(&text).unwrap_or(Value::Null))
    }

    /// Returns the status, the parsed body and the raw text (to check that no key is in it).
    async fn post(&self, path: &str, body: &Value) -> (u16, Value, String) {
        let (status, text) = self
            .send(self.client.post(format!("{}{path}", self.gui)).json(body))
            .await;
        (
            status,
            serde_json::from_str(&text).unwrap_or(Value::Null),
            text,
        )
    }

    async fn delete(&self, path: &str) -> (u16, Value, String) {
        let (status, text) = self
            .send(
                self.client
                    .delete(format!("{}{path}", self.gui))
                    .header("content-type", "application/json"),
            )
            .await;
        (
            status,
            serde_json::from_str(&text).unwrap_or(Value::Null),
            text,
        )
    }

    async fn key_status(&self) -> Value {
        let (status, body) = self.get("/local/keys").await;
        assert_eq!(status, 200);
        body["providers"][0]["source"].clone()
    }
}

fn args(ui_port: u16) -> Vec<String> {
    vec!["--ui-listen".into(), format!("127.0.0.1:{ui_port}")]
}

fn key_file(dir: &Path) -> std::path::PathBuf {
    dir.join("keys.env")
}

// Multi-threaded in every test below: the test blocks on the process while the mock provider must still answer.

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_request_without_the_session_or_from_elsewhere_changes_nothing() {
    let dir = state("local-boundary");
    let (port, ui_port) = (free_port(), free_port());
    let base = provider(false).await;
    write_config(&dir, port, &base, "");
    let mut up = Up::spawn_with(&dir, &args(ui_port), &[]);
    let page = Page::sign_in(&mut up).await;
    let body = json!({"provider": "mock", "key": GOOD, "save": true});
    let url = format!("{}/local/keys", page.gui);

    // No session.
    let anonymous = reqwest::Client::new();
    for request in [
        anonymous.get(format!("{}/local/keys", page.gui)),
        anonymous.post(&url).json(&body),
        anonymous
            .delete(format!("{}/local/keys/mock", page.gui))
            .header("content-type", "application/json"),
        anonymous
            .post(format!("{}/local/test", page.gui))
            .json(&json!({})),
    ] {
        assert_eq!(request.send().await.unwrap().status(), 401);
    }
    // Another site.
    let cross = page
        .client
        .post(&url)
        .header("cookie", &page.cookie)
        .header("sec-fetch-site", "cross-site")
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(cross.status(), 403);
    // Another origin posting from a page of its own.
    let origin = page
        .client
        .post(&url)
        .header("cookie", &page.cookie)
        .header("origin", "https://evil.example")
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(origin.status(), 403);
    // A rebound host name.
    let rebound = page
        .client
        .post(&url)
        .header("cookie", &page.cookie)
        .header("host", "evil.example")
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(rebound.status(), 403);
    // A form post, which a page can send to any address, is not JSON.
    let form = page
        .client
        .post(&url)
        .header("cookie", &page.cookie)
        .header("content-type", "application/x-www-form-urlencoded")
        .body(format!("provider=mock&key={GOOD}&save=true"))
        .send()
        .await
        .unwrap();
    assert_eq!(form.status(), 415);
    // A body past the limit.
    let big = page
        .client
        .post(&url)
        .header("cookie", &page.cookie)
        .header("content-type", "application/json")
        .body(format!("{{\"key\":\"{}\"}}", "x".repeat(10_000)))
        .send()
        .await
        .unwrap();
    assert_eq!(big.status(), 413);

    assert!(!key_file(&dir).exists(), "nothing was written");
    assert!(!connects(port), "the gateway was never started");
    assert_eq!(page.key_status().await, json!("none"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_key_held_for_the_run_starts_the_gateway_and_is_never_written_or_returned() {
    let dir = state("local-session");
    let (port, ui_port) = (free_port(), free_port());
    let base = provider(false).await;
    write_config(&dir, port, &base, "");
    let mut up = Up::spawn_with(&dir, &args(ui_port), &[]);
    up.wait_for("no key for mock");
    let page = Page::sign_in(&mut up).await;
    assert!(!connects(port), "no gateway without a key");
    assert_eq!(page.get("/local/connect").await.1["gateway_running"], false);

    let (status, body, text) = page
        .post(
            "/local/keys",
            &json!({"provider": "mock", "key": GOOD, "save": false}),
        )
        .await;
    assert_eq!(status, 200, "{text}");
    assert_eq!(body["ok"], true);
    assert!(!text.contains(GOOD), "the key is not echoed");
    assert!(connects(port), "the gateway started on the key");
    assert!(!key_file(&dir).exists(), "not saved unless asked");
    assert_eq!(page.key_status().await, json!("session"));
    let (_, listing) = page.get("/local/keys").await;
    assert!(!listing.to_string().contains(GOOD));
    assert!(!up.stdout().contains(GOOD) && !up.stderr().contains(GOOD));

    // The test request goes through the gateway to the provider with the key the child was given.
    let (status, result, _) = page.post("/local/test", &json!({})).await;
    assert_eq!(status, 200);
    assert_eq!(result["ok"], true, "{result}");
    assert_eq!(result["model"], "fast");
    assert!(result["latency_ms"].is_u64());
    assert!(up.terminate().success());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_key_the_provider_rejects_is_not_saved_and_the_gateway_is_not_started() {
    let dir = state("local-rejected");
    let (port, ui_port) = (free_port(), free_port());
    let base = provider(false).await;
    write_config(&dir, port, &base, "");
    let mut up = Up::spawn_with(&dir, &args(ui_port), &[]);
    let page = Page::sign_in(&mut up).await;
    let (status, body, text) = page
        .post(
            "/local/keys",
            &json!({"provider": "mock", "key": BAD, "save": true}),
        )
        .await;
    assert_eq!(status, 422, "{text}");
    assert_eq!(body["error"]["code"], "provider_rejected_key");
    assert!(!text.contains(BAD));
    assert!(!key_file(&dir).exists());
    assert!(!connects(port));
    assert_eq!(page.key_status().await, json!("none"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_saved_key_is_private_and_a_later_run_starts_from_it() {
    let dir = state("local-saved");
    let (port, ui_port) = (free_port(), free_port());
    let base = provider(false).await;
    write_config(&dir, port, &base, "");
    {
        let mut up = Up::spawn_with(&dir, &args(ui_port), &[]);
        let page = Page::sign_in(&mut up).await;
        let (status, _, text) = page
            .post(
                "/local/keys",
                &json!({"provider": "mock", "key": GOOD, "save": true}),
            )
            .await;
        assert_eq!(status, 200, "{text}");
        assert!(!text.contains(GOOD));
        assert_eq!(page.key_status().await, json!("session"));
        assert_eq!(
            std::fs::metadata(key_file(&dir))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        assert_eq!(
            std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert!(up.terminate().success());
    }
    // A second run with no key in the environment finds it in the file and starts the gateway at once.
    let mut up = Up::spawn_with(&dir, &args(ui_port), &[]);
    let page = Page::sign_in(&mut up).await;
    assert!(connects(port), "the gateway started from the saved key");
    assert_eq!(page.key_status().await, json!("file"));
    assert!(!up.stdout().contains(GOOD) && !up.stderr().contains(GOOD));
    assert!(up.terminate().success());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_environment_wins_over_a_saved_key() {
    let dir = state("local-env-wins");
    let (port, ui_port) = (free_port(), free_port());
    let base = provider(false).await;
    write_config(&dir, port, &base, "");
    let file = key_file(&dir);
    std::fs::write(&file, "MOCK_KEY=from-the-file-not-accepted\n").unwrap();
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).unwrap();
    // The file's key would be refused by the provider; the environment's is accepted.
    let mut up = Up::spawn_with(&dir, &args(ui_port), &[("MOCK_KEY", GOOD)]);
    let page = Page::sign_in(&mut up).await;
    assert_eq!(page.key_status().await, json!("environment"));
    let (_, result, _) = page.post("/local/test", &json!({})).await;
    assert_eq!(result["ok"], true, "{result}");
    assert!(up.terminate().success());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_key_is_replaced_and_removed_and_the_gateway_follows() {
    let dir = state("local-replace");
    let (port, ui_port) = (free_port(), free_port());
    let base = provider(false).await;
    write_config(&dir, port, &base, "");
    let mut up = Up::spawn_with(&dir, &args(ui_port), &[]);
    let page = Page::sign_in(&mut up).await;
    for key in [GOOD, OTHER_GOOD] {
        let (status, _, text) = page
            .post(
                "/local/keys",
                &json!({"provider": "mock", "key": key, "save": true}),
            )
            .await;
        assert_eq!(status, 200, "{text}");
        assert!(connects(port));
    }
    let saved = std::fs::read_to_string(key_file(&dir)).unwrap();
    assert!(saved.contains(OTHER_GOOD) && !saved.contains(GOOD));
    let (_, result, _) = page.post("/local/test", &json!({})).await;
    assert_eq!(result["ok"], true);

    let (status, _, text) = page.delete("/local/keys/mock").await;
    assert_eq!(status, 200, "{text}");
    assert!(
        !std::fs::read_to_string(key_file(&dir))
            .unwrap()
            .contains("MOCK_KEY")
    );
    assert!(!connects(port), "no key, no gateway");
    assert_eq!(page.key_status().await, json!("none"));
    let (_, result, _) = page.post("/local/test", &json!({})).await;
    assert_eq!(result["ok"], false);
    assert_eq!(result["step"], "gateway");
    assert!(up.terminate().success());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_gateway_that_will_not_start_on_the_new_key_leaves_everything_as_it_was() {
    let dir = state("local-rollback");
    let (port, ui_port) = (free_port(), free_port());
    let base = provider(false).await;
    // A ledger the gateway cannot open, so no start can succeed.
    write_config(
        &dir,
        port,
        &base,
        "[ledger]\npath = \"/nonexistent-directory-for-humpyard-test/ledger.db\"\n",
    );
    let mut up = Up::spawn_with(&dir, &args(ui_port), &[]);
    let page = Page::sign_in(&mut up).await;
    let (status, body, text) = page
        .post(
            "/local/keys",
            &json!({"provider": "mock", "key": GOOD, "save": true}),
        )
        .await;
    assert_eq!(status, 502, "{text}");
    assert_eq!(body["error"]["code"], "gateway_restart_failed");
    assert!(!text.contains(GOOD));
    assert!(
        !std::fs::read_to_string(key_file(&dir))
            .unwrap_or_default()
            .contains(GOOD),
        "the key was not kept"
    );
    assert_eq!(page.key_status().await, json!("none"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_provider_that_cannot_answer_is_reported_in_words() {
    let dir = state("local-test-fails");
    let (port, ui_port) = (free_port(), free_port());
    let base = provider(true).await;
    write_config(&dir, port, &base, "");
    // The key arrives by the environment, so the provider's failing chat does not stop the start.
    let mut up = Up::spawn_with(&dir, &args(ui_port), &[("MOCK_KEY", GOOD)]);
    let page = Page::sign_in(&mut up).await;
    let (status, result, text) = page.post("/local/test", &json!({})).await;
    assert_eq!(status, 200, "{text}");
    assert_eq!(result["ok"], false);
    assert_eq!(result["step"], "provider");
    assert!(!text.contains(GOOD));
    assert!(up.terminate().success());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_connect_settings_name_the_live_address_and_the_routes_and_no_secret() {
    let dir = state("local-connect");
    let (port, ui_port) = (free_port(), free_port());
    let base = provider(false).await;
    write_config(&dir, port, &base, "");
    let mut up = Up::spawn_with(&dir, &args(ui_port), &[("MOCK_KEY", GOOD)]);
    let page = Page::sign_in(&mut up).await;
    let (status, body) = page.get("/local/connect").await;
    assert_eq!(status, 200);
    assert_eq!(body["gateway"], format!("http://127.0.0.1:{port}"));
    assert_eq!(body["routes"], json!(["fast"]));
    assert_eq!(body["key_entry"], true);
    assert!(!body.to_string().contains(GOOD));
    assert!(up.terminate().success());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn key_entry_is_off_when_the_page_is_served_beyond_this_machine() {
    let dir = state("local-remote");
    let (port, ui_port) = (free_port(), free_port());
    let base = provider(false).await;
    write_config(&dir, port, &base, "");
    let mut up = Up::spawn_with(
        &dir,
        &[
            "--ui-listen".into(),
            format!("0.0.0.0:{ui_port}"),
            "--allow-host".into(),
            format!("localhost:{ui_port}"),
        ],
        &[],
    );
    let (_, token) = up.gui();
    let gui = format!("http://127.0.0.1:{ui_port}");
    let client = reqwest::Client::new();
    let session = client
        .post(format!("{gui}/ui/session"))
        .header("host", format!("localhost:{ui_port}"))
        .json(&json!({ "token": token }))
        .send()
        .await
        .unwrap();
    assert_eq!(session.status(), 204);
    let cookie = session.headers()["set-cookie"]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_string();
    let host = format!("localhost:{ui_port}");
    let keys = client
        .get(format!("{gui}/local/keys"))
        .header("cookie", &cookie)
        .header("host", &host)
        .send()
        .await
        .unwrap();
    assert_eq!(keys.status(), 200);
    assert_eq!(keys.json::<Value>().await.unwrap()["key_entry"], false);
    let attempt = client
        .post(format!("{gui}/local/keys"))
        .header("cookie", &cookie)
        .header("host", &host)
        .json(&json!({"provider": "mock", "key": GOOD, "save": true}))
        .send()
        .await
        .unwrap();
    assert_eq!(attempt.status(), 403);
    assert_eq!(
        attempt.json::<Value>().await.unwrap()["error"]["code"],
        "key_entry_disabled"
    );
    assert!(!key_file(&dir).exists());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_key_can_be_set_only_for_a_provider_the_configuration_names() {
    let dir = state("local-unknown-provider");
    let (port, ui_port) = (free_port(), free_port());
    let base = provider(false).await;
    write_config(&dir, port, &base, "");
    let mut up = Up::spawn_with(&dir, &args(ui_port), &[]);
    let page = Page::sign_in(&mut up).await;
    let (status, body, text) = page
        .post(
            "/local/keys",
            &json!({"provider": "somewhere-else", "key": GOOD, "save": true}),
        )
        .await;
    assert_eq!(status, 400, "{text}");
    assert_eq!(body["error"]["code"], "unknown_provider");
    let (status, _, _) = page.delete("/local/keys/somewhere-else").await;
    assert_eq!(status, 400);
    // A variable name cannot be smuggled in as a provider, and a field the form does not have is refused.
    let (status, _, _) = page
        .post(
            "/local/keys",
            &json!({"provider": "mock", "key": GOOD, "variable": "PATH"}),
        )
        .await;
    assert_eq!(status, 400);
    assert!(!key_file(&dir).exists());
    assert!(!connects(port));
}
