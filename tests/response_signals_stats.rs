//! Statistics report the response signals per group (spec: `routing-statistics`, change
//! `record-response-quality-signals`, ADR 0030): how many answer calls were observed and how many carried each
//! signal, so a rate is a count over the observed calls. Calls with unknown signals count in neither figure, and
//! each key sees only its own calls.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test scaffolding

mod common;

use std::time::Duration;

use axum::Router;
use axum::extract::Json;
use axum::routing::post;
use common::{serve, usage};
use humpyard::config::Config;
use humpyard::ledger::Ledger;
use humpyard::server;
use serde_json::{Value, json};
use sqlx::Connection;

/// A user message of `length` makes the upstream stop at its output limit; anything else is a normal answer.
async fn completions(Json(body): Json<Value>) -> Json<Value> {
    let finish = if body["messages"][0]["content"] == "length" {
        "length"
    } else {
        "stop"
    };
    Json(
        json!({"id": "c", "object": "chat.completion", "created": 1, "model": "m",
        "choices": [{"index": 0, "message": {"role": "assistant", "content": "hello"}, "finish_reason": finish}],
        "usage": usage(10, 5)}),
    )
}

struct Rig {
    url: String,
    alice: String,
    bob: String,
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
    let (bob, bob_hash) = humpyard::auth::generate_key().unwrap();
    let (gary, gary_hash) = humpyard::auth::generate_key().unwrap();
    let toml = format!(
        "listen = \"127.0.0.1:0\"\n[ledger]\npath = \"{}\"\n\
         [keys.alice]\nsha256 = \"{alice_hash}\"\n[keys.bob]\nsha256 = \"{bob_hash}\"\n\
         [admin.credentials.gary]\nsha256 = \"{gary_hash}\"\nscopes = [\"read\"]\n\
         [providers.p]\nbase_url = \"{upstream}\"\napi_key_env = \"K\"\nmax_retries = 0\ntimeout_secs = 5\n\
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
        bob,
        gary,
        db,
        _dir: dir,
    }
}

impl Rig {
    async fn ask(&self, key: &str, content: &str) {
        let response = reqwest::Client::new()
            .post(format!("{}/v1/chat/completions", self.url))
            .bearer_auth(key)
            .json(&json!({"model": "r", "messages": [{"role": "user", "content": content}]}))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
    }

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

    /// The `response_signals` of the group `t` from a statistics URL.
    async fn signals(&self, path: &str, key: &str) -> Value {
        let body: Value = reqwest::Client::new()
            .get(format!("{}{path}", self.url))
            .bearer_auth(key)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        let group = body["data"]
            .as_array()
            .unwrap()
            .iter()
            .find(|g| g["group"] == "t")
            .unwrap_or_else(|| panic!("no group t in {body}"));
        group["response_signals"].clone()
    }
}

#[tokio::test]
async fn a_group_reports_its_observed_calls_and_a_count_per_signal() {
    let rig = rig().await;
    for _ in 0..8 {
        rig.ask(&rig.alice, "hi").await;
    }
    for _ in 0..2 {
        rig.ask(&rig.alice, "length").await;
    }
    rig.wait_for_rows(10).await;
    let expected = json!({"observed": 10, "empty_output": 0, "truncated_output": 2,
                          "bad_tool_call": 0, "cut_stream": 0});
    assert_eq!(
        rig.signals("/v1/stats?group_by=target", &rig.alice).await,
        expected
    );
    assert_eq!(
        rig.signals("/admin/v1/stats?group_by=target", &rig.gary)
            .await,
        expected
    );
}

#[tokio::test]
async fn unknown_calls_are_in_neither_figure() {
    let rig = rig().await;
    for _ in 0..4 {
        rig.ask(&rig.alice, "hi").await;
    }
    for _ in 0..2 {
        rig.ask(&rig.alice, "length").await;
    }
    rig.wait_for_rows(6).await;
    // Make three of the clean rows and one of the truncated rows read as unknown, as rows from before schema v13 do.
    let options = sqlx::sqlite::SqliteConnectOptions::new().filename(&rig.db);
    let mut conn = sqlx::SqliteConnection::connect_with(&options)
        .await
        .unwrap();
    for id in [1, 2, 3, 6] {
        sqlx::query(
            "UPDATE usage SET empty_output = NULL, truncated_output = NULL, bad_tool_call = NULL, \
             cut_stream = NULL WHERE id = ?1",
        )
        .bind(id)
        .execute(&mut conn)
        .await
        .unwrap();
    }
    conn.close().await.unwrap();
    // Ids 4 (clean) and 5 (truncated) remain known: two observed, one truncated; the rest do not lower any rate.
    assert_eq!(
        rig.signals("/v1/stats?group_by=target", &rig.alice).await,
        json!({"observed": 2, "empty_output": 0, "truncated_output": 1,
               "bad_tool_call": 0, "cut_stream": 0})
    );
}

#[tokio::test]
async fn two_keys_see_only_their_own_figures() {
    let rig = rig().await;
    for _ in 0..3 {
        rig.ask(&rig.alice, "hi").await;
    }
    for _ in 0..2 {
        rig.ask(&rig.bob, "length").await;
    }
    rig.wait_for_rows(5).await;
    let alice = rig.signals("/v1/stats?group_by=target", &rig.alice).await;
    let bob = rig.signals("/v1/stats?group_by=target", &rig.bob).await;
    assert_eq!(
        (alice["observed"].clone(), alice["truncated_output"].clone()),
        (json!(3), json!(0))
    );
    assert_eq!(
        (bob["observed"].clone(), bob["truncated_output"].clone()),
        (json!(2), json!(2))
    );
    // The admin view can filter to one key and sees the same figures; unfiltered it counts both.
    assert_eq!(
        rig.signals("/admin/v1/stats?group_by=target&key=bob", &rig.gary)
            .await,
        bob
    );
    let both = rig
        .signals("/admin/v1/stats?group_by=target", &rig.gary)
        .await;
    assert_eq!(both["observed"], 5);
    assert_eq!(both["truncated_output"], 2);
}
