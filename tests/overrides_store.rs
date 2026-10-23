//! The runtime-override store (spec: `gateway-config`, `add-runtime-overrides` tasks 1.1 and 1.2): creation,
//! expiry, revocation, the audit trail and surviving a restart. Nothing here changes a request's plan.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // integration tests fail loudly on purpose

use std::sync::Arc;

use humpyard::config::OverridesConfig;
use humpyard::ledger::{Ledger, NewOverride};
use humpyard::overrides::{Create, Kind, LedgerOverrides, OverrideError, OverrideStore};

const NOW: i64 = 1_800_000_000_000;
const HOUR_MS: i64 = 3_600_000;
const LIMITS: OverridesConfig = OverridesConfig {
    default_secs: 14_400,
    max_secs: 86_400,
};

struct Setup {
    _dir: tempfile::TempDir,
    path: std::path::PathBuf,
    ledger: Arc<Ledger>,
    store: LedgerOverrides,
}

async fn setup() -> Setup {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ledger.db");
    let ledger = Arc::new(Ledger::open(&path).await.unwrap());
    let store = LedgerOverrides::open(ledger.override_db(), NOW)
        .await
        .unwrap();
    Setup {
        _dir: dir,
        path,
        ledger,
        store,
    }
}

fn disable(target: &str, secs: Option<u64>) -> Create {
    Create {
        kind: Kind::DisableTarget,
        subject: target.into(),
        route: None,
        reason: "bad answers".into(),
        expires_in_secs: secs,
        created_by: "ops".into(),
    }
}

#[tokio::test]
async fn an_override_defaults_to_four_hours_and_is_listed() {
    let s = setup().await;
    let o = s
        .store
        .create(disable("deep", None), LIMITS, NOW)
        .await
        .unwrap();
    assert_eq!(o.expires_ms, NOW + 4 * HOUR_MS);
    assert_eq!(s.store.active(NOW), vec![o]);
}

#[tokio::test]
async fn an_override_cannot_last_forever_or_not_at_all() {
    let s = setup().await;
    for secs in [0, 86_401, u64::MAX] {
        let error = s
            .store
            .create(disable("deep", Some(secs)), LIMITS, NOW)
            .await
            .unwrap_err();
        assert!(
            matches!(error, OverrideError::Expiry { max: 86_400 }),
            "{secs}: {error}"
        );
    }
    let longest = s
        .store
        .create(disable("deep", Some(86_400)), LIMITS, NOW)
        .await
        .unwrap();
    assert_eq!(longest.expires_ms, NOW + 24 * HOUR_MS);
    assert_eq!(
        s.store.active(NOW).len(),
        1,
        "a refused override is not stored"
    );
}

#[tokio::test]
async fn only_a_pin_names_a_route_and_every_override_has_a_reason() {
    let s = setup().await;
    let mut pin = disable("fast", None);
    pin.kind = Kind::PinRoute;
    assert!(matches!(
        s.store.create(pin.clone(), LIMITS, NOW).await.unwrap_err(),
        OverrideError::PinNeedsRoute
    ));
    pin.route = Some("auto".into());
    assert_eq!(
        s.store
            .create(pin, LIMITS, NOW)
            .await
            .unwrap()
            .route
            .as_deref(),
        Some("auto")
    );

    let mut drain = disable("groq", None);
    drain.kind = Kind::DrainProvider;
    drain.route = Some("auto".into());
    assert!(matches!(
        s.store.create(drain, LIMITS, NOW).await.unwrap_err(),
        OverrideError::RouteNotAllowed
    ));

    let mut blank = disable("deep", None);
    blank.reason = "  ".into();
    assert!(matches!(
        s.store.create(blank, LIMITS, NOW).await.unwrap_err(),
        OverrideError::NoReason
    ));
}

#[tokio::test]
async fn an_override_ends_at_its_expiry_with_no_sweep() {
    let s = setup().await;
    s.store
        .create(disable("deep", Some(60)), LIMITS, NOW)
        .await
        .unwrap();
    assert_eq!(s.store.active(NOW + 59_999).len(), 1);
    assert!(
        s.store.active(NOW + 60_000).is_empty(),
        "in force until, not at, the expiry"
    );
}

#[tokio::test]
async fn revoking_ends_it_once_and_the_audit_names_the_key_id() {
    let s = setup().await;
    let o = s
        .store
        .create(disable("deep", None), LIMITS, NOW)
        .await
        .unwrap();
    assert!(s.store.revoke(o.id, "ops2", NOW + 1000).await.unwrap());
    assert!(
        s.store.active(NOW + 1000).is_empty(),
        "a revoked override is no longer in force"
    );
    assert!(
        !s.store.revoke(o.id, "ops2", NOW + 2000).await.unwrap(),
        "already revoked"
    );
    assert!(!s.store.revoke(9999, "ops2", NOW).await.unwrap(), "unknown");

    let audit = s.ledger.override_db().audit(o.id).await.unwrap();
    let events: Vec<_> = audit
        .iter()
        .map(|a| (a.event.as_str(), a.at_ms, a.by.as_deref()))
        .collect();
    assert_eq!(
        events,
        [
            ("created", NOW, Some("ops")),
            ("revoked", NOW + 1000, Some("ops2"))
        ]
    );
}

#[tokio::test]
async fn an_expired_override_cannot_be_revoked() {
    let s = setup().await;
    let o = s
        .store
        .create(disable("deep", Some(60)), LIMITS, NOW)
        .await
        .unwrap();
    assert!(!s.store.revoke(o.id, "ops", NOW + 61_000).await.unwrap());
}

#[tokio::test]
async fn an_expiry_is_audited_once_at_the_time_it_ran_out() {
    let s = setup().await;
    let o = s
        .store
        .create(disable("deep", Some(60)), LIMITS, NOW)
        .await
        .unwrap();
    s.store.sweep(NOW + 30_000).await.unwrap();
    assert_eq!(
        s.ledger.override_db().audit(o.id).await.unwrap().len(),
        1,
        "not expired yet"
    );

    // Noticed late: the row says when it ended, not when someone looked.
    s.store.sweep(NOW + 10 * HOUR_MS).await.unwrap();
    s.store.sweep(NOW + 11 * HOUR_MS).await.unwrap();
    let audit = s.ledger.override_db().audit(o.id).await.unwrap();
    assert_eq!(audit.len(), 2, "{audit:?}");
    assert_eq!(
        (audit[1].event.as_str(), audit[1].at_ms, audit[1].by.clone()),
        ("expired", NOW + 60_000, None)
    );
}

#[tokio::test]
async fn an_override_in_force_survives_a_restart_and_an_ended_one_does_not() {
    let s = setup().await;
    let keep = s
        .store
        .create(disable("deep", Some(7200)), LIMITS, NOW)
        .await
        .unwrap();
    let revoked = s
        .store
        .create(disable("fast", Some(7200)), LIMITS, NOW)
        .await
        .unwrap();
    s.store.revoke(revoked.id, "ops", NOW).await.unwrap();
    s.store
        .create(disable("smart", Some(60)), LIMITS, NOW)
        .await
        .unwrap();

    let Setup {
        path,
        ledger,
        store,
        _dir,
    } = s;
    drop(store);
    Arc::try_unwrap(ledger).ok().unwrap().shutdown().await;

    let ledger = Arc::new(Ledger::open(&path).await.unwrap());
    let reopened = LedgerOverrides::open(ledger.override_db(), NOW + HOUR_MS)
        .await
        .unwrap();
    assert_eq!(reopened.active(NOW + HOUR_MS), vec![keep]);
}

#[tokio::test]
async fn history_is_pruned_but_an_override_in_force_never_is() {
    let dir = tempfile::tempdir().unwrap();
    let ledger = Ledger::open_with(&dir.path().join("l.db"), 100, 1)
        .await
        .unwrap();
    let real_now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap();
    let real_now = i64::try_from(real_now.as_millis()).unwrap();
    let new = |created_ms, expires_ms| NewOverride {
        kind: "disable_target".into(),
        subject: "deep".into(),
        route: None,
        reason: "r".into(),
        created_ms,
        expires_ms,
        created_by: "ops".into(),
    };
    let old = ledger.override_db().create(&new(1000, 2000)).await.unwrap();
    let live = ledger
        .override_db()
        .create(&new(real_now, real_now + HOUR_MS))
        .await
        .unwrap();

    // The writer prunes on its first pass; a flush makes it take one.
    ledger.flush().await;
    ledger.flush().await;

    assert!(
        ledger.override_db().audit(old).await.unwrap().is_empty(),
        "an ended override's history is pruned"
    );
    assert_eq!(ledger.override_db().audit(live).await.unwrap().len(), 1);
    let active = ledger.override_db().active(real_now).await.unwrap();
    assert_eq!(active.iter().map(|o| o.id).collect::<Vec<_>>(), [live]);
}
