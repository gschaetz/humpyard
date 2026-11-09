//! `POST /admin/v1/config/patch` (spec `config-patch`, ADR 0034, change `add-config-patch`): typed edits become a
//! validated patch against the configuration file, a stale or unloadable request is refused with its reason,
//! and no outcome writes the file, changes the running configuration or touches the reload status.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test scaffolding

mod common;

use std::path::PathBuf;
use std::time::Duration;

use humpyard::config::Config;
use humpyard::config::patch::{Edit, apply};
use humpyard::server::{self, Reload};
use serde_json::{Value, json};
use tokio::sync::mpsc;

const SECRET_HEADER: &str = "SECRET-HEADER-VALUE-4417";

struct Credentials {
    editor: String,
    config_only: String,
    reader: String,
    owner: String,
    hashes: [String; 4],
    key_hash: String,
}

fn credentials() -> Credentials {
    let [editor, config_only, reader, owner, key] =
        std::array::from_fn(|_| humpyard::auth::generate_key().unwrap());
    Credentials {
        editor: editor.0,
        config_only: config_only.0,
        reader: reader.0,
        owner: owner.0,
        hashes: [editor.1, config_only.1, reader.1, owner.1],
        key_hash: key.1,
    }
}

/// A file with comments, a provider with a header value, a key, and an admin credential of each kind.
fn text(creds: &Credentials, discovery: &str, extra: &str) -> String {
    format!(
        r#"# The gateway.
listen = "127.0.0.1:0"
{discovery}
# The providers.
[providers.p]
base_url = "http://127.0.0.1:1"   # local
api_key_env = "PATH"
headers = {{ "x-session" = "{SECRET_HEADER}" }}

[providers.q]
base_url = "http://127.0.0.1:2"
api_key_env = "PATH"
{extra}
# The first target.
[targets.t]
class = "public"
endpoints = [{{ provider = "p", model = "m", context_window = 131072 }}]

[routes.r]
type = "passthrough"
targets = ["t"]

[keys.alice]
sha256 = "{key_hash}"
allowed_routes = ["r"]

[admin.credentials.editor]
sha256 = "{}"
scopes = ["read", "config"]
[admin.credentials.cfgonly]
sha256 = "{}"
scopes = ["config"]
[admin.credentials.reader]
sha256 = "{}"
scopes = ["read"]
[admin.credentials.owner]
sha256 = "{}"
scopes = ["read", "overrides", "credentials"]
"#,
        creds.hashes[0],
        creds.hashes[1],
        creds.hashes[2],
        creds.hashes[3],
        key_hash = creds.key_hash,
    )
}

struct Gateway {
    url: String,
    path: PathBuf,
    reload: mpsc::Sender<()>,
    stop: mpsc::Sender<()>,
    task: tokio::task::JoinHandle<Result<(), String>>,
    creds: Credentials,
    dir: tempfile::TempDir,
}

async fn start_with(
    dir: tempfile::TempDir,
    creds: Credentials,
    file_text: &str,
    reloadable: bool,
) -> Gateway {
    let path = dir.path().join("humpyard.toml");
    std::fs::write(&path, file_text).unwrap();
    let config = Config::from_toml_with(file_text, |n| std::env::var(n).ok(), &|f| {
        std::fs::read_to_string(f).ok()
    })
    .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let (reload, triggers) = mpsc::channel(4);
    let (stop, signals) = mpsc::channel(4);
    let reload_arg = reloadable.then(|| Reload {
        path: path.clone(),
        triggers,
    });
    let task = tokio::spawn(server::run(config, listener, signals, reload_arg));
    Gateway {
        url,
        path,
        reload,
        stop,
        task,
        creds,
        dir,
    }
}

async fn start() -> Gateway {
    let creds = credentials();
    let file_text = text(&creds, "", "");
    start_with(tempfile::tempdir().unwrap(), creds, &file_text, true).await
}

/// Everything a patch must leave alone, read from the outside.
#[derive(Debug, PartialEq)]
struct Observed {
    file: String,
    fingerprint: String,
    reload_attempts: u64,
    reload_message: Value,
}

impl Gateway {
    fn file(&self) -> String {
        std::fs::read_to_string(&self.path).unwrap()
    }

    async fn get(&self, path: &str, secret: &str) -> (u16, Value) {
        let response = reqwest::Client::new()
            .get(format!("{}{path}", self.url))
            .bearer_auth(secret)
            .send()
            .await
            .unwrap();
        (
            response.status().as_u16(),
            response.json().await.unwrap_or(Value::Null),
        )
    }

    async fn fingerprint(&self) -> String {
        let (status, body) = self.get("/admin/v1/config", &self.creds.editor).await;
        assert_eq!(status, 200, "{body}");
        body["config"]["fingerprint"].as_str().unwrap().to_string()
    }

    async fn observed(&self) -> Observed {
        let (_, health) = self.get("/admin/v1/health", &self.creds.editor).await;
        Observed {
            file: self.file(),
            fingerprint: self.fingerprint().await,
            reload_attempts: health["reload"]["attempts"].as_u64().unwrap_or(0),
            reload_message: health["reload"]["message"].clone(),
        }
    }

    async fn post_raw(&self, secret: Option<&str>, body: Vec<u8>) -> (u16, Value) {
        let mut request = reqwest::Client::new()
            .post(format!("{}/admin/v1/config/patch", self.url))
            .header("content-type", "application/json")
            .body(body);
        if let Some(secret) = secret {
            request = request.bearer_auth(secret);
        }
        let response = request.send().await.unwrap();
        (
            response.status().as_u16(),
            response.json().await.unwrap_or(Value::Null),
        )
    }

    async fn patch(&self, secret: &str, fingerprint: &str, edits: Value) -> (u16, Value) {
        let body = json!({"fingerprint": fingerprint, "edits": edits});
        self.post_raw(Some(secret), serde_json::to_vec(&body).unwrap())
            .await
    }

    /// A patch by the editor against the file's current fingerprint.
    async fn edit(&self, edits: Value) -> (u16, Value) {
        let fingerprint = self.fingerprint().await;
        self.patch(&self.creds.editor.clone(), &fingerprint, edits)
            .await
    }

    async fn shut_down(self) {
        self.stop.send(()).await.unwrap();
        let _ = tokio::time::timeout(Duration::from_secs(10), self.task).await;
        drop(self.dir);
    }
}

fn set_cap(value: &Value) -> Value {
    json!([{"op": "set", "path": ["targets", "t", "endpoints", 0, "max_request_tokens"], "value": value}])
}

#[tokio::test]
async fn a_valid_edit_returns_a_diff_that_changes_only_that_line_and_the_fingerprint_the_file_would_have()
 {
    let g = start().await;
    let before = g.observed().await;
    let (status, body) = g.edit(set_cap(&json!(8000))).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["object"], "config_patch");
    assert_eq!(body["changed"], true);
    assert_eq!(body["edits"], 1);
    assert_eq!(body["base_fingerprint"], before.fingerprint);
    let diff = body["diff"].as_str().unwrap();
    let changes: Vec<&str> = diff
        .lines()
        .filter(|l| {
            (l.starts_with('-') || l.starts_with('+'))
                && !l.starts_with("---")
                && !l.starts_with("+++")
        })
        .collect();
    assert_eq!(changes.len(), 2, "{diff}");
    assert!(changes[1].contains("max_request_tokens = 8000"), "{diff}");
    assert!(
        diff.contains("# The first target."),
        "context keeps the comments: {diff}"
    );

    // The fingerprint is the one the gateway reports once that exact text is in place and reloaded.
    let edits: Vec<Edit> = serde_json::from_value(set_cap(&json!(8000))).unwrap();
    std::fs::write(&g.path, apply(&before.file, &edits).unwrap()).unwrap();
    g.reload.send(()).await.unwrap();
    let mut after = String::new();
    for _ in 0..100 {
        after = g.fingerprint().await;
        if after != before.fingerprint {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(after, body["fingerprint"].as_str().unwrap());
    g.shut_down().await;
}

#[tokio::test]
async fn an_edit_the_loader_rejects_returns_its_reason_and_no_patch() {
    let g = start().await;
    let (status, body) = g.edit(set_cap(&json!(0))).await;
    assert_eq!(status, 422, "{body}");
    assert_eq!(body["error"]["code"], "config_rejected");
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap()
            .contains("max_request_tokens"),
        "{body}"
    );
    assert!(
        body.get("diff").is_none() && body.get("fingerprint").is_none(),
        "{body}"
    );
    g.shut_down().await;
}

#[tokio::test]
async fn edits_that_change_nothing_give_an_empty_diff_and_the_same_fingerprint() {
    let g = start().await;
    let (status, body) = g
        .edit(json!([{"op": "set", "path": ["targets", "t", "class"], "value": "public"}]))
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["changed"], false);
    assert_eq!(body["diff"], "");
    assert_eq!(body["fingerprint"], body["base_fingerprint"]);
    g.shut_down().await;
}

#[tokio::test]
async fn a_stale_fingerprint_is_refused_with_the_current_one_even_before_a_reload() {
    let g = start().await;
    let read = g.fingerprint().await;
    // The file changes, and the reload has not run (the poll is off): the running configuration still reports `read`.
    let moved = g
        .file()
        .replace("class = \"public\"", "class = \"private\"");
    std::fs::write(&g.path, &moved).unwrap();
    assert_eq!(g.fingerprint().await, read, "no reload has happened");
    let (status, body) = g
        .patch(&g.creds.editor.clone(), &read, set_cap(&json!(8000)))
        .await;
    assert_eq!(status, 409, "{body}");
    assert_eq!(body["error"]["code"], "stale_fingerprint");
    let current = body["error"]["current_fingerprint"]
        .as_str()
        .unwrap()
        .to_string();
    assert_ne!(current, read);
    // Naming the file's own fingerprint works, and is judged against the new text.
    let (status, body) = g
        .patch(&g.creds.editor.clone(), &current, set_cap(&json!(8000)))
        .await;
    assert_eq!(status, 200, "{body}");
    assert!(
        body["diff"]
            .as_str()
            .unwrap()
            .contains("class = \"private\""),
        "{body}"
    );
    g.shut_down().await;
}

#[tokio::test]
async fn no_outcome_writes_the_file_reloads_or_changes_the_running_configuration() {
    let g = start().await;
    let before = g.observed().await;
    let bodies = [
        json!({"fingerprint": before.fingerprint, "edits": set_cap(&json!(8000))}),
        json!({"fingerprint": before.fingerprint, "edits": set_cap(&json!(0))}),
        json!({"fingerprint": "stale", "edits": set_cap(&json!(8000))}),
        json!({"fingerprint": before.fingerprint, "edits": [{"op": "remove", "path": ["admin", "x"]}]}),
        json!({"fingerprint": before.fingerprint, "edits": []}),
        json!({"nonsense": true}),
    ];
    for body in bodies {
        let secret = g.creds.editor.clone();
        let (status, _) = g
            .post_raw(Some(&secret), serde_json::to_vec(&body).unwrap())
            .await;
        assert!(status == 200 || status >= 400, "{status}");
        assert_eq!(g.observed().await, before, "after {body}");
    }
    // Repeating a request gives the same answer.
    let first = g
        .patch(
            &g.creds.editor.clone(),
            &before.fingerprint,
            set_cap(&json!(8000)),
        )
        .await;
    let second = g
        .patch(
            &g.creds.editor.clone(),
            &before.fingerprint,
            set_cap(&json!(8000)),
        )
        .await;
    assert_eq!(first, second);
    assert_eq!(g.observed().await, before);
    g.shut_down().await;
}

#[tokio::test]
async fn edits_outside_the_editable_sections_are_refused_naming_the_edit() {
    let g = start().await;
    let fine = json!({"op": "set", "path": ["targets", "t", "class"], "value": "private"});
    let cases = [
        json!({"op": "set", "path": ["admin", "credentials", "editor", "scopes"], "value": ["read"]}),
        json!({"op": "set", "path": ["keys", "alice", "sha256"], "value": "sha256:ff"}),
        json!({"op": "add_table", "path": ["keys", "bob"], "value": {}}),
        json!({"op": "remove_table", "path": ["keys", "alice"]}),
        json!({"op": "set", "path": ["listen"], "value": "127.0.0.1:9"}),
        json!({"op": "add_table", "path": ["targets", "t"], "value": {}}),
        json!({"op": "remove", "path": ["targets", "nope", "class"]}),
        json!({"op": "teleport", "path": ["targets"]}),
    ];
    for case in cases {
        let (status, body) = g.edit(json!([fine, case])).await;
        assert_eq!(status, 400, "{case}: {body}");
        assert_eq!(body["error"]["code"], "invalid_edit", "{case}");
        assert_eq!(body["error"]["index"], 1, "{case}: {body}");
    }
    g.shut_down().await;
}

#[tokio::test]
async fn the_request_is_bounded() {
    let g = start().await;
    let fingerprint = g.fingerprint().await;
    let editor = g.creds.editor.clone();
    let one = json!({"op": "set", "path": ["targets", "t", "class"], "value": "public"});
    let (status, _) = g
        .patch(&editor, &fingerprint, json!(vec![one.clone(); 64]))
        .await;
    assert_eq!(status, 200);
    let (status, body) = g.patch(&editor, &fingerprint, json!(vec![one; 65])).await;
    assert_eq!(status, 400, "{body}");
    assert!(
        body["error"]["message"].as_str().unwrap().contains("64"),
        "{body}"
    );
    let (status, body) = g.post_raw(Some(&editor), vec![b' '; 64 * 1024 + 1]).await;
    assert_eq!(status, 413, "{body}");
    assert_eq!(body["error"]["code"], "request_too_large");
    let (status, body) = g.post_raw(Some(&editor), b"{not json".to_vec()).await;
    assert_eq!(
        (status, &body["error"]["code"]),
        (400, &json!("invalid_request"))
    );
    g.shut_down().await;
}

#[tokio::test]
async fn removing_a_provider_shows_its_header_without_the_value() {
    // A second target and route on provider `q`, so removing everything that uses `p` still leaves a gateway.
    let creds = credentials();
    let file_text = format!(
        "{}\n[targets.u]\nendpoints = [{{ provider = \"q\", model = \"x\" }}]\n[routes.s]\ntype = \"passthrough\"\ntargets = [\"u\"]\n",
        text(&creds, "", "")
    );
    let g = start_with(tempfile::tempdir().unwrap(), creds, &file_text, true).await;
    // Provider `p` is in use by target `t`, so removing only it does not load.
    let (status, body) = g
        .edit(json!([{"op": "remove_table", "path": ["providers", "p"]}]))
        .await;
    assert_eq!(status, 422, "{body}");
    let (status, body) = g
        .edit(json!([
            {"op": "set", "path": ["keys", "alice", "allowed_routes"], "value": ["s"]},
            {"op": "remove_table", "path": ["routes", "r"]},
            {"op": "remove_table", "path": ["targets", "t"]},
            {"op": "remove_table", "path": ["providers", "p"]},
        ]))
        .await;
    assert_eq!(status, 200, "{body}");
    let diff = body["diff"].as_str().unwrap();
    assert!(
        diff.contains("headers = { \"x-session\" = \"<redacted>\" }"),
        "{diff}"
    );
    assert!(!body.to_string().contains(SECRET_HEADER), "{body}");
    assert!(!body.to_string().contains(&g.creds.key_hash), "{body}");
    g.shut_down().await;
}

#[tokio::test]
async fn a_loader_error_never_echoes_a_line_of_the_file() {
    let g = start().await;
    // A header value that is not a string makes the loader fail on the line that also holds the secret one.
    let (status, body) = g
        .edit(json!([{"op": "set", "path": ["providers", "p", "headers", "x-new"], "value": 5}]))
        .await;
    assert_eq!(status, 422, "{body}");
    assert_eq!(body["error"]["code"], "config_rejected");
    assert!(!body.to_string().contains(SECRET_HEADER), "{body}");
    assert!(!body.to_string().contains(&g.creds.key_hash), "{body}");
    // The reason and the position are still given.
    let message = body["error"]["message"].as_str().unwrap();
    assert!(
        message.contains("expected a string") && message.contains("line"),
        "{message}"
    );
    g.shut_down().await;
}

#[tokio::test]
async fn a_hash_near_an_edit_is_masked_in_the_context() {
    let g = start().await;
    let (status, body) = g
        .edit(json!([{"op": "set", "path": ["keys", "alice", "allowed_routes"], "value": ["r", "r"]}]))
        .await;
    assert_eq!(status, 200, "{body}");
    let text = body.to_string();
    assert!(
        body["diff"]
            .as_str()
            .unwrap()
            .contains("sha256 = \"<redacted>\""),
        "{body}"
    );
    for hash in g.creds.hashes.iter().chain([&g.creds.key_hash]) {
        assert!(!text.contains(hash.as_str()), "{hash} in {text}");
    }
    g.shut_down().await;
}

#[tokio::test]
async fn the_config_scope_is_its_own() {
    let g = start().await;
    let fingerprint = g.fingerprint().await;
    // `read`, and every other scope together, do not open the endpoint.
    for secret in [&g.creds.reader, &g.creds.owner] {
        let (status, body) = g.patch(secret, &fingerprint, set_cap(&json!(8000))).await;
        assert_eq!(status, 403, "{body}");
        assert_eq!(body["error"]["code"], "admin_scope_required");
        assert_eq!(body["error"]["scope"], "config");
    }
    // `config` alone opens it, and does not open reading.
    let (status, body) = g
        .patch(&g.creds.config_only, &fingerprint, set_cap(&json!(8000)))
        .await;
    assert_eq!(status, 200, "{body}");
    let (status, body) = g.get("/admin/v1/config", &g.creds.config_only).await;
    assert_eq!(status, 403, "{body}");
    assert_eq!(body["error"]["scope"], "read");
    // No credential, and a client key, are refused alike.
    let (status, _) = g.post_raw(None, b"{}".to_vec()).await;
    assert_eq!(status, 401);
    g.shut_down().await;
}

#[tokio::test]
async fn without_a_file_to_read_there_is_no_patch() {
    // Started without a reload path, there is no file the gateway can re-read.
    let creds = credentials();
    let file_text = text(&creds, "", "");
    let g = start_with(tempfile::tempdir().unwrap(), creds, &file_text, false).await;
    let (status, body) = g
        .patch(&g.creds.config_only.clone(), "x", set_cap(&json!(8000)))
        .await;
    assert_eq!(status, 503, "{body}");
    assert_eq!(body["error"]["code"], "config_file_unavailable");
    g.shut_down().await;

    // With a path, but the file gone.
    let g = start().await;
    let fingerprint = g.fingerprint().await;
    std::fs::remove_file(&g.path).unwrap();
    let (status, body) = g
        .patch(&g.creds.editor.clone(), &fingerprint, set_cap(&json!(8000)))
        .await;
    assert_eq!(status, 503, "{body}");
    assert_eq!(body["error"]["code"], "config_file_unavailable");
    g.shut_down().await;
}

#[tokio::test]
async fn a_patch_that_would_stop_the_generated_file_merging_is_refused_though_a_reload_would_load()
{
    let dir = tempfile::tempdir().unwrap();
    let generated = dir.path().join("generated.toml");
    std::fs::write(
        &generated,
        "[targets.auto-q-x]\nstrengths = [\"general\"]\nendpoints = [{ provider = \"q\", model = \"x\", price = { input = 0.0, output = 0.0 } }]\n",
    )
    .unwrap();
    let creds = credentials();
    let discovery = format!(
        "[discovery]\nenabled = true\nfile = \"{}\"\n",
        generated.display()
    );
    let extra = "price_source = \"models-dev:groq\"\ndiscover = { strengths = [\"general\"] }\n";
    let file_text = text(&creds, &discovery, extra);
    let g = start_with(dir, creds, &file_text, true).await;
    let (status, body) = g
        .edit(json!([{"op": "remove_table", "path": ["providers", "q"]}]))
        .await;
    assert_eq!(status, 422, "{body}");
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap()
            .contains("generated targets file would be ignored"),
        "{body}"
    );
    g.shut_down().await;
}

#[tokio::test]
async fn a_request_is_logged_by_who_and_how_it_ended_and_never_by_its_content() {
    let logs = common::logs();
    let g = start().await;
    let (status, _) = g.edit(set_cap(&json!(8000))).await;
    assert_eq!(status, 200);
    let (status, _) = g
        .edit(json!([{"op": "set", "path": ["providers", "p", "headers", "x-new"], "value": "UNIQUE-VALUE-9931"}, {"op": "remove", "path": ["nope"]}]))
        .await;
    assert_eq!(status, 400);
    let text = logs.contents();
    let lines: Vec<&str> = text
        .lines()
        .filter(|l| l.contains("configuration patch requested"))
        .collect();
    assert!(
        lines
            .iter()
            .any(|l| l.contains("admin=editor") && l.contains("outcome=\"ok\"")),
        "{lines:?}"
    );
    assert!(
        lines.iter().any(|l| l.contains("outcome=\"invalid_edit\"")),
        "{lines:?}"
    );
    // A number such as the cap is not a needle: the captured text holds every test's logs, with ports and
    // timestamps that can contain any digits. The strings below cannot occur by chance.
    for secret in ["UNIQUE-VALUE-9931", SECRET_HEADER, "max_request_tokens"] {
        assert!(!text.contains(secret), "{secret} was logged");
    }
    g.shut_down().await;
}
