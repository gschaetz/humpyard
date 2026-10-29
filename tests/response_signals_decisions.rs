//! A decision shows the response signals set on the call that served it, on `GET /v1/decisions`,
//! `GET /admin/v1/decisions` and `humpyard ctl decisions` (spec: `decision-log`, change
//! `record-response-quality-signals`, ADR 0030): a flagged answer, a clean one, a stream still open and a decision
//! whose usage row has unknown signals (as one from before schema v13 does).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test scaffolding

mod common;

use std::convert::Infallible;
use std::time::Duration;

use axum::Router;
use axum::extract::Json;
use axum::response::sse::{Event, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use common::{chunk, serve, usage};
use humpyard::config::Config;
use humpyard::ctl::{Command, Settings, connect, run};
use humpyard::ledger::Ledger;
use humpyard::server;
use serde_json::{Value, json};
use sqlx::Connection;

/// What the user message asks the fake upstream to do: `bad` answers with a cut-off tool call, `slow` streams one
/// chunk and then waits, anything else answers "hello".
async fn completions(Json(body): Json<Value>) -> Response {
    let ask = body["messages"][0]["content"].as_str().unwrap_or("");
    if ask == "slow" {
        let events = async_stream::stream! {
            yield Ok::<_, Infallible>(Event::default().data(
                chunk(json!({"role": "assistant", "content": "Hel"}), None).to_string()));
            tokio::time::sleep(Duration::from_secs(30)).await;
        };
        return Sse::new(events).into_response();
    }
    let message = if ask == "bad" {
        json!({"role": "assistant", "content": null, "tool_calls": [{"id": "call_1", "type": "function",
               "function": {"name": "get_weather", "arguments": "{\"city\":\"Par"}}]})
    } else {
        json!({"role": "assistant", "content": "hello"})
    };
    let finish = if ask == "bad" { "tool_calls" } else { "stop" };
    Json(
        json!({"id": "c", "object": "chat.completion", "created": 1, "model": "m",
        "choices": [{"index": 0, "message": message, "finish_reason": finish}],
        "usage": usage(10, 5)}),
    )
    .into_response()
}

struct Rig {
    url: String,
    alice: String,
    gary: String,
    db: std::path::PathBuf,
    _dir: tempfile::TempDir,
}

async fn rig() -> Rig {
    let upstream = format!(
        "http://{}",
        serve(Router::new().route("/chat/completions", post(completions))).await
    );
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("ledger.db");
    let (alice, alice_hash) = humpyard::auth::generate_key().unwrap();
    let (gary, gary_hash) = humpyard::auth::generate_key().unwrap();
    let toml = format!(
        "listen = \"127.0.0.1:0\"\n[ledger]\npath = \"{}\"\n\
         [keys.alice]\nsha256 = \"{alice_hash}\"\n\
         [admin.credentials.gary]\nsha256 = \"{gary_hash}\"\nscopes = [\"read\"]\n\
         [providers.p]\nbase_url = \"{upstream}\"\napi_key_env = \"K\"\nmax_retries = 0\ntimeout_secs = 60\n\
         [targets.t]\nendpoints = [{{ provider = \"p\", model = \"m\" }}]\n\
         [routes.r]\ntype = \"passthrough\"\ntargets = [\"t\"]\n",
        db.display()
    );
    let config = Config::from_toml(&toml, |_| Some("k".into())).unwrap();
    let url = format!(
        "http://{}",
        serve(server::router(config).await.unwrap()).await
    );
    Rig {
        url,
        alice,
        gary,
        db,
        _dir: dir,
    }
}

impl Rig {
    /// Sends `ask` (streaming when `stream`) and returns its request id and the response, kept so a stream stays open.
    async fn ask(&self, ask: &str, stream: bool) -> (String, reqwest::Response) {
        let response = reqwest::Client::new()
            .post(format!("{}/v1/chat/completions", self.url))
            .bearer_auth(&self.alice)
            .json(&json!({"model": "r", "stream": stream,
                          "messages": [{"role": "user", "content": ask}]}))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        let id = response.headers()["x-humpyard-request-id"]
            .to_str()
            .unwrap()
            .to_string();
        (id, response)
    }

    /// The decision for `id` as `/v1/decisions` and `/admin/v1/decisions` return it.
    async fn decision(&self, id: &str) -> (Value, Value) {
        let get = |path: String, key: String| async move {
            let body: Value = reqwest::Client::new()
                .get(format!("{}{path}", self.url))
                .bearer_auth(key)
                .send()
                .await
                .unwrap()
                .json()
                .await
                .unwrap();
            let data = body["data"].as_array().unwrap().clone();
            assert_eq!(data.len(), 1, "one decision for {id}");
            data[0].clone()
        };
        (
            get(format!("/v1/decisions?request_id={id}"), self.alice.clone()).await,
            get(
                format!("/admin/v1/decisions?request_id={id}"),
                self.gary.clone(),
            )
            .await,
        )
    }

    /// What `humpyard ctl decisions` prints for alice.
    async fn ctl(&self) -> String {
        let alice = self.alice.clone();
        let env = move |n: &str| (n == "CTL_KEY").then(|| alice.clone());
        let settings = Settings {
            url: Some(self.url.clone()),
            key_env: "CTL_KEY".into(),
            allow_insecure_http: false,
            json: false,
        };
        let gateway = connect(&settings, &env).unwrap();
        let mut out: Vec<u8> = Vec::new();
        run(
            &gateway,
            Command::Decisions {
                limit: Some(20),
                route: None,
                rule: None,
                outcome: None,
                request_id: None,
            },
            false,
            &mut out,
        )
        .await
        .unwrap();
        String::from_utf8(out).unwrap()
    }

    /// Waits until the ledger holds `rows` usage rows.
    async fn wait_for_rows(&self, rows: usize) {
        let ledger = Ledger::open(&self.db).await.unwrap();
        for _ in 0..200 {
            if ledger.entries_since(0).await.unwrap().len() >= rows {
                return;
            }
            tokio::time::sleep(Duration::from_millis(30)).await;
        }
        panic!("the ledger never held {rows} rows");
    }
}

/// The response signals of a decision, or `None` when the field is absent.
fn signals(decision: &Value) -> Option<Vec<String>> {
    decision.get("response_signals").map(|v| {
        v.as_array()
            .unwrap()
            .iter()
            .map(|s| s.as_str().unwrap().to_string())
            .collect()
    })
}

#[tokio::test]
async fn a_flagged_answer_lists_its_signals_on_all_three_surfaces() {
    let rig = rig().await;
    let (id, _r) = rig.ask("bad", false).await;
    rig.wait_for_rows(1).await;
    let (public, admin) = rig.decision(&id).await;
    assert_eq!(signals(&public), Some(vec!["bad_tool_call".to_string()]));
    assert_eq!(signals(&admin), Some(vec!["bad_tool_call".to_string()]));
    let out = rig.ctl().await;
    assert!(out.contains("response signals"), "{out}");
    assert!(out.contains(&format!("  {id}  bad_tool_call\n")), "{out}");
}

#[tokio::test]
async fn a_clean_answer_shows_nothing_and_its_output_is_unchanged() {
    let rig = rig().await;
    let (id, _r) = rig.ask("good", false).await;
    rig.wait_for_rows(1).await;
    let (public, admin) = rig.decision(&id).await;
    assert_eq!(signals(&public), None);
    assert_eq!(signals(&admin), None);
    let out = rig.ctl().await;
    assert!(!out.contains("response signals"), "{out}");
}

#[tokio::test]
async fn a_stream_still_open_is_returned_without_signals() {
    let rig = rig().await;
    let (id, response) = rig.ask("slow", true).await;
    // The provider is still streaming: no answer row yet, and nothing fails.
    let (public, admin) = rig.decision(&id).await;
    assert_eq!(signals(&public), None);
    assert_eq!(signals(&admin), None);
    assert!(!rig.ctl().await.contains("response signals"));
    drop(response);
}

#[tokio::test]
async fn unknown_signals_show_nothing_as_on_a_decision_from_before_the_columns() {
    let rig = rig().await;
    let (id, _r) = rig.ask("bad", false).await;
    rig.wait_for_rows(1).await;
    assert!(
        signals(&rig.decision(&id).await.0).is_some(),
        "flagged first"
    );
    // Make the row read as unknown, which is how every row written before schema v13 reads.
    let options = sqlx::sqlite::SqliteConnectOptions::new().filename(&rig.db);
    let mut conn = sqlx::SqliteConnection::connect_with(&options)
        .await
        .unwrap();
    sqlx::query(
        "UPDATE usage SET empty_output = NULL, truncated_output = NULL, bad_tool_call = NULL, cut_stream = NULL",
    )
    .execute(&mut conn)
    .await
    .unwrap();
    conn.close().await.unwrap();
    let (public, admin) = rig.decision(&id).await;
    assert_eq!(signals(&public), None);
    assert_eq!(signals(&admin), None);
    assert!(!rig.ctl().await.contains("response signals"));
}
