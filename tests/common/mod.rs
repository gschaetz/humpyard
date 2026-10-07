//! Shared scaffolding for the integration tests: serving a router on an ephemeral port, capturing
//! logs, and building the OpenAI-shaped JSON the mock upstreams and clients exchange.
//!
//! Each test binary compiles this module separately and uses a subset of it.
#![allow(
    dead_code,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::needless_pass_by_value
)]

use std::net::SocketAddr;
use std::sync::{Arc, Mutex, OnceLock};

use axum::Router;
use serde_json::{Value, json};

/// Serves `app` on an ephemeral localhost port and returns its address.
pub async fn serve(app: Router) -> SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    addr
}

/// An in-memory log sink.
#[derive(Clone, Default)]
pub struct LogBuf(pub Arc<Mutex<Vec<u8>>>);

impl LogBuf {
    /// Everything logged so far.
    pub fn contents(&self) -> String {
        String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
    }
}

impl std::io::Write for LogBuf {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for LogBuf {
    type Writer = LogBuf;

    fn make_writer(&'a self) -> LogBuf {
        self.clone()
    }
}

/// Installs, once per test binary, a process-wide subscriber that captures every log line.
/// (Per-test subscribers race on tracing's global callsite cache.)
pub fn logs() -> &'static LogBuf {
    static LOGS: OnceLock<LogBuf> = OnceLock::new();
    LOGS.get_or_init(|| {
        let buf = LogBuf::default();
        tracing_subscriber::fmt()
            .with_max_level(tracing::Level::TRACE)
            .with_ansi(false)
            .with_writer(buf.clone())
            .init();
        buf
    })
}

/// A buffered chat-completion response body.
pub fn chat_completion(model: &str, content: &str, usage: Option<Value>) -> Value {
    let mut body = json!({
        "id": "c", "object": "chat.completion", "created": 1, "model": model,
        "choices": [{"index": 0, "message": {"role": "assistant", "content": content}, "finish_reason": "stop"}],
    });
    if let Some(usage) = usage {
        body["usage"] = usage;
    }
    body
}

/// Token usage as the chat-completions API reports it.
pub fn usage(prompt: u64, completion: u64) -> Value {
    json!({"prompt_tokens": prompt, "completion_tokens": completion, "total_tokens": prompt + completion})
}

/// One streamed chat-completion chunk.
pub fn chunk(delta: Value, finish: Option<&str>) -> Value {
    json!({"id": "c", "object": "chat.completion.chunk", "created": 1, "model": "m",
           "choices": [{"index": 0, "delta": delta, "finish_reason": finish}]})
}

/// A client request for `model`, optionally streaming.
pub fn chat_request(model: &str, stream: bool) -> Value {
    json!({"model": model, "stream": stream, "messages": [{"role": "user", "content": "hi"}]})
}

/// An agent turn: a task, a Bash tool call, and its result (a failure or a pass). A failing turn
/// makes a stage router escalate.
pub fn tool_turn(model: &str, failed: bool) -> Value {
    let result = if failed {
        "fatal runtime error: out of memory"
    } else {
        "ok"
    };
    json!({"model": model, "messages": [
        {"role": "user", "content": "fix the build"},
        {"role": "assistant", "content": null, "tool_calls": [{"id": "call_1", "type": "function",
            "function": {"name": "Bash", "arguments": "{\"command\":\"cargo test\"}"}}]},
        {"role": "tool", "tool_call_id": "call_1", "content": result}]})
}
