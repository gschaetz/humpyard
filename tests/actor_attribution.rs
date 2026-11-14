//! Attribution (ADR 0037): a decision records the actor and owner its key was under, a ledger from before v14
//! upgrades, and the statistics count by either with the cost found through the request id.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test scaffolding

use humpyard::ledger::{
    Attempt, Decision, DecisionQuery, Entry, GroupBy, Kind, Ledger, StatsQuery,
};
use sqlx::Connection;

const NOW: i64 = 1_800_000_000_000;

fn attempt(outcome: &str, ms: u64) -> Attempt {
    Attempt {
        target: "t".into(),
        provider: "p".into(),
        model: "m".into(),
        outcome: outcome.into(),
        ms,
        class: None,
        retry_after_ms: None,
        message: None,
    }
}

/// A decision for `id` made under `actor` and `owner` (key `k`, route `r`).
fn decision(
    id: &str,
    actor: Option<&str>,
    owner: Option<&str>,
    attempts: Vec<Attempt>,
) -> Decision {
    Decision {
        request_id: id.into(),
        ts_ms: NOW - 1000,
        key_id: Some("k".into()),
        actor: actor.map(str::to_string),
        owner: owner.map(str::to_string),
        route: Some("r".into()),
        attempts,
        outcome: "ok".into(),
        status: 200,
        ..Decision::default()
    }
}

fn answer(id: &str, cost: u64) -> Entry {
    Entry {
        ts_ms: NOW - 1000,
        key_id: Some("k".into()),
        session_id: None,
        route: "r".into(),
        target: "t".into(),
        provider: "p".into(),
        model: "m".into(),
        kind: Kind::Answer,
        input_tokens: 10,
        cached_input_tokens: 0,
        cache_creation_tokens: 0,
        output_tokens: 5,
        reasoning_tokens: 0,
        cost_micro_usd: cost,
        outcome: "ok".into(),
        usage_missing: false,
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

/// Five requests: two of agent `claude-code` (owner `gary`, one needing a fallback), one of the human `gary`
/// himself, one of an agent with no owner, and one of a key with no actor.
async fn seeded(path: &std::path::Path) -> Ledger {
    let ledger = Ledger::open(path).await.unwrap();
    for d in [
        decision(
            "a",
            Some("claude-code"),
            Some("gary"),
            vec![attempt("ok", 100)],
        ),
        decision(
            "b",
            Some("claude-code"),
            Some("gary"),
            vec![attempt("rate_limit", 50), attempt("ok", 200)],
        ),
        decision("c", Some("gary"), Some("gary"), vec![attempt("ok", 300)]),
        decision("d", Some("openclaw"), None, vec![attempt("ok", 400)]),
        decision("e", None, None, vec![attempt("ok", 500)]),
    ] {
        ledger.record_decision(d);
    }
    for (id, cost) in [("a", 100), ("b", 200), ("c", 300), ("d", 400), ("e", 500)] {
        ledger.record(answer(id, cost));
    }
    ledger.flush().await;
    ledger
}

fn group<'a>(stats: &'a humpyard::ledger::Stats, name: &str) -> &'a humpyard::ledger::GroupStats {
    stats
        .groups
        .iter()
        .find(|g| g.group == name)
        .unwrap_or_else(|| panic!("no group {name}"))
}

#[tokio::test]
async fn a_decision_keeps_its_actor_and_owner_and_a_key_without_one_keeps_none() {
    let dir = tempfile::tempdir().unwrap();
    let ledger = seeded(&dir.path().join("l.db")).await;
    let all = ledger
        .decisions(&DecisionQuery {
            limit: 10,
            ..DecisionQuery::default()
        })
        .await
        .unwrap();
    let of = |id: &str| all.iter().find(|d| d.request_id == id).unwrap();
    assert_eq!(
        (of("a").actor.as_deref(), of("a").owner.as_deref()),
        (Some("claude-code"), Some("gary"))
    );
    assert_eq!(
        (of("c").actor.as_deref(), of("c").owner.as_deref()),
        (Some("gary"), Some("gary"))
    );
    assert_eq!(
        (of("d").actor.as_deref(), of("d").owner.as_deref()),
        (Some("openclaw"), None)
    );
    assert_eq!(
        (of("e").actor.as_deref(), of("e").owner.as_deref()),
        (None, None)
    );
}

#[tokio::test]
async fn statistics_by_actor_count_each_request_once_with_its_cost_and_put_the_rest_under_none() {
    let dir = tempfile::tempdir().unwrap();
    let ledger = seeded(&dir.path().join("l.db")).await;
    let stats = ledger.stats(&query(GroupBy::Actor)).await.unwrap();
    assert_eq!(
        stats.groups.len(),
        4,
        "{:?}",
        stats.groups.iter().map(|g| &g.group).collect::<Vec<_>>()
    );
    let agent = group(&stats, "claude-code");
    assert_eq!(
        (
            agent.requests,
            agent.attempts,
            agent.fallbacks,
            agent.failures
        ),
        (2, 3, 1, 1)
    );
    assert_eq!(agent.answered, 2);
    assert_eq!(agent.avg_cost_micro_usd, Some(150.0));
    assert_eq!(group(&stats, "gary").requests, 1);
    assert_eq!(group(&stats, "openclaw").requests, 1);
    let none = group(&stats, "(none)");
    assert_eq!((none.requests, none.answered), (1, 1));
    assert_eq!(none.avg_cost_micro_usd, Some(500.0));
    // Nothing is dropped from the totals.
    let total: u64 = stats.groups.iter().map(|g| g.requests).sum();
    assert_eq!(total, 5);
}

#[tokio::test]
async fn statistics_by_owner_roll_agents_and_the_human_up_together() {
    let dir = tempfile::tempdir().unwrap();
    let ledger = seeded(&dir.path().join("l.db")).await;
    let stats = ledger.stats(&query(GroupBy::Owner)).await.unwrap();
    let gary = group(&stats, "gary");
    // Both of claude-code's requests and the human's own.
    assert_eq!((gary.requests, gary.answered), (3, 3));
    assert_eq!(gary.avg_cost_micro_usd, Some(200.0));
    // An agent with no owner and a key with no actor both have nobody to roll up to.
    assert_eq!(group(&stats, "(none)").requests, 2);
    assert_eq!(stats.groups.len(), 2);
}

#[tokio::test]
async fn the_key_and_target_filters_still_apply_when_grouping_by_actor() {
    let dir = tempfile::tempdir().unwrap();
    let ledger = seeded(&dir.path().join("l.db")).await;
    let mut other_key = query(GroupBy::Actor);
    other_key.key = Some("nobody".into());
    assert_eq!(ledger.stats(&other_key).await.unwrap().groups.len(), 0);
    let mut other_target = query(GroupBy::Actor);
    other_target.target = Some("not-a-target".into());
    let stats = ledger.stats(&other_target).await.unwrap();
    // Requests are still counted once, but none had an attempt on that target.
    assert!(
        stats.groups.iter().all(|g| g.attempts == 0),
        "{:?}",
        stats.groups
    );
}

#[tokio::test]
async fn a_series_by_actor_counts_the_buckets_as_the_totals_do() {
    use humpyard::ledger::SeriesQuery;
    let dir = tempfile::tempdir().unwrap();
    let ledger = seeded(&dir.path().join("l.db")).await;
    let series = ledger
        .series(&SeriesQuery {
            stats: query(GroupBy::Actor),
            interval_ms: 60_000,
            now_ms: NOW,
            limit: 8,
        })
        .await
        .unwrap();
    let totals = ledger.stats(&query(GroupBy::Actor)).await.unwrap();
    for g in &series.groups {
        let requests: u64 = g.buckets.iter().map(|b| b.requests).sum();
        assert_eq!(requests, group(&totals, &g.group).requests, "{}", g.group);
    }
    assert!(series.groups.iter().any(|g| g.group == "claude-code"));
}

/// A ledger file as v13 left it: no actor and owner columns, one decision.
async fn previous_version(path: &std::path::Path) {
    let ledger = Ledger::open(path).await.unwrap();
    ledger.record_decision(Decision {
        request_id: "old".into(),
        ts_ms: NOW - 1000,
        route: Some("r".into()),
        outcome: "ok".into(),
        status: 200,
        ..Decision::default()
    });
    ledger.flush().await;
    ledger.shutdown().await;
    let options = sqlx::sqlite::SqliteConnectOptions::new().filename(path);
    let mut conn = sqlx::SqliteConnection::connect_with(&options)
        .await
        .unwrap();
    for statement in [
        "ALTER TABLE decisions DROP COLUMN actor",
        "ALTER TABLE decisions DROP COLUMN owner",
        "PRAGMA user_version = 13",
    ] {
        sqlx::query(statement).execute(&mut conn).await.unwrap();
    }
    conn.close().await.unwrap();
}

#[tokio::test]
async fn a_ledger_from_before_v14_upgrades_reads_its_old_rows_as_none_and_takes_new_ones() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("old.db");
    previous_version(&path).await;

    let ledger = Ledger::open(&path).await.unwrap();
    ledger.record_decision(decision(
        "new",
        Some("claude-code"),
        Some("gary"),
        vec![attempt("ok", 1)],
    ));
    ledger.flush().await;
    let rows = ledger
        .decisions(&DecisionQuery {
            limit: 10,
            ..DecisionQuery::default()
        })
        .await
        .unwrap();
    let of = |id: &str| rows.iter().find(|d| d.request_id == id).unwrap();
    assert_eq!(
        (of("old").actor.clone(), of("old").owner.clone()),
        (None, None)
    );
    assert_eq!(of("new").actor.as_deref(), Some("claude-code"));
    // The old row is counted under "(none)" next to the new one.
    let stats = ledger.stats(&query(GroupBy::Actor)).await.unwrap();
    assert_eq!(group(&stats, "(none)").requests, 1);
    assert_eq!(group(&stats, "claude-code").requests, 1);
    ledger.shutdown().await;

    let options = sqlx::sqlite::SqliteConnectOptions::new().filename(&path);
    let mut conn = sqlx::SqliteConnection::connect_with(&options)
        .await
        .unwrap();
    let version: i64 = sqlx::query_scalar("PRAGMA user_version")
        .fetch_one(&mut conn)
        .await
        .unwrap();
    assert_eq!(version, 14);
    conn.close().await.unwrap();
}
