//! Heavy reads (ADR 0036): statistics and session views fold up to 50,000 ledger rows, so they are rationed. At most
//! one runs at a time on each surface (administrative, and client keys), a second is answered 503 with `Retry-After`
//! and is never queued; identical reads share one computation while it runs and its result for ten seconds after.
//!
//! A read is identified by the surface, the identity it reads for (a client key's id, or the admin `key` filter), the
//! endpoint and its normalised query, so one caller never gets another's result. Only a 200 within the size bound is
//! kept; an error or a busy answer is returned to whoever asked and then forgotten.
//!
//! Authentication and the scope check happen before a handler comes here, so a caller without the scope can neither
//! occupy a gate nor read a shared result.

use std::collections::HashMap;
use std::future::Future;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::body::Bytes;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde_json::json;
use tokio::sync::{OnceCell, OwnedSemaphorePermit, Semaphore, watch};

/// How long a computed answer is served to identical reads. A constant, not a setting: an operator has nothing
/// sensible to tune, and a knob invites setting it to zero.
pub const CACHE_TTL: Duration = Duration::from_secs(10);
/// Answers kept at once, and the largest one kept.
const MAX_ENTRIES: usize = 16;
const MAX_ENTRY_BYTES: usize = 2 * 1024 * 1024;
/// The most of an answer's body read back to store it.
const MAX_BODY_BYTES: usize = 64 * 1024 * 1024;
const RETRY_AFTER_SECS: &str = "5";

/// Which API a heavy read arrived on. Each has its own gate, so a client key reading its own statistics in a loop
/// cannot make the operator's GUI answer busy.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Surface {
    Admin,
    Client,
}

impl Surface {
    fn busy_code(self) -> &'static str {
        match self {
            Self::Admin => "admin_busy",
            Self::Client => "stats_busy",
        }
    }
}

/// The ledger could not be read in time (its read pool or lock was busy): the answer is a 503, not a failure.
pub(super) struct Busy;

#[derive(Clone, PartialEq, Eq, Hash)]
struct Key {
    surface: Surface,
    /// The identity read for: a client key's id, or the admin `key` filter.
    caller: Option<String>,
    endpoint: &'static str,
    /// The query parameters, sorted by name.
    query: Vec<(String, String)>,
}

#[derive(Clone)]
struct Answer {
    status: StatusCode,
    content_type: Option<HeaderValue>,
    body: Bytes,
}

impl Answer {
    async fn of(response: Response) -> Self {
        let (parts, body) = response.into_parts();
        Self {
            status: parts.status,
            content_type: parts.headers.get(header::CONTENT_TYPE).cloned(),
            body: axum::body::to_bytes(body, MAX_BODY_BYTES)
                .await
                .unwrap_or_default(),
        }
    }

    fn into_response(self) -> Response {
        let mut response = (self.status, self.body).into_response();
        if let Some(content_type) = self.content_type {
            response
                .headers_mut()
                .insert(header::CONTENT_TYPE, content_type);
        }
        response
    }
}

enum Outcome {
    Answer(Answer),
    Busy,
}

/// One read in flight or done: every caller of an identical read waits on the same cell.
struct Slot {
    cell: OnceCell<(Outcome, Instant)>,
}

pub struct HeavyReads {
    admin: Arc<Semaphore>,
    client: Arc<Semaphore>,
    slots: Mutex<HashMap<Key, Arc<Slot>>>,
    ttl: Duration,
    max_entries: usize,
    max_entry_bytes: usize,
    /// Computations started, for tests.
    computations: AtomicU64,
    /// While `true`, a computation that has its gate waits before reading, for tests.
    paused: Arc<watch::Sender<bool>>,
}

impl Default for HeavyReads {
    fn default() -> Self {
        Self::with_limits(CACHE_TTL, MAX_ENTRIES, MAX_ENTRY_BYTES)
    }
}

impl HeavyReads {
    /// With the given freshness (zero keeps nothing: every read after one finishes computes again).
    #[must_use]
    pub fn with_ttl(ttl: Duration) -> Self {
        Self::with_limits(ttl, MAX_ENTRIES, MAX_ENTRY_BYTES)
    }

    #[doc(hidden)]
    #[must_use]
    pub fn with_limits(ttl: Duration, max_entries: usize, max_entry_bytes: usize) -> Self {
        Self {
            admin: Arc::new(Semaphore::new(1)),
            client: Arc::new(Semaphore::new(1)),
            slots: Mutex::new(HashMap::new()),
            ttl,
            max_entries,
            max_entry_bytes,
            computations: AtomicU64::new(0),
            paused: Arc::new(watch::channel(false).0),
        }
    }

    fn gate(&self, surface: Surface) -> Arc<Semaphore> {
        match surface {
            Surface::Admin => self.admin.clone(),
            Surface::Client => self.client.clone(),
        }
    }

    /// Answers a heavy read. `compute` does the ledger work and is polled only by the caller that finds neither a
    /// fresh answer nor a read in flight and gets the gate; every other identical caller waits for that one.
    pub(super) async fn run<F>(
        &self,
        surface: Surface,
        endpoint: &'static str,
        caller: Option<&str>,
        params: &HashMap<String, String>,
        compute: F,
    ) -> Response
    where
        F: Future<Output = Result<Response, Busy>> + Send,
    {
        let mut query: Vec<(String, String)> =
            params.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        query.sort();
        let key = Key {
            surface,
            caller: caller.map(str::to_string),
            endpoint,
            query,
        };
        let slot = self.slot_for(&key);
        let gate = self.gate(surface);
        let (outcome, _) = slot
            .cell
            .get_or_init(|| async {
                let Ok(permit) = gate.try_acquire_owned() else {
                    return (Outcome::Busy, Instant::now());
                };
                self.wait_unpaused().await;
                self.computations.fetch_add(1, Ordering::Relaxed);
                let result = compute.await;
                drop(permit);
                match result {
                    Ok(response) => (Outcome::Answer(Answer::of(response).await), Instant::now()),
                    Err(Busy) => (Outcome::Busy, Instant::now()),
                }
            })
            .await;
        match outcome {
            Outcome::Answer(answer) => {
                if answer.status != StatusCode::OK
                    || answer.body.len() > self.max_entry_bytes
                    || self.ttl.is_zero()
                {
                    self.forget(&key, &slot);
                }
                answer.clone().into_response()
            }
            Outcome::Busy => {
                self.forget(&key, &slot);
                busy_response(surface)
            }
        }
    }

    /// The slot for `key`: a fresh or running one if there is one, else a new one.
    fn slot_for(&self, key: &Key) -> Arc<Slot> {
        let mut slots = self
            .slots
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(slot) = slots.get(key).filter(|slot| !self.is_stale(slot)) {
            return slot.clone();
        }
        let slot = Arc::new(Slot {
            cell: OnceCell::new(),
        });
        slots.insert(key.clone(), slot.clone());
        if slots.len() > self.max_entries {
            // Make room: drop stale answers first, then the oldest finished one. A read in flight is never dropped.
            slots.retain(|_, s| !self.is_stale(s));
            while slots.len() > self.max_entries {
                let oldest = slots
                    .iter()
                    .filter_map(|(k, s)| s.cell.get().map(|(_, at)| (k.clone(), *at)))
                    .min_by_key(|(_, at)| *at)
                    .map(|(k, _)| k);
                match oldest {
                    Some(k) => slots.remove(&k),
                    None => break,
                };
            }
        }
        slot
    }

    fn is_stale(&self, slot: &Slot) -> bool {
        slot.cell
            .get()
            .is_some_and(|(_, computed)| computed.elapsed() >= self.ttl)
    }

    fn forget(&self, key: &Key, slot: &Arc<Slot>) {
        let mut slots = self
            .slots
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if slots.get(key).is_some_and(|s| Arc::ptr_eq(s, slot)) {
            slots.remove(key);
        }
    }

    async fn wait_unpaused(&self) {
        let mut paused = self.paused.subscribe();
        let _ = paused.wait_for(|p| !*p).await;
    }

    /// Answers kept (finished or running), for tests.
    #[doc(hidden)]
    #[must_use]
    pub fn cache_len(&self) -> usize {
        self.slots
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .len()
    }

    /// Computations that got a gate and started, for tests.
    #[doc(hidden)]
    #[must_use]
    pub fn computations(&self) -> u64 {
        self.computations.load(Ordering::Relaxed)
    }

    /// Takes a surface's gate until the permit is dropped, so a test can see a busy answer. Fails if it is taken.
    ///
    /// # Errors
    /// The gate is already taken.
    #[doc(hidden)]
    pub fn hold(
        &self,
        surface: Surface,
    ) -> Result<OwnedSemaphorePermit, tokio::sync::TryAcquireError> {
        self.gate(surface).try_acquire_owned()
    }

    /// Whether a surface's gate is taken, for tests: looking does not take it, so it cannot steal it from a read.
    #[doc(hidden)]
    #[must_use]
    pub fn gate_taken(&self, surface: Surface) -> bool {
        self.gate(surface).available_permits() == 0
    }

    /// Makes every computation that has a gate wait until the guard is dropped, so a test can have a read in flight.
    #[doc(hidden)]
    #[must_use]
    pub fn pause(&self) -> Pause {
        self.paused.send_replace(true);
        Pause(self.paused.clone())
    }
}

/// While alive, computations wait before reading; dropping it lets them go.
#[doc(hidden)]
pub struct Pause(Arc<watch::Sender<bool>>);

impl Drop for Pause {
    fn drop(&mut self) {
        self.0.send_replace(false);
    }
}

fn busy_response(surface: Surface) -> Response {
    let mut response = (
        StatusCode::SERVICE_UNAVAILABLE,
        axum::Json(json!({"error": {
            "message": "another statistics read is running; retry in a few seconds",
            "type": "overloaded_error",
            "code": surface.busy_code(),
        }})),
    )
        .into_response();
    response.headers_mut().insert(
        header::RETRY_AFTER,
        HeaderValue::from_static(RETRY_AFTER_SECS),
    );
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect()
    }

    fn ok(body: &str) -> std::future::Ready<Result<Response, Busy>> {
        std::future::ready(Ok(axum::Json(json!({"body": body})).into_response()))
    }

    async fn text(response: Response) -> String {
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        String::from_utf8(bytes.to_vec()).unwrap()
    }

    #[tokio::test]
    async fn an_identical_read_within_the_ttl_is_answered_from_the_first() {
        let heavy = HeavyReads::default();
        let p = params(&[("window", "24h")]);
        let first = heavy
            .run(Surface::Admin, "stats", None, &p, ok("one"))
            .await;
        let second = heavy
            .run(Surface::Admin, "stats", None, &p, ok("two"))
            .await;
        assert_eq!(text(first).await, text(second).await);
        assert_eq!(heavy.computations(), 1);
    }

    #[tokio::test]
    async fn reads_that_differ_are_computed_separately() {
        let heavy = HeavyReads::default();
        let a = heavy
            .run(
                Surface::Admin,
                "stats",
                None,
                &params(&[("window", "1h")]),
                ok("a"),
            )
            .await;
        let b = heavy
            .run(
                Surface::Admin,
                "stats",
                None,
                &params(&[("window", "2h")]),
                ok("b"),
            )
            .await;
        let c = heavy
            .run(
                Surface::Admin,
                "stats",
                Some("alice"),
                &params(&[("window", "1h")]),
                ok("c"),
            )
            .await;
        let d = heavy
            .run(
                Surface::Admin,
                "switches",
                None,
                &params(&[("window", "1h")]),
                ok("d"),
            )
            .await;
        assert_eq!(heavy.computations(), 4);
        let bodies = [text(a).await, text(b).await, text(c).await, text(d).await];
        assert!(bodies[0].contains("\"a\"") && bodies[1].contains("\"b\""));
        assert!(bodies[2].contains("\"c\"") && bodies[3].contains("\"d\""));
    }

    #[tokio::test]
    async fn the_order_of_the_query_parameters_does_not_matter() {
        let heavy = HeavyReads::default();
        let one = params(&[("window", "1h"), ("group_by", "target")]);
        let two = params(&[("group_by", "target"), ("window", "1h")]);
        heavy
            .run(Surface::Admin, "stats", None, &one, ok("x"))
            .await;
        heavy
            .run(Surface::Admin, "stats", None, &two, ok("y"))
            .await;
        assert_eq!(heavy.computations(), 1);
    }

    #[tokio::test]
    async fn a_zero_ttl_computes_every_read_and_keeps_nothing() {
        let heavy = HeavyReads::with_ttl(Duration::ZERO);
        let p = params(&[]);
        heavy.run(Surface::Admin, "stats", None, &p, ok("1")).await;
        heavy.run(Surface::Admin, "stats", None, &p, ok("2")).await;
        assert_eq!(heavy.computations(), 2);
        assert_eq!(heavy.cache_len(), 0);
    }

    #[tokio::test]
    async fn a_read_is_busy_while_the_gate_is_held_and_the_other_surface_is_not() {
        let heavy = HeavyReads::default();
        let _held = heavy.hold(Surface::Client).unwrap();
        let busy = heavy
            .run(Surface::Client, "stats", Some("k"), &params(&[]), ok("x"))
            .await;
        assert_eq!(busy.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(busy.headers()[header::RETRY_AFTER], "5");
        assert!(text(busy).await.contains("stats_busy"));
        let admin = heavy
            .run(Surface::Admin, "stats", None, &params(&[]), ok("y"))
            .await;
        assert_eq!(admin.status(), StatusCode::OK);
        assert_eq!(heavy.cache_len(), 1, "the busy answer was not kept");
    }

    #[tokio::test]
    async fn a_busy_answer_is_not_remembered() {
        let heavy = HeavyReads::default();
        let p = params(&[]);
        {
            let _held = heavy.hold(Surface::Admin).unwrap();
            let busy = heavy.run(Surface::Admin, "stats", None, &p, ok("x")).await;
            assert_eq!(busy.status(), StatusCode::SERVICE_UNAVAILABLE);
            assert!(text(busy).await.contains("admin_busy"));
        }
        let now = heavy.run(Surface::Admin, "stats", None, &p, ok("z")).await;
        assert_eq!(now.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn a_ledger_that_was_busy_is_a_503_and_is_not_kept() {
        let heavy = HeavyReads::default();
        let p = params(&[]);
        let busy = heavy
            .run(Surface::Admin, "stats", None, &p, async { Err(Busy) })
            .await;
        assert_eq!(busy.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(busy.headers()[header::RETRY_AFTER], "5");
        assert_eq!(heavy.cache_len(), 0);
        let next = heavy
            .run(Surface::Admin, "stats", None, &p, ok("fine"))
            .await;
        assert_eq!(next.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn an_error_answer_is_returned_but_not_kept() {
        let heavy = HeavyReads::default();
        let p = params(&[]);
        let bad = heavy
            .run(Surface::Admin, "stats", None, &p, async {
                Ok((StatusCode::BAD_REQUEST, "no").into_response())
            })
            .await;
        assert_eq!(bad.status(), StatusCode::BAD_REQUEST);
        assert_eq!(heavy.cache_len(), 0);
    }

    #[tokio::test]
    async fn identical_reads_at_the_same_moment_share_one_computation() {
        let heavy = Arc::new(HeavyReads::default());
        let pause = heavy.pause();
        let p = params(&[("window", "24h")]);
        let readers: Vec<_> = (0..5)
            .map(|i| {
                let (heavy, p) = (heavy.clone(), p.clone());
                tokio::spawn(async move {
                    heavy
                        .run(
                            Surface::Admin,
                            "stats",
                            None,
                            &p,
                            ok(&format!("reader {i}")),
                        )
                        .await
                })
            })
            .collect();
        // Let all five arrive while the first computation waits, then let it go.
        tokio::time::sleep(Duration::from_millis(100)).await;
        drop(pause);
        let mut bodies = Vec::new();
        for reader in readers {
            let response = reader.await.unwrap();
            assert_eq!(response.status(), StatusCode::OK, "none is refused as busy");
            bodies.push(text(response).await);
        }
        assert_eq!(heavy.computations(), 1);
        assert!(bodies.windows(2).all(|w| w[0] == w[1]), "{bodies:?}");
    }

    #[tokio::test]
    async fn a_different_read_while_one_runs_is_shed_not_queued() {
        let heavy = Arc::new(HeavyReads::default());
        let pause = heavy.pause();
        let running = {
            let heavy = heavy.clone();
            tokio::spawn(async move {
                heavy
                    .run(
                        Surface::Admin,
                        "stats",
                        None,
                        &params(&[("window", "1h")]),
                        ok("slow"),
                    )
                    .await
            })
        };
        tokio::time::sleep(Duration::from_millis(100)).await;
        let started = Instant::now();
        let shed = heavy
            .run(
                Surface::Admin,
                "stats",
                None,
                &params(&[("window", "2h")]),
                ok("other"),
            )
            .await;
        assert_eq!(shed.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "it must not wait"
        );
        drop(pause);
        assert_eq!(running.await.unwrap().status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn the_cache_keeps_at_most_its_bound_of_answers() {
        let heavy = HeavyReads::with_limits(Duration::from_secs(60), 4, 1024);
        for i in 0..10 {
            let w = format!("{i}h");
            heavy
                .run(
                    Surface::Admin,
                    "stats",
                    None,
                    &params(&[("window", &w)]),
                    ok("x"),
                )
                .await;
        }
        assert_eq!(heavy.cache_len(), 4);
        assert_eq!(heavy.computations(), 10);
    }

    #[tokio::test]
    async fn an_answer_over_the_size_bound_is_returned_but_not_kept() {
        let heavy = HeavyReads::with_limits(Duration::from_secs(60), 16, 10);
        let p = params(&[]);
        let big = "x".repeat(100);
        let first = heavy.run(Surface::Admin, "stats", None, &p, ok(&big)).await;
        assert_eq!(first.status(), StatusCode::OK);
        assert_eq!(heavy.cache_len(), 0);
        heavy.run(Surface::Admin, "stats", None, &p, ok(&big)).await;
        assert_eq!(heavy.computations(), 2);
    }

    #[tokio::test]
    async fn an_answer_expires_after_the_ttl() {
        let heavy = HeavyReads::with_ttl(Duration::from_millis(50));
        let p = params(&[]);
        heavy.run(Surface::Admin, "stats", None, &p, ok("1")).await;
        tokio::time::sleep(Duration::from_millis(80)).await;
        heavy.run(Surface::Admin, "stats", None, &p, ok("2")).await;
        assert_eq!(heavy.computations(), 2);
    }
}
