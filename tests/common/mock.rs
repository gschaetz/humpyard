//! A mock OpenAI-compatible upstream for the harnesses: answers a completion after a fixed delay, or streams a
//! fixed number of chunks a fixed gap apart when the request asks to stream.
#![allow(clippy::unwrap_used)] // test scaffolding

use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::extract::State;
use axum::response::IntoResponse;
use axum::response::sse::{Event, Sse};
use axum::routing::post;

use super::{chat_completion, chunk, serve, usage};

struct Mock {
    delay: Duration,
}

/// Chunks of a streamed answer, and the gap the mock leaves between them.
pub const STREAM_CHUNKS: u64 = 20;
pub const STREAM_GAP: Duration = Duration::from_millis(5);

async fn completions(
    State(mock): State<Arc<Mock>>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> axum::response::Response {
    if body["stream"] == true {
        let events = async_stream::stream! {
            for i in 0..STREAM_CHUNKS {
                if i > 0 {
                    tokio::time::sleep(STREAM_GAP).await;
                }
                yield Ok::<_, std::io::Error>(Event::default().data(
                    chunk(serde_json::json!({"content": "word "}), None).to_string()));
            }
            let mut last = chunk(serde_json::json!({}), Some("stop"));
            last["usage"] = usage(20, STREAM_CHUNKS);
            yield Ok(Event::default().data(last.to_string()));
            yield Ok(Event::default().data("[DONE]"));
        };
        return Sse::new(events).into_response();
    }
    if !mock.delay.is_zero() {
        tokio::time::sleep(mock.delay).await;
    }
    axum::Json(chat_completion("m", "ok", Some(usage(20, 5)))).into_response()
}

pub async fn mock_upstream(delay: Duration) -> String {
    let app = Router::new()
        .route("/chat/completions", post(completions))
        .with_state(Arc::new(Mock { delay }));
    format!("http://{}", serve(app).await)
}
