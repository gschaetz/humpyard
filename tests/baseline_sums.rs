//! The sums a baseline cost is computed from (`add-baseline-cost`): per group, the answered calls whose provider
//! reported usage and what the calls cost, beside the figures the statistics already had.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test scaffolding

use humpyard::ledger::{Attempt, Decision, Entry, GroupBy, Kind, Ledger, StatsQuery};

const NOW: i64 = 1_800_000_000_000;

fn attempt(target: &str) -> Attempt {
    Attempt {
        target: target.into(),
        provider: "p".into(),
        model: target.into(),
        outcome: "ok".into(),
        ms: 10,
        class: None,
        retry_after_ms: None,
        message: None,
    }
}

fn decision(id: &str, target: &str) -> Decision {
    Decision {
        request_id: id.into(),
        ts_ms: NOW - 1000,
        key_id: Some("k".into()),
        route: Some("r".into()),
        attempts: vec![attempt(target)],
        outcome: "ok".into(),
        status: 200,
        ..Decision::default()
    }
}

/// An answer for `id` on `target` with the given token counts and cost; `usage_missing` zeroes them as the ledger does.
fn answer(
    id: &str,
    target: &str,
    tokens: (u64, u64, u64, u64),
    cost: u64,
    usage_missing: bool,
    kind: Kind,
) -> Entry {
    Entry {
        ts_ms: NOW - 1000,
        key_id: Some("k".into()),
        session_id: None,
        route: "r".into(),
        target: target.into(),
        provider: "p".into(),
        model: target.into(),
        kind,
        input_tokens: tokens.0,
        cached_input_tokens: tokens.1,
        cache_creation_tokens: tokens.2,
        output_tokens: tokens.3,
        reasoning_tokens: 0,
        cost_micro_usd: cost,
        outcome: "ok".into(),
        usage_missing,
        request_id: Some(id.into()),
        signals: None,
    }
}

fn query(group_by: GroupBy) -> StatsQuery {
    StatsQuery {
        key: None,
        since_ms: NOW - 3_600_000,
        group_by,
        route: None,
        target: None,
        row_cap: 1000,
    }
}

/// Target `paid`: two calls with usage (cost 300 and 700) and one with none. Target `free`: one call with usage at no
/// cost. A judge call on `paid` is not an answer and counts nowhere.
async fn seeded(path: &std::path::Path) -> Ledger {
    let ledger = Ledger::open(path).await.unwrap();
    for (id, target) in [("a", "paid"), ("b", "paid"), ("c", "paid"), ("d", "free")] {
        ledger.record_decision(decision(id, target));
    }
    for e in [
        answer("a", "paid", (100, 40, 10, 50), 300, false, Kind::Answer),
        answer("b", "paid", (200, 0, 0, 80), 700, false, Kind::Answer),
        answer("c", "paid", (0, 0, 0, 0), 0, true, Kind::Answer),
        answer("d", "free", (1_000, 0, 0, 500), 0, false, Kind::Answer),
        answer("a", "paid", (9_999, 0, 0, 9_999), 9_999, false, Kind::Judge),
    ] {
        ledger.record(e);
    }
    ledger.flush().await;
    ledger
}

#[tokio::test]
async fn a_group_reports_the_calls_with_known_usage_and_what_the_calls_cost() {
    let dir = tempfile::tempdir().unwrap();
    let ledger = seeded(&dir.path().join("l.db")).await;
    let stats = ledger.stats(&query(GroupBy::Target)).await.unwrap();
    let paid = stats.groups.iter().find(|g| g.group == "paid").unwrap();
    // Three answered calls, two with usage; the judge call is in none of the figures.
    assert_eq!(
        (paid.answered, paid.usage_known, paid.cost_micro_usd),
        (3, 2, 1_000)
    );
    let free = stats.groups.iter().find(|g| g.group == "free").unwrap();
    assert_eq!(
        (free.answered, free.usage_known, free.cost_micro_usd),
        (1, 1, 0)
    );
}

#[tokio::test]
async fn the_figures_the_statistics_already_had_are_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    let ledger = seeded(&dir.path().join("l.db")).await;
    let stats = ledger.stats(&query(GroupBy::Target)).await.unwrap();
    let paid = stats.groups.iter().find(|g| g.group == "paid").unwrap();
    // The average still divides by every answered call, as it always did, and the token split still counts only
    // the calls that reported usage.
    assert_eq!(paid.avg_cost_micro_usd, Some(1_000.0 / 3.0));
    assert_eq!(
        paid.avg_tokens,
        Some((100.0 + 40.0 + 10.0 + 50.0 + 200.0 + 80.0) / 3.0)
    );
    assert_eq!(
        (
            paid.tokens.input,
            paid.tokens.cached_input,
            paid.tokens.cache_creation,
            paid.tokens.output
        ),
        (300, 40, 10, 130)
    );
}

#[tokio::test]
async fn the_sums_hold_for_every_grouping() {
    let dir = tempfile::tempdir().unwrap();
    let ledger = seeded(&dir.path().join("l.db")).await;
    for group_by in [
        GroupBy::Route,
        GroupBy::Endpoint,
        GroupBy::Actor,
        GroupBy::Owner,
    ] {
        let stats = ledger.stats(&query(group_by)).await.unwrap();
        let known: u64 = stats.groups.iter().map(|g| g.usage_known).sum();
        let cost: u64 = stats.groups.iter().map(|g| g.cost_micro_usd).sum();
        // Whatever the grouping, each answered call is in exactly one group.
        assert_eq!((known, cost), (3, 1_000), "{group_by:?}");
    }
}
