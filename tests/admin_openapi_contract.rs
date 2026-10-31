//! The admin API's `OpenAPI` document is a contract for the fields it describes (spec: `routing-statistics` and
//! `decision-log`, change `record-response-quality-signals`, ADR 0030): real `/admin/v1/decisions` and
//! `/admin/v1/stats` responses are checked against the schemas in `docs/openapi/admin-v1.json`, and the check fails
//! when a documented field is missing. (The route table itself is checked by a unit test in `server::admin`.)
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

const DOCUMENT: &str = include_str!("../docs/openapi/admin-v1.json");

/// Follows a `#/components/...` reference.
fn resolve<'a>(doc: &'a Value, schema: &'a Value) -> &'a Value {
    match schema["$ref"].as_str() {
        Some(reference) => {
            let mut node = doc;
            for part in reference.trim_start_matches("#/").split('/') {
                node = &node[part];
            }
            resolve(doc, node)
        }
        None => schema,
    }
}

/// What differs between `value` and `schema`: a missing required property, a wrong type, a value outside an enum.
/// Properties the schema does not list are ignored, because the document says only the listed ones are contract.
fn violations(doc: &Value, schema: &Value, value: &Value, path: &str) -> Vec<String> {
    let schema = resolve(doc, schema);
    let mut out = Vec::new();
    if let Some(parts) = schema["allOf"].as_array() {
        for part in parts {
            out.extend(violations(doc, part, value, path));
        }
    }
    if value.is_null() && schema["nullable"] == true {
        return out;
    }
    match schema["type"].as_str() {
        Some("object") => {
            let Some(map) = value.as_object() else {
                out.push(format!("{path}: not an object"));
                return out;
            };
            for name in schema["required"].as_array().into_iter().flatten() {
                let name = name.as_str().unwrap();
                if !map.contains_key(name) {
                    out.push(format!("{path}.{name}: missing"));
                }
            }
            for (name, sub) in schema["properties"].as_object().into_iter().flatten() {
                if let Some(found) = map.get(name) {
                    out.extend(violations(doc, sub, found, &format!("{path}.{name}")));
                }
            }
        }
        Some("array") => {
            let Some(items) = value.as_array() else {
                out.push(format!("{path}: not an array"));
                return out;
            };
            for (n, item) in items.iter().enumerate() {
                out.extend(violations(
                    doc,
                    &schema["items"],
                    item,
                    &format!("{path}[{n}]"),
                ));
            }
        }
        Some("integer") if !(value.is_u64() || value.is_i64()) => {
            out.push(format!("{path}: not an integer"));
        }
        Some("string") => match value.as_str() {
            None => out.push(format!("{path}: not a string")),
            Some(text) => {
                let allowed = schema["enum"].as_array();
                if allowed.is_some_and(|list| !list.iter().any(|a| a == text)) {
                    out.push(format!(
                        "{path}: {text:?} is not one of the documented values"
                    ));
                }
            }
        },
        Some("boolean") if !value.is_boolean() => out.push(format!("{path}: not a boolean")),
        _ => {}
    }
    out
}

/// The schema of a 200 response of an admin operation.
fn response_schema(doc: &Value, path: &str) -> Value {
    doc["paths"][path]["get"]["responses"]["200"]["content"]["application/json"]["schema"].clone()
}

/// The upstream answers "hello", or a tool call with cut-off arguments when the user message is `bad`.
async fn completions(Json(body): Json<Value>) -> Json<Value> {
    let bad = body["messages"][0]["content"] == "bad";
    let message = if bad {
        json!({"role": "assistant", "content": null, "tool_calls": [{"id": "call_1", "type": "function",
               "function": {"name": "get_weather", "arguments": "{\"city\":\"Par"}}]})
    } else {
        json!({"role": "assistant", "content": "hello"})
    };
    Json(
        json!({"id": "c", "object": "chat.completion", "created": 1, "model": "m",
        "choices": [{"index": 0, "message": message, "finish_reason": if bad { "tool_calls" } else { "stop" }}],
        "usage": usage(10, 5)}),
    )
}

struct Rig {
    url: String,
    gary: String,
}

/// A gateway that has served one good and one flagged answer, with an admin credential that may read.
async fn rig() -> (Rig, tempfile::TempDir) {
    let upstream = format!(
        "http://{}",
        serve(Router::new().route("/chat/completions", post(completions))).await
    );
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("ledger.db");
    let (alice, alice_hash) = humpyard::auth::generate_key().unwrap();
    let (gary, gary_hash) = humpyard::auth::generate_key().unwrap();
    let toml = format!(
        "listen = \"127.0.0.1:0\"\n[ledger]\npath = \"{}\"\n[keys.alice]\nsha256 = \"{alice_hash}\"\n\
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
    for ask in ["good", "bad"] {
        let status = reqwest::Client::new()
            .post(format!("{url}/v1/chat/completions"))
            .bearer_auth(&alice)
            .json(&json!({"model": "r", "messages": [{"role": "user", "content": ask}]}))
            .send()
            .await
            .unwrap()
            .status();
        assert_eq!(status, 200);
    }
    let ledger = Ledger::open(&db).await.unwrap();
    for _ in 0..200 {
        if ledger.entries_since(0).await.unwrap().len() >= 2 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(30)).await;
    }
    (Rig { url, gary }, dir)
}

impl Rig {
    async fn get(&self, path: &str) -> Value {
        reqwest::Client::new()
            .get(format!("{}{path}", self.url))
            .bearer_auth(&self.gary)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap()
    }
}

#[tokio::test]
async fn real_decisions_and_statistics_match_the_documented_schemas() {
    let doc: Value = serde_json::from_str(DOCUMENT).unwrap();
    let (rig, _dir) = rig().await;

    let decisions = rig.get("/admin/v1/decisions").await;
    let found = violations(
        &doc,
        &response_schema(&doc, "/admin/v1/decisions"),
        &decisions,
        "decisions",
    );
    assert!(found.is_empty(), "{found:#?}\n{decisions}");
    // The flagged answer's decision carries the documented signal; the good one carries none.
    let signalled: Vec<&Value> = decisions["data"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|d| d.get("response_signals").is_some())
        .collect();
    assert_eq!(signalled.len(), 1, "{decisions}");
    assert_eq!(signalled[0]["response_signals"], json!(["bad_tool_call"]));

    let stats = rig.get("/admin/v1/stats?group_by=target").await;
    let found = violations(
        &doc,
        &response_schema(&doc, "/admin/v1/stats"),
        &stats,
        "stats",
    );
    assert!(found.is_empty(), "{found:#?}\n{stats}");
    assert_eq!(
        stats["data"][0]["response_signals"],
        json!({"observed": 2, "empty_output": 0, "truncated_output": 0, "bad_tool_call": 1, "cut_stream": 0})
    );
}

#[tokio::test]
async fn the_check_fails_when_a_documented_field_is_missing_from_a_response() {
    let doc: Value = serde_json::from_str(DOCUMENT).unwrap();
    let (rig, _dir) = rig().await;
    let schema = response_schema(&doc, "/admin/v1/stats");

    // The whole object gone, then one counter gone: each is reported by name.
    let mut stats = rig.get("/admin/v1/stats?group_by=target").await;
    stats["data"][0]
        .as_object_mut()
        .unwrap()
        .remove("response_signals");
    let found = violations(&doc, &schema, &stats, "stats");
    assert_eq!(found, ["stats.data[0].response_signals: missing"]);

    let mut stats = rig.get("/admin/v1/stats?group_by=target").await;
    stats["data"][0]["response_signals"]
        .as_object_mut()
        .unwrap()
        .remove("cut_stream");
    let found = violations(&doc, &schema, &stats, "stats");
    assert_eq!(
        found,
        ["stats.data[0].response_signals.cut_stream: missing"]
    );

    // A signal name outside the documented set is reported too.
    let schema = response_schema(&doc, "/admin/v1/decisions");
    let mut decisions = rig.get("/admin/v1/decisions").await;
    let flagged = decisions["data"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|d| d.get("response_signals").is_some())
        .unwrap();
    flagged["response_signals"] = json!(["something_else"]);
    let found = violations(&doc, &schema, &decisions, "decisions");
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(
        found[0].contains("not one of the documented values"),
        "{found:?}"
    );
}
