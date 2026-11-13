//! Per-key request-rate and concurrency limits (`add-key-rate-limits`).
//!
//! A key may be limited to `max_requests_per_minute` and to `max_concurrent` requests in flight. The
//! rate is an exact sliding window: the admission times of the last N requests, admitting a request only
//! when the one N places back is at least 60 seconds old. The concurrency limit is a [`Permit`] whose
//! `Drop` returns it, so whatever ends a request (an answer, an error, a cancelled stream, a panic
//! unwinding) gives the permit back.
//!
//! State is one entry per configured key, built from the configuration; nothing is keyed by what a caller
//! sends, so it cannot grow with traffic. A reload keeps the counters of keys that stay limited (and the
//! permits in flight), applies changed limits from the next request and drops the rest. Counters live in
//! memory, per process: a restart clears them and each replica
//! enforces its own limit. The check is the caller's to place after authentication,
//! so only a key's holder can spend its allowance.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use parking_lot::Mutex;

use crate::clock::Clock;
use crate::config::KeyConfig;

/// The length of the rate window.
const WINDOW_MS: i64 = 60_000;

/// Why a request was refused, with what a client needs to retry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// The key has already made `limit` requests in the last 60 seconds.
    Rate { limit: u32, retry_after_secs: u64 },
    /// The key already has `limit` requests in flight.
    Concurrent { limit: u32 },
}

impl Refusal {
    /// Whole seconds until a retry can succeed; a concurrency refusal says one second, since a
    /// permit may come back at any moment.
    pub fn retry_after_secs(&self) -> u64 {
        match self {
            Self::Rate {
                retry_after_secs, ..
            } => *retry_after_secs,
            Self::Concurrent { .. } => 1,
        }
    }
}

/// Requests of one key in flight. Shared with every [`Permit`] it has handed out.
#[derive(Debug)]
struct InFlight {
    /// Changed in place by a reload, so the permits already handed out keep counting against it.
    limit: AtomicU32,
    count: AtomicU32,
}

/// One request's place under a key's concurrency limit, returned when it is dropped.
#[derive(Debug)]
pub struct Permit {
    in_flight: Arc<InFlight>,
}

impl Drop for Permit {
    fn drop(&mut self) {
        // Saturating, so a count that is somehow already zero cannot wrap around and lock the key out.
        adjust(&self.in_flight.count, |n| Some(n.saturating_sub(1)));
    }
}

/// Applies `change` to a counter atomically; `None` from it leaves the counter alone. Returns whether it
/// changed. (A compare-exchange loop: `fetch_update` is deprecated and its replacement is newer than
/// the oldest Rust this builds on.)
fn adjust(counter: &AtomicU32, change: impl Fn(u32) -> Option<u32>) -> bool {
    let mut seen = counter.load(Ordering::Acquire);
    loop {
        let Some(next) = change(seen) else {
            return false;
        };
        match counter.compare_exchange_weak(seen, next, Ordering::AcqRel, Ordering::Acquire) {
            Ok(_) => return true,
            Err(actual) => seen = actual,
        }
    }
}

/// The admission times of a key's last requests, oldest first.
#[derive(Debug)]
struct Window {
    limit: u32,
    /// Never more than `limit` entries once a request has been judged.
    admitted: VecDeque<i64>,
    /// The latest time seen, so a clock that moves backwards counts as no time having passed.
    latest_ms: i64,
}

impl Window {
    /// Admits a request at `now_ms`, or says how many whole seconds until one can be.
    fn admit(&mut self, now_ms: i64) -> Result<(), u64> {
        self.latest_ms = self.latest_ms.max(now_ms);
        let now = self.latest_ms;
        while self
            .admitted
            .front()
            .is_some_and(|&at| now.saturating_sub(at) >= WINDOW_MS)
        {
            self.admitted.pop_front();
        }
        let limit = usize::try_from(self.limit).unwrap_or(usize::MAX);
        if self.admitted.len() < limit {
            self.admitted.push_back(now);
            return Ok(());
        }
        // The entry that must leave the window before this request fits: the oldest when the window is
        // exactly full, an older-numbered one if the limit was lowered under a fuller window.
        let blocking = self
            .admitted
            .get(self.admitted.len().saturating_sub(limit))
            .copied()
            .unwrap_or(now);
        let remaining_ms = WINDOW_MS.saturating_sub(now.saturating_sub(blocking));
        let secs = remaining_ms.saturating_add(999) / 1000;
        Err(u64::try_from(secs).unwrap_or(1).max(1))
    }
}

#[derive(Debug)]
struct Entry {
    rate: Option<Arc<Mutex<Window>>>,
    concurrent: Option<Arc<InFlight>>,
}

/// The limits of every configured key.
pub struct Limiters {
    clock: Arc<dyn Clock>,
    by_key: HashMap<String, Entry>,
}

impl Limiters {
    pub fn new(clock: Arc<dyn Clock>, keys: &BTreeMap<String, KeyConfig>) -> Self {
        Self::build(clock, keys, None)
    }

    /// The limiters for a reloaded configuration. A key that is still limited keeps its counters, so a
    /// reload neither resets a window nor loses a request in flight, and takes its new limits from now
    /// on; a key that is gone, or no longer limited, is dropped. Permits already handed out still return
    /// to the counter they came from.
    #[must_use]
    pub fn reloaded(&self, keys: &BTreeMap<String, KeyConfig>) -> Self {
        Self::build(self.clock.clone(), keys, Some(self))
    }

    fn build(
        clock: Arc<dyn Clock>,
        keys: &BTreeMap<String, KeyConfig>,
        previous: Option<&Self>,
    ) -> Self {
        let by_key = keys
            .iter()
            .filter(|(_, key)| {
                key.max_requests_per_minute.is_some() || key.max_concurrent.is_some()
            })
            .map(|(id, key)| {
                let before = previous.and_then(|p| p.by_key.get(id));
                let entry = Entry {
                    rate: key
                        .max_requests_per_minute
                        .map(|limit| carry_window(before, limit)),
                    concurrent: key
                        .max_concurrent
                        .map(|limit| carry_in_flight(before, limit)),
                };
                (id.clone(), entry)
            })
            .collect();
        Self { clock, by_key }
    }

    /// Admits one request of `key`, or refuses it. A key with no limit, and a key this gateway does
    /// not know, are admitted without any state being kept. The returned permit, if any, must be held
    /// until the response has ended.
    pub fn admit(&self, key: &str) -> Result<Option<Permit>, Refusal> {
        let Some(entry) = self.by_key.get(key) else {
            return Ok(None);
        };
        // Concurrency first: a request refused for it has not used a place in the rate window. If the
        // rate then refuses, dropping the permit gives its place back.
        let permit = match &entry.concurrent {
            Some(in_flight) => Some(take(in_flight)?),
            None => None,
        };
        self.admit_in_window(entry)?;
        Ok(permit)
    }

    fn admit_in_window(&self, entry: &Entry) -> Result<(), Refusal> {
        let Some(rate) = &entry.rate else {
            return Ok(());
        };
        let mut window = rate.lock();
        window
            .admit(self.clock.now_ms())
            .map_err(|retry_after_secs| Refusal::Rate {
                limit: window.limit,
                retry_after_secs,
            })
    }

    /// Requests of `key` in flight now; zero for a key without a concurrency limit.
    pub fn in_flight(&self, key: &str) -> u32 {
        self.by_key
            .get(key)
            .and_then(|entry| entry.concurrent.as_ref())
            .map_or(0, |in_flight| in_flight.count.load(Ordering::Acquire))
    }
}

/// The key's window from before a reload at its new limit, or a fresh one.
fn carry_window(before: Option<&Entry>, limit: u32) -> Arc<Mutex<Window>> {
    if let Some(window) = before.and_then(|e| e.rate.as_ref()) {
        window.lock().limit = limit;
        return Arc::clone(window);
    }
    Arc::new(Mutex::new(Window {
        limit,
        admitted: VecDeque::new(),
        latest_ms: i64::MIN,
    }))
}

/// The key's in-flight counter from before a reload at its new limit, or a fresh one.
fn carry_in_flight(before: Option<&Entry>, limit: u32) -> Arc<InFlight> {
    if let Some(in_flight) = before.and_then(|e| e.concurrent.as_ref()) {
        in_flight.limit.store(limit, Ordering::Release);
        return Arc::clone(in_flight);
    }
    Arc::new(InFlight {
        limit: AtomicU32::new(limit),
        count: AtomicU32::new(0),
    })
}

/// Takes a place under the limit, refusing when none is left.
fn take(in_flight: &Arc<InFlight>) -> Result<Permit, Refusal> {
    let limit = in_flight.limit.load(Ordering::Acquire);
    if adjust(&in_flight.count, |n| {
        (n < limit).then(|| n.saturating_add(1))
    }) {
        Ok(Permit {
            in_flight: Arc::clone(in_flight),
        })
    } else {
        Err(Refusal::Concurrent { limit })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicI64;

    use super::*;

    struct TestClock(AtomicI64);

    impl TestClock {
        fn at(ms: i64) -> Arc<Self> {
            Arc::new(Self(AtomicI64::new(ms)))
        }

        fn set(&self, ms: i64) {
            self.0.store(ms, Ordering::SeqCst);
        }

        fn advance(&self, ms: i64) {
            self.0.fetch_add(ms, Ordering::SeqCst);
        }
    }

    impl Clock for TestClock {
        fn now_ms(&self) -> i64 {
            self.0.load(Ordering::SeqCst)
        }
    }

    fn key(rate: Option<u32>, concurrent: Option<u32>) -> KeyConfig {
        KeyConfig {
            sha256: [0; 32],
            allowed_routes: None,
            over_budget: crate::config::OverBudget::Block,
            limits: crate::config::Limits::default(),
            max_request_usd: None,
            max_requests_per_minute: rate,
            max_concurrent: concurrent,
            read_config: false,
            manage_overrides: false,
            profile: None,
            actor: None,
            owner: None,
        }
    }

    fn limiters(clock: &Arc<TestClock>, keys: &[(&str, Option<u32>, Option<u32>)]) -> Limiters {
        let keys = keys
            .iter()
            .map(|(id, rate, concurrent)| ((*id).to_string(), key(*rate, *concurrent)))
            .collect();
        Limiters::new(clock.clone(), &keys)
    }

    #[test]
    fn exactly_n_are_admitted_in_a_window_and_the_next_is_refused() {
        let clock = TestClock::at(1_000_000);
        let limiters = limiters(&clock, &[("a", Some(3), None)]);
        for _ in 0..3 {
            assert!(limiters.admit("a").is_ok());
        }
        assert_eq!(
            limiters.admit("a").unwrap_err(),
            Refusal::Rate {
                limit: 3,
                retry_after_secs: 60
            }
        );
    }

    #[test]
    fn the_window_slides_and_a_request_exactly_sixty_seconds_after_the_oldest_is_admitted() {
        let clock = TestClock::at(1_000_000);
        let limiters = limiters(&clock, &[("a", Some(2), None)]);
        assert!(limiters.admit("a").is_ok()); // t = 0
        clock.advance(10_000);
        assert!(limiters.admit("a").is_ok()); // t = 10 s
        clock.advance(49_999); // t = 59.999 s: the oldest is not yet 60 s old
        assert!(limiters.admit("a").is_err());
        clock.advance(1); // t = 60 s
        assert!(limiters.admit("a").is_ok());
        // The second request (t = 10 s) is now the oldest, and the third (t = 60 s) is in the window.
        assert!(limiters.admit("a").is_err());
        clock.advance(10_000); // t = 70 s
        assert!(limiters.admit("a").is_ok());
    }

    #[test]
    fn retry_after_is_the_remaining_time_of_the_oldest_rounded_up() {
        let clock = TestClock::at(5_000_000);
        let limiters = limiters(&clock, &[("a", Some(1), None)]);
        assert!(limiters.admit("a").is_ok());
        clock.advance(20_400); // 39.6 s remain
        assert_eq!(limiters.admit("a").unwrap_err().retry_after_secs(), 40);
        clock.advance(38_600); // 1.0 s remains
        assert_eq!(limiters.admit("a").unwrap_err().retry_after_secs(), 1);
        clock.advance(999); // 1 ms remains: still a whole second, never zero
        assert_eq!(limiters.admit("a").unwrap_err().retry_after_secs(), 1);
    }

    #[test]
    fn refused_requests_do_not_use_the_window() {
        let clock = TestClock::at(1_000_000);
        let limiters = limiters(&clock, &[("a", Some(1), None)]);
        assert!(limiters.admit("a").is_ok());
        for _ in 0..100 {
            clock.advance(100);
            assert!(limiters.admit("a").is_err());
        }
        clock.set(1_000_000 + WINDOW_MS);
        assert!(limiters.admit("a").is_ok());
    }

    #[test]
    fn keys_are_independent_and_unlimited_or_unknown_keys_are_always_admitted() {
        let clock = TestClock::at(1_000_000);
        let limiters = limiters(&clock, &[("a", Some(1), None), ("b", Some(1), None)]);
        assert!(limiters.admit("a").is_ok());
        assert!(limiters.admit("a").is_err());
        assert!(limiters.admit("b").is_ok());
        for _ in 0..1000 {
            assert!(limiters.admit("no-limit").is_ok());
            assert!(limiters.admit("unknown").is_ok());
        }
    }

    #[test]
    fn a_permit_is_held_until_it_is_dropped_and_then_returned() {
        let clock = TestClock::at(0);
        let limiters = limiters(&clock, &[("a", None, Some(2))]);
        let first = limiters.admit("a").unwrap();
        let second = limiters.admit("a").unwrap();
        assert!(first.is_some() && second.is_some());
        assert_eq!(limiters.in_flight("a"), 2);
        assert_eq!(
            limiters.admit("a").unwrap_err(),
            Refusal::Concurrent { limit: 2 }
        );
        assert_eq!(limiters.admit("a").unwrap_err().retry_after_secs(), 1);
        drop(first);
        assert_eq!(limiters.in_flight("a"), 1);
        let third = limiters.admit("a").unwrap();
        assert_eq!(limiters.in_flight("a"), 2);
        drop((second, third));
        assert_eq!(limiters.in_flight("a"), 0);
    }

    #[test]
    fn a_permit_is_returned_when_the_holder_panics() {
        let clock = TestClock::at(0);
        let limiters = Arc::new(limiters(&clock, &[("a", None, Some(1))]));
        let held = Arc::clone(&limiters);
        let result = std::thread::spawn(move || {
            let _permit = held.admit("a").unwrap();
            panic!("the request handler failed");
        })
        .join();
        assert!(result.is_err());
        assert_eq!(limiters.in_flight("a"), 0);
        assert!(limiters.admit("a").is_ok());
    }

    #[test]
    fn a_request_refused_for_its_rate_gives_its_permit_back_and_one_refused_for_concurrency_uses_no_rate()
     {
        let clock = TestClock::at(1_000_000);
        let limiters = limiters(&clock, &[("a", Some(2), Some(1))]);
        let held = limiters.admit("a").unwrap(); // 1 of 2 in the window, the one permit taken
        // Refused for concurrency: the window must not have been used.
        assert_eq!(
            limiters.admit("a").unwrap_err(),
            Refusal::Concurrent { limit: 1 }
        );
        drop(held);
        let second = limiters.admit("a").unwrap(); // 2 of 2
        drop(second);
        // Refused for its rate: the permit it took is returned.
        assert!(matches!(
            limiters.admit("a").unwrap_err(),
            Refusal::Rate { limit: 2, .. }
        ));
        assert_eq!(limiters.in_flight("a"), 0);
    }

    #[test]
    fn a_clock_that_moves_backwards_counts_as_no_time_having_passed() {
        let clock = TestClock::at(1_000_000);
        let limiters = limiters(&clock, &[("a", Some(2), None)]);
        assert!(limiters.admit("a").is_ok());
        assert!(limiters.admit("a").is_ok());
        // An hour back: a naive subtraction would be negative and the window would never empty or
        // would admit as if nothing had been used.
        clock.set(1_000_000 - 3_600_000);
        assert!(limiters.admit("a").is_err());
        // Time then resumes; the window empties 60 s after the highest time seen, not before.
        clock.set(1_000_000 + 59_999);
        assert!(limiters.admit("a").is_err());
        clock.set(1_000_000 + WINDOW_MS);
        assert!(limiters.admit("a").is_ok());
    }

    #[test]
    fn extreme_clock_values_do_not_overflow() {
        let clock = TestClock::at(i64::MAX);
        let limiters = limiters(&clock, &[("a", Some(1), None)]);
        assert!(limiters.admit("a").is_ok());
        assert!(limiters.admit("a").is_err());
        clock.set(i64::MIN);
        assert!(limiters.admit("a").is_err());
    }

    #[test]
    fn state_does_not_grow_with_traffic() {
        let clock = TestClock::at(0);
        let limiters = limiters(&clock, &[("a", Some(5), Some(5))]);
        for i in 0..2000 {
            clock.advance(7);
            let _ = limiters.admit("a");
            let _ = limiters.admit(&format!("stranger-{i}"));
        }
        assert_eq!(limiters.by_key.len(), 1, "no entry per unknown caller");
        let held = limiters.by_key["a"]
            .rate
            .as_ref()
            .unwrap()
            .lock()
            .admitted
            .len();
        assert!(held <= 5, "{held} timestamps kept for a limit of 5");
    }

    #[test]
    fn a_reload_keeps_counters_applies_new_limits_and_drops_removed_keys() {
        let clock = TestClock::at(1_000_000);
        let before = limiters(&clock, &[("a", Some(2), Some(1)), ("gone", Some(1), None)]);
        let held = before.admit("a").unwrap();
        assert!(held.is_some());
        assert!(before.admit("gone").is_ok());
        assert!(before.admit("gone").is_err());
        let keys = [
            ("a".to_string(), key(Some(5), Some(3))),
            ("new".to_string(), key(Some(1), None)),
        ]
        .into_iter()
        .collect();
        let after = before.reloaded(&keys);
        // The window and the in-flight count came across, under the raised limits.
        assert_eq!(after.in_flight("a"), 1);
        let second = after.admit("a").unwrap();
        assert_eq!(after.in_flight("a"), 2);
        // A removed key has no state; a new key starts empty.
        assert!(!after.by_key.contains_key("gone"));
        assert!(after.admit("new").is_ok());
        // The permit taken before the reload returns to the counter the new limiters read.
        drop(held);
        assert_eq!(after.in_flight("a"), 1);
        drop(second);
        assert_eq!(after.in_flight("a"), 0);
    }

    #[test]
    fn a_lowered_limit_applies_from_the_next_request() {
        let clock = TestClock::at(1_000_000);
        let before = limiters(&clock, &[("a", Some(3), Some(3))]);
        let _p = before.admit("a").unwrap();
        let _q = before.admit("a").unwrap();
        let keys = [("a".to_string(), key(Some(2), Some(1)))]
            .into_iter()
            .collect();
        let after = before.reloaded(&keys);
        assert!(matches!(
            after.admit("a"),
            Err(Refusal::Concurrent { limit: 1 })
        ));
    }
}
