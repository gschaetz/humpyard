//! Per-key budgets: live counters for the current UTC day and month, recovered from the ledger at
//! startup, and the routing policy that degrades or restricts traffic as a key nears its limits.

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::Mutex;

use crate::clock::{Clock, day_start_ms, month_start_ms};
use crate::config::{BudgetConfig, Config, Limits};
use crate::ledger::{Ledger, LedgerError, Spend};
use crate::num::f64_from_u64;
use crate::policy::{BudgetState, PolicyContext, RoutingPolicy};

#[derive(Clone, Copy, Debug, Default)]
struct KeyCounters {
    day_start_ms: i64,
    month_start_ms: i64,
    day: Spend,
    month: Spend,
}

/// A key's spend and limits at one instant.
#[derive(Clone, Debug)]
pub struct BudgetStatus {
    pub state: BudgetState,
    /// The limit that decided the state (the one closest to, or past, its cap).
    pub binding_limit: Option<&'static str>,
    pub day: Spend,
    pub month: Spend,
    pub limits: Limits,
}

pub struct BudgetTracker {
    clock: Arc<dyn Clock>,
    restricted_at: f64,
    counters: Mutex<HashMap<String, KeyCounters>>,
}

impl BudgetTracker {
    pub fn new(clock: Arc<dyn Clock>, budget: &BudgetConfig) -> Self {
        Self {
            clock,
            restricted_at: budget.restricted_at,
            counters: Mutex::new(HashMap::new()),
        }
    }

    /// Loads the current day's and month's spend per key from the ledger.
    pub async fn hydrate(&self, ledger: &Ledger) -> Result<(), LedgerError> {
        let now = self.clock.now_ms();
        let (day_start, month_start) = (day_start_ms(now), month_start_ms(now));
        let day = ledger.spend_since(day_start).await?;
        let month = ledger.spend_since(month_start).await?;
        let mut counters = self.counters.lock();
        for key in month.keys().chain(day.keys()) {
            counters.insert(
                key.clone(),
                KeyCounters {
                    day_start_ms: day_start,
                    month_start_ms: month_start,
                    day: day.get(key).copied().unwrap_or_default(),
                    month: month.get(key).copied().unwrap_or_default(),
                },
            );
        }
        Ok(())
    }

    fn rolled<'a>(
        counters: &'a mut HashMap<String, KeyCounters>,
        key: &str,
        now: i64,
    ) -> &'a mut KeyCounters {
        let (day_start, month_start) = (day_start_ms(now), month_start_ms(now));
        let entry = counters.entry(key.to_string()).or_insert(KeyCounters {
            day_start_ms: day_start,
            month_start_ms: month_start,
            ..KeyCounters::default()
        });
        if entry.day_start_ms != day_start {
            entry.day_start_ms = day_start;
            entry.day = Spend::default();
        }
        if entry.month_start_ms != month_start {
            entry.month_start_ms = month_start;
            entry.month = Spend::default();
        }
        entry
    }

    /// Adds a finished call's usage; the next request sees it.
    pub fn record(&self, key: &str, micro_usd: u64, tokens: u64) {
        let now = self.clock.now_ms();
        let mut counters = self.counters.lock();
        let entry = Self::rolled(&mut counters, key, now);
        for spend in [&mut entry.day, &mut entry.month] {
            spend.micro_usd += micro_usd;
            spend.tokens += tokens;
        }
    }

    pub fn status(&self, key: &str, limits: &Limits) -> BudgetStatus {
        let now = self.clock.now_ms();
        let mut counters = self.counters.lock();
        let entry = *Self::rolled(&mut counters, key, now);
        let usd = |s: Spend| f64_from_u64(s.micro_usd) / 1_000_000.0;
        let checks: [(&'static str, f64, Option<f64>); 4] = [
            ("daily_usd", usd(entry.day), limits.daily_usd),
            ("monthly_usd", usd(entry.month), limits.monthly_usd),
            (
                "daily_tokens",
                f64_from_u64(entry.day.tokens),
                limits.daily_tokens.map(f64_from_u64),
            ),
            (
                "monthly_tokens",
                f64_from_u64(entry.month.tokens),
                limits.monthly_tokens.map(f64_from_u64),
            ),
        ];
        let mut worst: Option<(&'static str, f64)> = None;
        for (name, spent, limit) in checks {
            let Some(limit) = limit else { continue };
            // A zero limit means "nothing allowed".
            let ratio = if limit <= 0.0 {
                f64::INFINITY
            } else {
                spent / limit
            };
            if worst.is_none_or(|(_, r)| ratio > r) {
                worst = Some((name, ratio));
            }
        }
        let state = match worst {
            Some((_, ratio)) if ratio >= 1.0 => BudgetState::Exhausted,
            Some((_, ratio)) if ratio >= self.restricted_at => BudgetState::Restricted,
            _ => BudgetState::Healthy,
        };
        BudgetStatus {
            state,
            binding_limit: worst.map(|(name, _)| name),
            day: entry.day,
            month: entry.month,
            limits: *limits,
        }
    }
}

/// What the budget policy needs to know about a target's cost.
#[derive(Clone, Copy, Debug)]
struct TargetCost {
    /// Highest output price among the target's endpoints (conservative: one expensive failover
    /// endpoint makes the whole target expensive).
    max_output: f64,
    /// Every endpoint is priced zero.
    free: bool,
}

/// Restricts targets by price according to the key's budget state.
pub struct BudgetPolicy {
    costs: HashMap<String, TargetCost>,
    ceiling: Option<f64>,
}

impl BudgetPolicy {
    pub fn new(config: &Config) -> Self {
        let costs = config
            .targets
            .iter()
            .map(|(name, endpoints)| {
                let prices = endpoints
                    .iter()
                    .map(|e| e.price.unwrap_or(crate::config::Price::FREE));
                let max_output = prices.clone().map(|p| p.output).fold(0.0, f64::max);
                let free = prices.clone().all(|p| p.is_free());
                (name.clone(), TargetCost { max_output, free })
            })
            .collect();
        Self {
            costs,
            ceiling: config.budget.restricted_max_output_price,
        }
    }
}

impl RoutingPolicy for BudgetPolicy {
    fn is_eligible(&self, context: &PolicyContext<'_>, target: &str) -> bool {
        let Some(key) = &context.key else { return true };
        let Some(cost) = self.costs.get(target) else {
            return true;
        };
        match key.budget {
            BudgetState::Healthy => true,
            BudgetState::Restricted => self.ceiling.is_none_or(|c| cost.max_output <= c),
            // Only reached for keys that continue on free targets after their limit.
            BudgetState::Exhausted => cost.free,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicI64, Ordering};

    struct TestClock(AtomicI64);
    impl Clock for TestClock {
        fn now_ms(&self) -> i64 {
            self.0.load(Ordering::SeqCst)
        }
    }

    const DAY: i64 = 86_400_000;

    fn tracker(now: i64, restricted_at: f64) -> (Arc<TestClock>, BudgetTracker) {
        let clock = Arc::new(TestClock(AtomicI64::new(now)));
        let budget = BudgetConfig {
            restricted_at,
            restricted_max_output_price: None,
        };
        (clock.clone(), BudgetTracker::new(clock, &budget))
    }

    fn usd(daily: f64) -> Limits {
        Limits {
            daily_usd: Some(daily),
            ..Limits::default()
        }
    }

    #[test]
    fn state_moves_from_healthy_to_restricted_to_exhausted() {
        let (_, t) = tracker(10 * DAY, 0.8);
        let limits = usd(1.0);
        assert_eq!(t.status("a", &limits).state, BudgetState::Healthy);
        t.record("a", 790_000, 0);
        assert_eq!(t.status("a", &limits).state, BudgetState::Healthy);
        t.record("a", 10_000, 0);
        assert_eq!(t.status("a", &limits).state, BudgetState::Restricted);
        t.record("a", 200_000, 0);
        let status = t.status("a", &limits);
        assert_eq!(status.state, BudgetState::Exhausted);
        assert_eq!(status.binding_limit, Some("daily_usd"));
    }

    #[test]
    fn the_closest_limit_decides() {
        let (_, t) = tracker(10 * DAY, 0.8);
        let limits = Limits {
            daily_usd: Some(100.0),
            monthly_tokens: Some(100),
            ..Limits::default()
        };
        t.record("a", 1_000_000, 95);
        let status = t.status("a", &limits);
        assert_eq!(status.state, BudgetState::Restricted);
        assert_eq!(status.binding_limit, Some("monthly_tokens"));
    }

    #[test]
    fn unlimited_keys_are_always_healthy_and_zero_limits_block() {
        let (_, t) = tracker(10 * DAY, 0.8);
        t.record("a", 999_999_999, 999_999);
        assert_eq!(
            t.status("a", &Limits::default()).state,
            BudgetState::Healthy
        );
        assert_eq!(t.status("a", &usd(0.0)).state, BudgetState::Exhausted);
    }

    #[test]
    fn daily_spend_resets_at_utc_midnight_while_monthly_continues() {
        let start = 1_791_000_000_000_i64;
        let (clock, t) = tracker(start, 0.8);
        let limits = Limits {
            daily_usd: Some(1.0),
            monthly_usd: Some(50.0),
            ..Limits::default()
        };
        t.record("a", 1_000_000, 10);
        assert_eq!(t.status("a", &limits).state, BudgetState::Exhausted);
        clock
            .0
            .store(day_start_ms(start) + DAY + 1, Ordering::SeqCst);
        let status = t.status("a", &limits);
        assert_eq!(status.state, BudgetState::Healthy);
        assert_eq!(status.day, Spend::default());
        assert_eq!(
            status.month.micro_usd, 1_000_000,
            "the month carries over a day boundary"
        );
    }

    #[test]
    fn monthly_spend_resets_at_the_month_boundary() {
        let start = 1_791_000_000_000_i64;
        let (clock, t) = tracker(start, 0.8);
        t.record("a", 5, 5);
        clock
            .0
            .store(month_start_ms(start + 40 * DAY), Ordering::SeqCst);
        let status = t.status("a", &Limits::default());
        assert_eq!(status.month, Spend::default());
    }

    #[tokio::test]
    async fn hydration_restores_spend_from_the_ledger() {
        use crate::ledger::{Entry, Kind};
        let now = 1_791_000_000_000_i64;
        let dir = tempfile::tempdir().unwrap();
        let ledger = Ledger::open(&dir.path().join("l.db")).await.unwrap();
        let base = |ts_ms: i64, cost: u64| Entry {
            ts_ms,
            key_id: Some("a".into()),
            session_id: None,
            route: "r".into(),
            target: "t".into(),
            provider: "p".into(),
            model: "m".into(),
            kind: Kind::Answer,
            input_tokens: 10,
            cached_input_tokens: 0,
            cache_creation_tokens: 0,
            output_tokens: 0,
            reasoning_tokens: 0,
            cost_micro_usd: cost,
            outcome: "ok".into(),
            usage_missing: false,
        };
        ledger.record(base(day_start_ms(now) + 5, 400_000));
        ledger.record(base(month_start_ms(now) + 5, 100_000));
        ledger.record(base(month_start_ms(now) - 5, 9_000_000));
        ledger.shutdown().await;

        let ledger = Ledger::open(&dir.path().join("l.db")).await.unwrap();
        let (_, t) = tracker(now, 0.8);
        t.hydrate(&ledger).await.unwrap();
        let status = t.status("a", &usd(10.0));
        assert_eq!(status.day.micro_usd, 400_000);
        assert_eq!(status.month.micro_usd, 500_000);
    }
}
