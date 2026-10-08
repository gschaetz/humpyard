//! Usage metering: every upstream call a request makes (judge calls and the answer) becomes one
//! ledger entry attributed to the caller. Buffered calls are measured when they return; streamed
//! answers are measured by a tap on the stream when it ends or is dropped.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use async_trait::async_trait;
use futures::StreamExt;
use http::{HeaderName, HeaderValue};
use parking_lot::Mutex;
use switchyard_llm_client::ClientRouter;
use switchyard_protocol::{
    LlmClientError, LlmResponse, LlmResponseChunk, LlmResponseStream, ModelId, Request, Response,
    RoutedLlmClient, Usage,
};

use crate::budget::BudgetTracker;
use crate::clock::Clock;
use crate::config::Price;
use crate::estimate::tokens_for_bytes;
use crate::ledger::{Entry, Kind, Ledger};
use crate::pool::{Served, TargetClient};
use crate::pricing::cost_micro_usd;

/// Internal response header carrying a call's id from the metered client back to the handler.
/// It never reaches the client: the handler removes it.
pub const CALL_ID_HEADER: &str = "x-humpyard-call-id";

/// Where finished entries go: the ledger (when configured).
pub struct Accounting {
    ledger: Option<Ledger>,
    clock: Arc<dyn Clock>,
    tracker: Option<Arc<BudgetTracker>>,
}

impl Accounting {
    pub fn new(
        ledger: Option<Ledger>,
        clock: Arc<dyn Clock>,
        tracker: Option<Arc<BudgetTracker>>,
    ) -> Self {
        Self {
            ledger,
            clock,
            tracker,
        }
    }

    pub fn ledger(&self) -> Option<&Ledger> {
        self.ledger.as_ref()
    }

    /// Waits until everything recorded so far is in the ledger (a no-op without one).
    pub async fn flush(&self) {
        if let Some(ledger) = &self.ledger {
            ledger.flush().await;
        }
    }

    pub fn now_ms(&self) -> i64 {
        self.clock.now_ms()
    }

    /// Finalizes an entry: stamps the time, counts it against the key's budget immediately, and
    /// queues it for the ledger.
    pub fn complete(&self, mut entry: Entry) {
        entry.ts_ms = self.clock.now_ms();
        if let (Some(tracker), Some(key)) = (&self.tracker, &entry.key_id) {
            tracker.record(key, entry.cost_micro_usd, entry.total_tokens());
        }
        if let Some(ledger) = &self.ledger {
            ledger.record(entry);
        }
    }
}

/// The identity and routing facts every call of one request shares.
pub struct CallContext {
    accounting: Arc<Accounting>,
    key_id: Option<String>,
    session_id: Option<String>,
    route: String,
    next_id: AtomicU64,
    /// Buffered calls, completed on return but not yet classified as answer or judge.
    buffered: Mutex<Vec<(u64, Entry)>>,
    /// Streamed calls awaiting their tap.
    streams: Mutex<HashMap<u64, StreamTemplate>>,
}

pub struct StreamTemplate {
    entry: Entry,
    price: Option<Price>,
}

fn usage_is_empty(usage: &Usage) -> bool {
    usage.input_tokens.is_none()
        && usage.output_tokens.is_none()
        && usage.total_tokens.is_none()
        && usage.cached_input_tokens().is_none()
}

impl CallContext {
    pub fn new(
        accounting: Arc<Accounting>,
        key_id: Option<String>,
        session_id: Option<String>,
        route: String,
    ) -> Arc<Self> {
        Arc::new(Self {
            accounting,
            key_id,
            session_id,
            route,
            next_id: AtomicU64::new(1),
            buffered: Mutex::new(Vec::new()),
            streams: Mutex::new(HashMap::new()),
        })
    }

    fn base_entry(&self, target: &str, served: &Served) -> Entry {
        Entry {
            ts_ms: 0,
            key_id: self.key_id.clone(),
            session_id: self.session_id.clone(),
            route: self.route.clone(),
            target: target.to_string(),
            provider: served.provider.clone(),
            model: served.model.clone(),
            kind: Kind::Judge,
            input_tokens: 0,
            cached_input_tokens: 0,
            cache_creation_tokens: 0,
            output_tokens: 0,
            reasoning_tokens: 0,
            cost_micro_usd: 0,
            outcome: "ok".into(),
            usage_missing: true,
        }
    }

    /// A router whose clients meter every call into this context.
    pub fn router(
        self: &Arc<Self>,
        targets: &HashMap<ModelId, Arc<TargetClient>>,
        names: &[ModelId],
    ) -> ClientRouter {
        let clients: HashMap<ModelId, Arc<dyn RoutedLlmClient>> = names
            .iter()
            .filter_map(|name| {
                let inner = targets.get(name)?.clone();
                let metered = Metered {
                    inner,
                    target: name.to_string(),
                    ctx: self.clone(),
                };
                Some((name.clone(), Arc::new(metered) as Arc<dyn RoutedLlmClient>))
            })
            .collect();
        ClientRouter::new(clients)
    }

    /// The run succeeded. Buffered calls are recorded as the answer (the call whose id the
    /// response carried) or as judge calls. Returns the stream template when the answer streams.
    pub fn finish_ok(&self, answer_id: Option<u64>) -> Option<StreamTemplate> {
        for (id, mut entry) in self.buffered.lock().drain(..) {
            entry.kind = if Some(id) == answer_id {
                Kind::Answer
            } else {
                Kind::Judge
            };
            self.accounting.complete(entry);
        }
        let mut streams = self.streams.lock();
        let answer = answer_id.and_then(|id| streams.remove(&id));
        for (_, mut template) in streams.drain() {
            template.entry.outcome = "cancelled".into();
            self.accounting.complete(template.entry);
        }
        answer.map(|mut template| {
            template.entry.kind = Kind::Answer;
            template
        })
    }

    /// The run failed: tokens already spent on judge calls still count.
    pub fn finish_err(&self) {
        for (_, entry) in self.buffered.lock().drain(..) {
            self.accounting.complete(entry);
        }
        for (_, mut template) in self.streams.lock().drain() {
            template.entry.outcome = "cancelled".into();
            self.accounting.complete(template.entry);
        }
    }

    /// Wraps the answer stream so its usage is recorded when it ends or is dropped.
    pub fn tap(&self, stream: LlmResponseStream, template: StreamTemplate) -> LlmResponseStream {
        let mut recorder = StreamRecorder {
            template,
            accounting: self.accounting.clone(),
            usage: None,
            streamed_bytes: 0,
            terminal: false,
            failed: false,
            submitted: false,
        };
        Box::pin(async_stream::stream! {
            let mut stream = stream;
            while let Some(item) = stream.next().await {
                match &item {
                    Ok(event) => {
                        for chunk in event.normalized() {
                            recorder.streamed_bytes = recorder
                                .streamed_bytes
                                .saturating_add(generated_bytes(chunk));
                            match chunk {
                                LlmResponseChunk::Usage(usage) => recorder.usage = Some(usage.clone()),
                                LlmResponseChunk::MessageStop { .. } => recorder.terminal = true,
                                LlmResponseChunk::StreamError { .. }
                                | LlmResponseChunk::DecodeError { .. } => recorder.failed = true,
                                _ => {}
                            }
                        }
                    }
                    Err(_) => recorder.failed = true,
                }
                let stop = recorder.failed;
                yield item;
                if stop {
                    break;
                }
            }
            recorder.submit();
        })
    }
}

impl Drop for CallContext {
    /// A request cancelled mid-flight (shutdown, or a client that disconnected: axum drops the
    /// handler) still records the calls it already paid for. The normal paths drain these buffers
    /// first, so nothing is recorded twice.
    fn drop(&mut self) {
        self.finish_err();
    }
}

struct StreamRecorder {
    template: StreamTemplate,
    accounting: Arc<Accounting>,
    usage: Option<Usage>,
    /// Bytes of generated text seen so far, the basis of the estimate when no usage arrives.
    streamed_bytes: usize,
    terminal: bool,
    failed: bool,
    submitted: bool,
}

impl StreamRecorder {
    fn submit(&mut self) {
        if std::mem::replace(&mut self.submitted, true) {
            return;
        }
        let mut entry = self.template.entry.clone();
        entry.outcome = if self.failed {
            "error"
        } else if self.terminal {
            "ok"
        } else {
            "cancelled"
        }
        .into();
        if let Some(usage) = self.usage.as_ref().filter(|u| !usage_is_empty(u)) {
            fill_tokens(&mut entry, usage);
            entry.cost_micro_usd = self.template.price.map_or(0, |p| cost_micro_usd(usage, &p));
            entry.usage_missing = false;
        } else if self.streamed_bytes > 0 {
            // The provider's usage chunk never arrived (cut stream, dropped client), but it
            // generated what was delivered, so count an estimate rather than nothing. Input is
            // unknown here, so the estimate covers output only. `usage_missing` stays set to mark
            // the row as an estimate.
            let output = tokens_for_bytes(self.streamed_bytes);
            entry.output_tokens = output;
            entry.cost_micro_usd = self.template.price.map_or(0, |p| {
                let usage = Usage {
                    output_tokens: Some(output),
                    ..Usage::default()
                };
                cost_micro_usd(&usage, &p)
            });
            tracing::warn!(
                target = %entry.target,
                provider = %entry.provider,
                output_tokens = output,
                "stream ended without usage; recording an output-only estimate"
            );
        } else {
            tracing::warn!(
                target = %entry.target,
                provider = %entry.provider,
                "stream ended without usage or output; recording zero cost"
            );
        }
        self.accounting.complete(entry);
    }
}

impl Drop for StreamRecorder {
    /// A dropped stream (client disconnect) still records what was seen.
    fn drop(&mut self) {
        self.submit();
    }
}

/// Bytes of generated content a chunk carries (text, reasoning and tool-call arguments).
fn generated_bytes(chunk: &LlmResponseChunk) -> usize {
    match chunk {
        LlmResponseChunk::TextDelta { text, .. }
        | LlmResponseChunk::ReasoningDelta { text, .. }
        | LlmResponseChunk::ReasoningDetailsDelta { text, .. } => text.len(),
        LlmResponseChunk::ToolCallDelta {
            name,
            arguments_delta,
            ..
        } => name.as_deref().map_or(0, str::len) + arguments_delta.as_deref().map_or(0, str::len),
        _ => 0,
    }
}

fn fill_tokens(entry: &mut Entry, usage: &Usage) {
    entry.input_tokens = usage.input_tokens.unwrap_or(0);
    entry.cached_input_tokens = usage.cached_input_tokens().unwrap_or(0);
    entry.cache_creation_tokens = usage.cache_creation_input_tokens().unwrap_or(0);
    entry.output_tokens = usage.output_tokens.unwrap_or(0);
    entry.reasoning_tokens = usage.reasoning_tokens.unwrap_or(0);
}

/// A target client that reports every call it serves to its request's context.
struct Metered {
    inner: Arc<TargetClient>,
    target: String,
    ctx: Arc<CallContext>,
}

#[async_trait]
impl RoutedLlmClient for Metered {
    async fn call(&self, request: Request) -> Result<Response, LlmClientError> {
        let (mut response, served) = self.inner.call_detailed(request).await?;
        let id = self.ctx.next_id.fetch_add(1, Ordering::Relaxed);
        let mut entry = self.ctx.base_entry(&self.target, &served);
        match &response.llm_response {
            LlmResponse::Agg(agg) if !usage_is_empty(&agg.usage) => {
                fill_tokens(&mut entry, &agg.usage);
                entry.cost_micro_usd = served.price.map_or(0, |p| cost_micro_usd(&agg.usage, &p));
                entry.usage_missing = false;
                self.ctx.buffered.lock().push((id, entry));
            }
            LlmResponse::Agg(_) => {
                tracing::warn!(target = %self.target, provider = %served.provider, "response carried no usage; recording zero cost");
                self.ctx.buffered.lock().push((id, entry));
            }
            LlmResponse::Stream(_) => {
                self.ctx.streams.lock().insert(
                    id,
                    StreamTemplate {
                        entry,
                        price: served.price,
                    },
                );
            }
        }
        if let Ok(value) = HeaderValue::from_str(&id.to_string()) {
            response
                .upstream_headers
                .insert(HeaderName::from_static(CALL_ID_HEADER), value);
        }
        Ok(response)
    }
}
