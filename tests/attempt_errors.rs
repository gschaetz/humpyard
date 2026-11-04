//! A decision keeps, for each failed attempt, what its provider said (spec: `decision-log`, change
//! `record-attempt-errors`, ADR 0028): scrubbed, bounded, and never any part of the request.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test scaffolding

mod common;

use std::fmt::Write as _;
use std::sync::Arc;

use axum::Router;
use axum::extract::State;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use common::{chat_completion, serve, usage};
use humpyard::config::Config;
use humpyard::server;
use serde_json::{Value, json};

/// What one upstream answers: a status, a content type and a body, or a success.
#[derive(Clone)]
enum Answer {
    Fail(u16, &'static str, String),
    Ok,
    /// Never answers within the provider's timeout.
    Stall,
}

async fn answer(State(answer): State<Arc<Answer>>) -> Response {
    match answer.as_ref() {
        Answer::Fail(status, content_type, body) => (
            StatusCode::from_u16(*status).unwrap(),
            [(header::CONTENT_TYPE, *content_type)],
            body.clone(),
        )
            .into_response(),
        Answer::Ok => axum::Json(chat_completion("m", "ok", Some(usage(10, 5)))).into_response(),
        Answer::Stall => {
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            StatusCode::OK.into_response()
        }
    }
}

async fn upstream(answer_with: Answer) -> String {
    let app = Router::new()
        .route("/chat/completions", post(answer))
        .with_state(Arc::new(answer_with));
    format!("http://{}", serve(app).await)
}

fn json_body(value: &Value) -> String {
    value.to_string()
}

/// A gateway with one provider and target per upstream (`t0`, `t1`, ...), a passthrough route `r` over them that
/// falls over on `fallback_on`, and a ledger. Returns its URL.
async fn gateway(upstreams: Vec<Answer>, fallback_on: &str) -> (String, tempfile::TempDir, String) {
    let dir = tempfile::tempdir().unwrap();
    let (admin_secret, admin_hash) = humpyard::auth::generate_key().unwrap();
    let mut toml = format!(
        "listen = \"127.0.0.1:0\"\n[ledger]\npath = \"{}\"\n\
         [admin.credentials.gary]\nsha256 = \"{admin_hash}\"\nscopes = [\"read\"]\n",
        dir.path().join("ledger.db").display()
    );
    let mut targets = Vec::new();
    for (n, answer_with) in upstreams.into_iter().enumerate() {
        let url = upstream(answer_with).await;
        write!(
            toml,
            "[providers.p{n}]\nbase_url = \"{url}\"\napi_key_env = \"K\"\nmax_retries = 0\ntimeout_secs = 1\n\
             [targets.t{n}]\nendpoints = [{{ provider = \"p{n}\", model = \"m\" }}]\n"
        )
        .unwrap();
        targets.push(format!("\"t{n}\""));
    }
    write!(
        toml,
        "[routes.r]\ntype = \"passthrough\"\ntargets = [{}]\nfallback_on = {fallback_on}\n",
        targets.join(", ")
    )
    .unwrap();
    let config = Config::from_toml(&toml, |_| Some("k".into())).unwrap();
    let url = format!(
        "http://{}",
        serve(server::router(config).await.unwrap()).await
    );
    (url, dir, admin_secret)
}

async fn send(url: &str, content: &str) -> u16 {
    reqwest::Client::new()
        .post(format!("{url}/v1/chat/completions"))
        .json(&json!({"model": "r", "messages": [{"role": "user", "content": content}]}))
        .send()
        .await
        .unwrap()
        .status()
        .as_u16()
}

/// The newest decision, as `GET /v1/decisions` returns it.
async fn last_decision(url: &str) -> Value {
    let body: Value = reqwest::get(format!("{url}/v1/decisions?limit=1"))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    body["data"][0].clone()
}

fn messages(decision: &Value) -> Vec<Option<String>> {
    decision["attempts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["message"].as_str().map(str::to_owned))
        .collect()
}

#[tokio::test]
async fn every_rejection_in_a_chain_keeps_its_own_providers_message() {
    // Three providers, three body shapes: OpenAI's, Google's list, and plain text. The first echoes a key-shaped
    // value, built at run time so no source line holds one.
    let key = format!("{}{}", "sk-", "abcdefghijkl1234");
    let (url, _dir, _) = gateway(
        vec![
            Answer::Fail(
                400,
                "application/json",
                json_body(
                    &json!({"error": {"message": format!("alpha refuses the tools; key {key}")}}),
                ),
            ),
            Answer::Fail(
                400,
                "application/json",
                json_body(
                    &json!([{"error": {"code": 400, "message": "beta says invalid argument"}}]),
                ),
            ),
            Answer::Fail(400, "text/plain", "gamma: bad request".into()),
        ],
        "[\"rejected\"]",
    )
    .await;
    assert_eq!(send(&url, "hello").await, 400);

    let decision = last_decision(&url).await;
    let attempts = decision["attempts"].as_array().unwrap();
    assert_eq!(attempts.len(), 3, "{decision}");
    assert!(attempts.iter().all(|a| a["outcome"] == "rejected"));
    let messages = messages(&decision);
    let text = |n: usize| messages[n].clone().unwrap_or_default();
    assert!(
        text(0).starts_with("400: alpha refuses the tools"),
        "{messages:?}"
    );
    assert!(
        text(1).starts_with("400: beta says invalid argument"),
        "{messages:?}"
    );
    assert!(
        text(2).starts_with("400: gamma: bad request"),
        "{messages:?}"
    );
    // The key-shaped value is gone from every stored message and from the whole decision.
    assert!(text(0).contains("[redacted]"), "{messages:?}");
    assert!(!decision.to_string().contains(&key), "{decision}");
}

#[tokio::test]
async fn a_rate_limit_keeps_its_message_and_the_success_that_follows_has_none() {
    let (url, _dir, _) = gateway(
        vec![
            Answer::Fail(
                429,
                "application/json",
                json_body(&json!({"error": {"message": "slow down"}})),
            ),
            Answer::Ok,
        ],
        "[\"rate_limit\"]",
    )
    .await;
    assert_eq!(send(&url, "hello").await, 200);

    let decision = last_decision(&url).await;
    assert_eq!(
        messages(&decision),
        vec![Some("429: slow down".to_string()), None],
        "{decision}"
    );
    assert_eq!(decision["attempts"][0]["outcome"], "rate_limit");
    assert_eq!(decision["attempts"][1]["outcome"], "ok");
}

#[tokio::test]
async fn a_timeout_keeps_a_description_of_what_happened() {
    let (url, _dir, _) = gateway(vec![Answer::Stall, Answer::Ok], "[\"timeout\"]").await;
    assert_eq!(send(&url, "hello").await, 200);

    let decision = last_decision(&url).await;
    assert_eq!(decision["attempts"][0]["outcome"], "timeout", "{decision}");
    let message = messages(&decision)[0].clone().unwrap_or_default();
    assert!(!message.is_empty(), "{decision}");
    assert!(message.chars().count() <= 200);
    assert_eq!(messages(&decision)[1], None);
}

#[tokio::test]
async fn no_part_of_the_request_reaches_the_decision() {
    // The provider's refusal does not echo the request, so nothing of it may appear anywhere in the decision.
    let marker = "zebra-quartz-marker-request-text";
    let (url, _dir, _) = gateway(
        vec![Answer::Fail(
            400,
            "application/json",
            json_body(&json!({"error": {"message": "invalid argument"}})),
        )],
        "[]",
    )
    .await;
    assert_eq!(send(&url, marker).await, 400);

    let decision = last_decision(&url).await;
    assert!(!decision.to_string().contains(marker), "{decision}");
    assert_eq!(
        messages(&decision),
        vec![Some("400: invalid argument".into())]
    );
}

#[tokio::test]
async fn the_admin_decisions_endpoint_returns_the_message_too() {
    let (url, _dir, admin) = gateway(
        vec![Answer::Fail(
            400,
            "application/json",
            json_body(&json!({"error": {"message": "invalid argument"}})),
        )],
        "[]",
    )
    .await;
    assert_eq!(send(&url, "hello").await, 400);

    let body: Value = reqwest::Client::new()
        .get(format!("{url}/admin/v1/decisions?limit=1"))
        .bearer_auth(&admin)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        body["data"][0]["attempts"][0]["message"], "400: invalid argument",
        "{body}"
    );
}
