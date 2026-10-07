//! Routing policy: the seam where budget and provider-health information narrow which targets
//! a route may use. Algorithms never see ineligible targets.

use switchyard_protocol::Metadata;

/// What a policy may know about the request it is deciding for.
pub struct PolicyContext<'a> {
    /// The route (or bare target) the client requested.
    pub route: &'a str,
    /// Session id from `x-switchyard-session-id`, when the client sent one.
    pub session_id: Option<&'a str>,
    /// All request metadata (agent ids, task ids, headers Switchyard understands).
    pub metadata: &'a Metadata,
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
