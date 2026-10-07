//! Routing policy: the seam where budget and provider-health information narrow which targets
//! a route may use. Algorithms never see ineligible targets.

use std::sync::Arc;

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

/// The request metadata a policy may use, as the gateway understands it (not Switchyard's type).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RequestMeta {
    pub agent_id: Option<String>,
    pub parent_agent_id: Option<String>,
    /// The harness marked this request as coming from a child agent.
    pub is_subagent: bool,
    pub task_id: Option<String>,
}

/// What a policy may know about the request it is deciding for.
pub struct PolicyContext<'a> {
    /// The route (or bare target) the client requested.
    pub route: &'a str,
    /// Session id from `x-switchyard-session-id`, when the client sent one.
    pub session_id: Option<&'a str>,
    /// Agent and task identifiers the client supplied.
    pub metadata: &'a RequestMeta,
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
