//! Runtime overrides: an operator's expiring instruction to stop using a target or a provider, or to pin a
//! route to one of its targets (`add-runtime-overrides`, ADR 0025). An override only ever removes
//! candidates, always expires, and is audited. This module is the store: it creates, revokes and lists
//! them. Applying them to a request's plan is a separate step.
//!
//! The store is a trait so a shared backend can replace the ledger-backed one without touching callers,
//! as `KeyStore` does for keys.

use std::sync::Arc;

use async_trait::async_trait;
use parking_lot::RwLock;

use crate::clock::Clock;
use crate::config::OverridesConfig;
use crate::ledger::{LedgerError, NewOverride, OverrideDb, OverrideRecord};

/// What an override does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Remove one target from every request's candidates.
    DisableTarget,
    /// Remove every target endpoint of one provider.
    DrainProvider,
    /// Cut one route's candidates to a single target.
    PinRoute,
}

impl Kind {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::DisableTarget => "disable_target",
            Self::DrainProvider => "drain_provider",
            Self::PinRoute => "pin_route",
        }
    }

    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "disable_target" => Some(Self::DisableTarget),
            "drain_provider" => Some(Self::DrainProvider),
            "pin_route" => Some(Self::PinRoute),
            _ => None,
        }
    }
}

/// An override in force.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Override {
    pub id: i64,
    pub kind: Kind,
    /// The target, provider or (for a pin) target it names.
    pub subject: String,
    /// The route a `pin_route` applies to; `None` for the other kinds.
    pub route: Option<String>,
    pub reason: String,
    pub created_ms: i64,
    pub expires_ms: i64,
    /// The id of the key that created it, never the key.
    pub created_by: String,
}

impl Override {
    /// Whether it is still in force at `now_ms`. An override ends at its expiry with no background job.
    #[must_use]
    pub fn in_force(&self, now_ms: i64) -> bool {
        self.expires_ms > now_ms
    }

    fn from_record(record: OverrideRecord) -> Option<Self> {
        Some(Self {
            id: record.id,
            kind: Kind::parse(&record.kind)?,
            subject: record.subject,
            route: record.route,
            reason: record.reason,
            created_ms: record.created_ms,
            expires_ms: record.expires_ms,
            created_by: record.created_by,
        })
    }
}

/// The override that takes `target` out of `route`'s candidates, if one does; the first that does when several
/// would. `providers` are the providers of the target's endpoints.
///
/// - A `disable_target` on it removes it.
/// - A route with pins keeps only the targets a pin names, so any other target is removed by (the first) pin.
/// - A target whose every endpoint is on a drained provider is removed by that drain; with an endpoint left,
///   the target stays and the pool skips the drained endpoints itself.
#[must_use]
pub fn removed_by<'a>(
    active: &'a [Override],
    route: &str,
    target: &str,
    providers: &[&str],
) -> Option<&'a Override> {
    let pins = || {
        active
            .iter()
            .filter(|o| o.kind == Kind::PinRoute && o.route.as_deref() == Some(route))
    };
    active
        .iter()
        .find(|o| o.kind == Kind::DisableTarget && o.subject == target)
        .or_else(|| {
            let named = pins().any(|o| o.subject == target);
            pins().next().filter(|_| !named)
        })
        .or_else(|| {
            let first = providers.first()?;
            providers
                .iter()
                .all(|p| drains(active, p))
                .then(|| {
                    active
                        .iter()
                        .find(|o| o.kind == Kind::DrainProvider && o.subject == *first)
                })
                .flatten()
        })
}

/// Whether the overrides in `active` drain `provider`.
#[must_use]
pub fn drains(active: &[Override], provider: &str) -> bool {
    active
        .iter()
        .any(|o| o.kind == Kind::DrainProvider && o.subject == provider)
}

/// Which providers are drained right now. The pool asks this for every endpoint it is about to try, so it
/// must be cheap; an override ending needs no other action because the answer follows the clock.
pub trait Drains: Send + Sync {
    fn drained(&self, provider: &str) -> bool;
}

/// Nothing is drained: a gateway without a ledger, which cannot hold overrides.
pub struct NoDrains;

impl Drains for NoDrains {
    fn drained(&self, _provider: &str) -> bool {
        false
    }
}

/// The drains an [`OverrideStore`] holds, read against a clock.
pub struct StoreDrains {
    store: Arc<dyn OverrideStore>,
    clock: Arc<dyn Clock>,
}

impl StoreDrains {
    #[must_use]
    pub fn new(store: Arc<dyn OverrideStore>, clock: Arc<dyn Clock>) -> Self {
        Self { store, clock }
    }
}

impl Drains for StoreDrains {
    fn drained(&self, provider: &str) -> bool {
        self.store.provider_drained(provider, self.clock.now_ms())
    }
}

/// How an override stands: in force, removed by someone, or run out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Active,
    Revoked,
    Expired,
}

impl Status {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Revoked => "revoked",
            Self::Expired => "expired",
        }
    }
}

/// An override as the history lists it: still in force or ended, and when it ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Past {
    pub entry: Override,
    pub status: Status,
    /// When it was revoked or ran out; none while it is in force.
    pub ended_ms: Option<i64>,
}

/// A request to create an override.
#[derive(Clone, Debug)]
pub struct Create {
    pub kind: Kind,
    pub subject: String,
    /// Required for `pin_route`, refused for the other kinds.
    pub route: Option<String>,
    pub reason: String,
    /// How long it lasts; the configured default when `None`.
    pub expires_in_secs: Option<u64>,
    /// The id of the key that asked.
    pub created_by: String,
}

#[derive(Debug, thiserror::Error)]
pub enum OverrideError {
    #[error("an override must last between 1 and {max} seconds")]
    Expiry { max: u64 },
    #[error("pin_route names a route and a target")]
    PinNeedsRoute,
    #[error("only pin_route names a route")]
    RouteNotAllowed,
    #[error("an override needs a reason")]
    NoReason,
    #[error(transparent)]
    Ledger(#[from] LedgerError),
}

/// When an override created at `now_ms` for `requested` seconds (or the default) expires. Refuses zero and
/// anything over the configured maximum, so no override lasts forever.
///
/// # Errors
/// [`OverrideError::Expiry`] when the time is zero or above `limits.max_secs`.
pub fn expiry_ms(
    now_ms: i64,
    requested: Option<u64>,
    limits: OverridesConfig,
) -> Result<i64, OverrideError> {
    let secs = requested.unwrap_or(limits.default_secs);
    if secs == 0 || secs > limits.max_secs {
        return Err(OverrideError::Expiry {
            max: limits.max_secs,
        });
    }
    let millis = i64::try_from(secs.saturating_mul(1000)).unwrap_or(i64::MAX);
    Ok(now_ms.saturating_add(millis))
}

#[async_trait]
pub trait OverrideStore: Send + Sync {
    /// Creates an override, stores it with its audit row, and returns it.
    async fn create(
        &self,
        request: Create,
        limits: OverridesConfig,
        now_ms: i64,
    ) -> Result<Override, OverrideError>;

    /// Revokes an override that is still in force; false if there was none (unknown, revoked or expired).
    async fn revoke(&self, id: i64, by: &str, now_ms: i64) -> Result<bool, OverrideError>;

    /// Notices overrides that have expired: drops them from memory and writes their audit rows, once.
    /// Called when a request, a scrape or a listing next looks, never from a background job.
    async fn sweep(&self, now_ms: i64) -> Result<(), OverrideError>;

    /// The overrides in force at `now_ms`. Cheap and synchronous: it runs on the request path.
    fn active(&self, now_ms: i64) -> Vec<Override>;

    /// Overrides created at or after `since_ms`, in force or ended, newest first, at most `limit`.
    async fn history(
        &self,
        now_ms: i64,
        since_ms: i64,
        limit: i64,
    ) -> Result<Vec<Past>, OverrideError>;

    /// Whether a drain of `provider` is in force at `now_ms`. Runs once per endpoint tried, so it must not
    /// allocate.
    fn provider_drained(&self, provider: &str, now_ms: i64) -> bool;
}

/// Overrides kept in the ledger and held in memory while in force. State is per instance, like health and
/// budgets.
pub struct LedgerOverrides {
    db: OverrideDb,
    active: RwLock<Vec<Override>>,
}

impl LedgerOverrides {
    /// Loads the overrides still in force, so they survive a restart, and audits any that ran out while
    /// the gateway was down.
    ///
    /// # Errors
    /// A ledger query fails.
    pub async fn open(db: OverrideDb, now_ms: i64) -> Result<Self, OverrideError> {
        db.note_expired(now_ms).await?;
        let active = db
            .active(now_ms)
            .await?
            .into_iter()
            .filter_map(Override::from_record)
            .collect();
        Ok(Self {
            db,
            active: RwLock::new(active),
        })
    }
}

#[async_trait]
impl OverrideStore for LedgerOverrides {
    async fn create(
        &self,
        request: Create,
        limits: OverridesConfig,
        now_ms: i64,
    ) -> Result<Override, OverrideError> {
        if request.reason.trim().is_empty() {
            return Err(OverrideError::NoReason);
        }
        match (request.kind, &request.route) {
            (Kind::PinRoute, None) => return Err(OverrideError::PinNeedsRoute),
            (Kind::DisableTarget | Kind::DrainProvider, Some(_)) => {
                return Err(OverrideError::RouteNotAllowed);
            }
            _ => {}
        }
        let expires_ms = expiry_ms(now_ms, request.expires_in_secs, limits)?;
        let new = NewOverride {
            kind: request.kind.as_str().to_string(),
            subject: request.subject.clone(),
            route: request.route.clone(),
            reason: request.reason.clone(),
            created_ms: now_ms,
            expires_ms,
            created_by: request.created_by.clone(),
        };
        let id = self.db.create(&new).await?;
        let created = Override {
            id,
            kind: request.kind,
            subject: request.subject,
            route: request.route,
            reason: request.reason,
            created_ms: now_ms,
            expires_ms,
            created_by: request.created_by,
        };
        self.active.write().push(created.clone());
        Ok(created)
    }

    async fn revoke(&self, id: i64, by: &str, now_ms: i64) -> Result<bool, OverrideError> {
        let revoked = self.db.revoke(id, now_ms, by).await?;
        if revoked {
            self.active.write().retain(|o| o.id != id);
        }
        Ok(revoked)
    }

    async fn sweep(&self, now_ms: i64) -> Result<(), OverrideError> {
        // Cheap on the request path: nothing to do until something in memory has run out.
        if !self.active.read().iter().any(|o| !o.in_force(now_ms)) {
            return Ok(());
        }
        self.active.write().retain(|o| o.in_force(now_ms));
        self.db.note_expired(now_ms).await?;
        Ok(())
    }

    fn active(&self, now_ms: i64) -> Vec<Override> {
        self.active
            .read()
            .iter()
            .filter(|o| o.in_force(now_ms))
            .cloned()
            .collect()
    }

    async fn history(
        &self,
        now_ms: i64,
        since_ms: i64,
        limit: i64,
    ) -> Result<Vec<Past>, OverrideError> {
        let records = self.db.recent(since_ms, limit).await?;
        Ok(records
            .into_iter()
            .filter_map(|record| {
                let (status, ended_ms) = match record.revoked_ms {
                    Some(at) => (Status::Revoked, Some(at)),
                    None if record.expires_ms <= now_ms => {
                        (Status::Expired, Some(record.expires_ms))
                    }
                    None => (Status::Active, None),
                };
                Some(Past {
                    entry: Override::from_record(record)?,
                    status,
                    ended_ms,
                })
            })
            .collect())
    }

    fn provider_drained(&self, provider: &str, now_ms: i64) -> bool {
        self.active
            .read()
            .iter()
            .any(|o| o.kind == Kind::DrainProvider && o.subject == provider && o.in_force(now_ms))
    }
}
