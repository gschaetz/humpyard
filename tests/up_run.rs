#![cfg(unix)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test scaffolding: fail loudly
//! `humpyard up` as a real process (spec: `single-machine-up`): the state and run directories, the per-run admin
//! credential, the gateway child and its lifetime, and what a taken address or an `[admin]` section does.

mod common;
#[path = "common/up_harness.rs"]
mod harness;

use std::os::unix::fs::PermissionsExt;
use std::process::Command;
use std::time::Duration;

use axum::Router;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use common::{chat_completion, serve, usage};
use harness::{Up, connects, free_port, run_configs, state, write_config};

#[tokio::test]
async fn one_command_starts_the_gateway_and_the_gui_and_stopping_it_stops_the_gateway() {
    let dir = state("both");
    let (port, ui_port) = (free_port(), free_port());
    write_config(&dir, port, "");
    let mut up = Up::spawn(
        &dir,
        &["--ui-listen".into(), format!("127.0.0.1:{ui_port}")],
        &[],
    );
    let (gui, token) = up.gui();
    assert_eq!(gui, format!("http://127.0.0.1:{ui_port}"));
    assert!(connects(port), "the gateway is listening");

    let client = reqwest::Client::new();
    // The page is served by `up`, and the gateway's own address serves none.
    let page = client.get(format!("{gui}/")).send().await.unwrap();
    assert!(page.status().is_success());
    assert!(
        page.headers()["content-type"]
            .to_str()
            .unwrap()
            .starts_with("text/html")
    );
    let gateway_root = client
        .get(format!("http://127.0.0.1:{port}/"))
        .send()
        .await
        .unwrap();
    assert!(
        !gateway_root
            .headers()
            .get("content-type")
            .is_some_and(|v| v.to_str().unwrap().starts_with("text/html")),
        "the gateway serves no page"
    );

    // The signed-in link works with no credential to paste: the token opens a session, and the admin API answers
    // through the GUI with the credential `up` holds.
    let before = client
        .get(format!("{gui}/admin/v1/health"))
        .send()
        .await
        .unwrap();
    assert_eq!(before.status(), 401);
    let session = client
        .post(format!("{gui}/ui/session"))
        .json(&serde_json::json!({ "token": token }))
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
    let health = client
        .get(format!("{gui}/admin/v1/health"))
        .header("cookie", cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(health.status(), 200);

    let status = up.terminate();
    assert!(status.success(), "{status}: {}", up.stderr());
    assert!(!connects(port), "the gateway is stopped with up");
}

#[test]
fn the_run_directory_holds_a_hash_in_a_private_file_and_is_gone_afterwards() {
    let dir = state("run-dir");
    let (port, ui_port) = (free_port(), free_port());
    write_config(&dir, port, "");
    // What an earlier run left after a crash.
    std::fs::create_dir_all(dir.join("run/424242")).unwrap();
    std::fs::write(dir.join("run/424242/config.toml"), "stale").unwrap();

    let mut up = Up::spawn(
        &dir,
        &["--ui-listen".into(), format!("127.0.0.1:{ui_port}")],
        &[],
    );
    let (_, token) = up.gui();
    let configs = run_configs(&dir);
    assert_eq!(configs.len(), 1, "the stale directory is gone: {configs:?}");
    let run_dir = configs[0].parent().unwrap();
    assert_eq!(
        std::fs::metadata(run_dir).unwrap().permissions().mode() & 0o777,
        0o700
    );
    assert_eq!(
        std::fs::metadata(&configs[0]).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let text = std::fs::read_to_string(&configs[0]).unwrap();
    assert!(text.contains("[admin.credentials.up]") && text.contains("sha256:"));
    assert!(!text.contains("sk-humpyard-"), "no credential in the file");
    assert!(!text.contains(&token), "the token is not in the file");

    let status = up.terminate();
    assert!(status.success());
    assert!(!dir.join("run").join(run_dir.file_name().unwrap()).exists());
    // Neither stream carried a credential.
    for text in [up.stdout(), up.stderr()] {
        assert!(!text.contains("sk-humpyard-"), "{text}");
    }
}

#[test]
fn each_run_makes_its_own_credential() {
    let dir = state("fresh");
    let (port, ui_port) = (free_port(), free_port());
    write_config(&dir, port, "");
    let mut hashes = Vec::new();
    let mut tokens = Vec::new();
    for _ in 0..2 {
        let mut up = Up::spawn(
            &dir,
            &["--ui-listen".into(), format!("127.0.0.1:{ui_port}")],
            &[],
        );
        let (_, token) = up.gui();
        let text = std::fs::read_to_string(&run_configs(&dir)[0]).unwrap();
        let hash = text
            .lines()
            .find(|l| l.starts_with("sha256"))
            .unwrap()
            .to_string();
        hashes.push(hash);
        tokens.push(token);
        assert!(up.terminate().success());
    }
    assert_ne!(hashes[0], hashes[1]);
    assert_ne!(tokens[0], tokens[1]);
}

#[test]
fn a_configuration_with_an_admin_section_is_refused_by_name() {
    let dir = state("admin-section");
    let (port, ui_port) = (free_port(), free_port());
    write_config(
        &dir,
        port,
        &format!(
            "[admin.credentials.mine]\nsha256 = \"sha256:{}\"\nscopes = [\"read\"]\n",
            "0".repeat(64)
        ),
    );
    let mut up = Up::spawn(
        &dir,
        &["--ui-listen".into(), format!("127.0.0.1:{ui_port}")],
        &[],
    );
    let status = up.wait_exit();
    assert!(!status.success());
    std::thread::sleep(Duration::from_millis(200));
    assert!(up.stderr().contains("[admin]"), "{}", up.stderr());
    assert!(!connects(port), "nothing was started");
}

#[test]
fn a_taken_gui_address_starts_nothing_and_names_the_flag() {
    let dir = state("taken-ui");
    let port = free_port();
    write_config(&dir, port, "");
    let taken = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let ui = taken.local_addr().unwrap();
    let mut up = Up::spawn(&dir, &["--ui-listen".into(), ui.to_string()], &[]);
    assert!(!up.wait_exit().success());
    std::thread::sleep(Duration::from_millis(200));
    assert!(up.stderr().contains("--ui-listen"), "{}", up.stderr());
    assert!(!connects(port), "the gateway was not started");
}

#[test]
fn a_taken_gateway_address_starts_nothing_and_names_the_flag() {
    let dir = state("taken-gateway");
    let ui_port = free_port();
    let taken = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = taken.local_addr().unwrap().port();
    write_config(&dir, port, "");
    let mut up = Up::spawn(
        &dir,
        &["--ui-listen".into(), format!("127.0.0.1:{ui_port}")],
        &[],
    );
    assert!(!up.wait_exit().success());
    std::thread::sleep(Duration::from_millis(200));
    assert!(up.stderr().contains("--listen"), "{}", up.stderr());
    assert!(!connects(ui_port), "the GUI was not started");
}

#[test]
fn a_gateway_that_dies_is_reported_and_ends_up() {
    let dir = state("child-dies");
    let (port, ui_port) = (free_port(), free_port());
    write_config(&dir, port, "");
    let mut up = Up::spawn(
        &dir,
        &["--ui-listen".into(), format!("127.0.0.1:{ui_port}")],
        &[],
    );
    up.gui();
    // The only child of `up` is the gateway.
    let killed = Command::new("pkill")
        .args(["-KILL", "-P", &up.child.id().to_string()])
        .status()
        .unwrap();
    assert!(killed.success());
    let status = up.wait_exit();
    assert!(!status.success());
    std::thread::sleep(Duration::from_millis(200));
    assert!(
        up.stderr().contains("the gateway stopped"),
        "{}",
        up.stderr()
    );
}

#[test]
fn the_gateway_child_gets_no_key_on_its_command_line() {
    let dir = state("argv");
    let (port, ui_port) = (free_port(), free_port());
    write_config(&dir, port, "");
    let keys = dir.clone();
    std::fs::write(
        keys.join("keys.env"),
        "HUMPYARD_TEST_KEY=argv-secret-value\n",
    )
    .unwrap();
    std::fs::set_permissions(
        keys.join("keys.env"),
        std::fs::Permissions::from_mode(0o600),
    )
    .unwrap();
    let mut up = Up::spawn(
        &dir,
        &["--ui-listen".into(), format!("127.0.0.1:{ui_port}")],
        &[],
    );
    up.gui();
    let listing = Command::new("ps")
        .args(["-ax", "-o", "args="])
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&listing.stdout);
    assert!(text.contains("serve --config"), "the child is running");
    assert!(
        !text.contains("argv-secret-value"),
        "no key on a command line"
    );
}

/// A provider that lists `m` and answers chat for `m`.
async fn provider() -> String {
    let app = Router::new()
        .route(
            "/models",
            get(|| async { axum::Json(serde_json::json!({"data": [{"id": "m"}]})) }),
        )
        .route(
            "/chat/completions",
            post(|| async {
                axum::Json(chat_completion("m", "hello there", Some(usage(5, 3)))).into_response()
            }),
        );
    format!("http://{}", serve(app).await)
}

// Multi-threaded: the test waits on the process with blocking calls, and the mock provider must still answer it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_first_run_with_a_key_writes_a_configuration_and_starts_the_gateway() {
    let dir = state("first-run");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
    let base = provider().await;
    let (port, ui_port) = (free_port(), free_port());
    let mut up = Up::spawn(
        &dir,
        &[
            "--listen".into(),
            format!("127.0.0.1:{port}"),
            "--ui-listen".into(),
            format!("127.0.0.1:{ui_port}"),
            "--custom".into(),
            format!("mock,{base},MOCK_KEY,m"),
        ],
        &[("MOCK_KEY", "k")],
    );
    up.gui();
    let written = std::fs::read_to_string(dir.join("config.toml")).unwrap();
    assert!(written.contains("[providers.mock]"), "{written}");
    assert!(!written.contains("sk-humpyard-") && !written.contains("\"k\""));
    assert!(
        connects(port),
        "the gateway started on the new configuration"
    );
    assert!(up.terminate().success());
}

#[test]
fn a_first_run_with_no_key_serves_the_gui_and_waits() {
    let dir = state("no-key");
    let (port, ui_port) = (free_port(), free_port());
    let mut up = Up::spawn(
        &dir,
        &[
            "--listen".into(),
            format!("127.0.0.1:{port}"),
            "--ui-listen".into(),
            format!("127.0.0.1:{ui_port}"),
        ],
        &[],
    );
    up.gui();
    assert!(
        up.stdout().contains("no provider key found"),
        "{}",
        up.stdout()
    );
    assert!(!dir.join("config.toml").exists());
    assert!(!connects(port), "no gateway without a key");
    assert!(up.terminate().success());
}

#[tokio::test]
async fn an_existing_configuration_is_used_unchanged() {
    let dir = state("kept");
    let (port, ui_port) = (free_port(), free_port());
    write_config(&dir, port, "");
    let before = std::fs::read_to_string(dir.join("config.toml")).unwrap();
    let mut up = Up::spawn(
        &dir,
        &["--ui-listen".into(), format!("127.0.0.1:{ui_port}")],
        &[],
    );
    up.gui();
    assert_eq!(
        std::fs::read_to_string(dir.join("config.toml")).unwrap(),
        before
    );
    assert!(up.terminate().success());
}

#[test]
fn a_state_directory_others_can_use_is_refused_and_says_how_to_fix_it() {
    let dir = state("loose-state");
    let (port, ui_port) = (free_port(), free_port());
    write_config(&dir, port, "");
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
    let mut up = Up::spawn(
        &dir,
        &["--ui-listen".into(), format!("127.0.0.1:{ui_port}")],
        &[],
    );
    assert!(!up.wait_exit().success());
    std::thread::sleep(Duration::from_millis(200));
    assert!(up.stderr().contains("chmod 700"), "{}", up.stderr());
    assert!(!connects(port) && !connects(ui_port), "nothing was started");
}
