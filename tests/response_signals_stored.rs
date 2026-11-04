//! Which usage rows carry the facts (spec: `usage-ledger`, change `record-response-quality-signals`, ADR 0030):
//! the call whose answer reached the client does, and a routing-time judge call, which the client never sees,
//! reads as unknown.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test scaffolding

mod common;

use std::fmt::Write as _;
use std::time::Duration;

use axum::Router;
use axum::extract::Json;
use axum::routing::post;
use common::{chat_request, serve};
use humpyard::config::Config;
use humpyard::ledger::{Kind, Ledger};
use humpyard::server;
use humpyard::signals::Signals;
use serde_json::{Value, json};

/// The judge model answers with a usable verdict; every other model answers "hello".
async fn completions(Json(body): Json<Value>) -> Json<Value> {
    let model = body["model"].as_str().unwrap().to_string();
    let content = if model == "judge-m" {
        json!({"crux": "x", "primary_rule": "SUP-1", "capability_boundary": "supported", "p_solve": 0.9})
            .to_string()
    } else {
        "hello".to_string()
    };
    Json(
        json!({"id": "c", "object": "chat.completion", "created": 1, "model": model,
        "choices": [{"index": 0, "message": {"role": "assistant", "content": content}, "finish_reason": "stop"}],
        "usage": {"prompt_tokens": 3, "completion_tokens": 2, "total_tokens": 5}}),
    )
}

#[tokio::test]
async fn a_judge_calls_row_reads_unknown_and_the_answers_row_holds_the_facts() {
    let upstream = serve(Router::new().route("/chat/completions", post(completions))).await;
    let dir = tempfile::tempdir().unwrap();
    let mut toml = format!(
        "listen = \"127.0.0.1:0\"\n[ledger]\npath = \"{}\"\n\
         [providers.mock]\nbase_url = \"http://{upstream}\"\napi_key_env = \"K\"\nmax_retries = 0\n",
        dir.path().join("ledger.db").display()
    );
    for (target, model) in [
        ("fast", "fast-m"),
        ("smart", "smart-m"),
        ("judge", "judge-m"),
    ] {
        let _ = writeln!(
            toml,
            "[targets.{target}]\nendpoints = [{{ provider = \"mock\", model = \"{model}\" }}]"
        );
    }
    toml += "[routes.judged]\ntype = \"llm_classifier\"\nmode = \"capability\"\n\
             efficient = [\"fast\"]\ncapable = [\"smart\"]\njudge = [\"judge\"]\n";
    let config = Config::from_toml(&toml, |_| Some("key".into())).unwrap();
    let url = format!(
        "http://{}",
        serve(server::router(config).await.unwrap()).await
    );
    let status = reqwest::Client::new()
        .post(format!("{url}/v1/chat/completions"))
        .json(&chat_request("judged", false))
        .send()
        .await
        .unwrap()
        .status();
    assert_eq!(status, 200);

    let ledger = Ledger::open(&dir.path().join("ledger.db")).await.unwrap();
    let mut rows = Vec::new();
    for _ in 0..200 {
        rows = ledger.entries_since(0).await.unwrap();
        if rows.len() >= 2 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(30)).await;
    }
    assert_eq!(rows.len(), 2, "one judge call and one answer");
    let judge = rows.iter().find(|r| r.kind == Kind::Judge).unwrap();
    let served = rows.iter().find(|r| r.kind == Kind::Answer).unwrap();
    assert_eq!(judge.signals, None, "a judge call is never judged");
    assert_eq!(served.signals, Some(Signals::default()));
}
