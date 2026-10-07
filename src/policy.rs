//! Routing policy: the seam where budget and provider-health information narrow which targets
//! a route may use. Algorithms never see ineligible targets.

use std::sync::Arc;

use switchyard_protocol::Metadata;

/// How close an authenticated key is to its budget.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BudgetState {
    Healthy,
    /// Past the configured fraction of a limit.
    Restricted,
    /// At or past a limit (only reachable for keys that keep running on free targets).
    Exhausted,
}

/// The authenticated caller, when there is one.
pub struct KeyContext<'a> {
    pub id: &'a str,
    pub budget: BudgetState,
}

/// What a policy may know about the request it is deciding for.
pub struct PolicyContext<'a> {
    /// The route (or bare target) the client requested.
    pub route: &'a str,
    /// Session id from `x-switchyard-session-id`, when the client sent one.
    pub session_id: Option<&'a str>,
    /// All request metadata (agent ids, task ids, headers Switchyard understands).
    pub metadata: &'a Metadata,
    /// The authenticated key and its budget state; `None` when the gateway is open.
    pub key: Option<KeyContext<'a>>,
}

pub trait RoutingPolicy: Send + Sync {
    /// Whether `target` may serve this request.
    fn is_eligible(&self, context: &PolicyContext<'_>, target: &str) -> bool;
}

/// The default policy: every configured target is eligible.
pub struct AllowAll;

impl RoutingPolicy for AllowAll {
    fn is_eligible(&self, _context: &PolicyContext<'_>, _target: &str) -> bool {
        true
    }
}

/// A target is eligible only if every policy agrees.
pub struct All(pub Vec<Arc<dyn RoutingPolicy>>);

impl RoutingPolicy for All {
    fn is_eligible(&self, context: &PolicyContext<'_>, target: &str) -> bool {
        self.0.iter().all(|p| p.is_eligible(context, target))
    }
}
