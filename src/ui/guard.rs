//! Who may talk to `humpyard ui` (ADR 0032): the `Host` and `Origin` rules that defeat DNS rebinding and
//! cross-site requests, and the per-run access token that stands between other local users and the credential
//! the command holds.

use std::collections::HashSet;

use parking_lot::Mutex;
use sha2::{Digest, Sha256};

/// The session cookie's name.
pub(super) const COOKIE: &str = "humpyard_ui";

/// How many sessions are remembered; beyond it the oldest are forgotten and those browsers ask for the token again.
const MAX_SESSIONS: usize = 64;

/// The host part of a `Host` header, lower-cased and without its port: `Localhost:8787` is `localhost`,
/// `[::1]:8787` is `[::1]`.
fn host_only(value: &str) -> String {
    let value = value.trim().to_ascii_lowercase();
    if value.starts_with('[') {
        return value
            .find(']')
            .map_or(value.clone(), |end| value[..=end].to_string());
    }
    match value.rsplit_once(':') {
        Some((host, port)) if !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit()) => {
            host.to_string()
        }
        _ => value,
    }
}

/// Whether the request's `Host` names this machine's loopback or a host the operator allowed. An allowed entry
/// with a port must match host and port; one without matches the host on any port.
pub(super) fn host_allowed(host: &str, allowed: &[String]) -> bool {
    let host = host.trim().to_ascii_lowercase();
    if matches!(
        host_only(&host).as_str(),
        "localhost" | "127.0.0.1" | "[::1]"
    ) {
        return true;
    }
    allowed.iter().any(|entry| {
        let entry = entry.trim().to_ascii_lowercase();
        entry == host || (host_only(&entry) == entry && host_only(&host) == entry)
    })
}

/// Whether an `Origin` header is this service's own: `http(s)://` and exactly the `Host` the request came to.
pub(super) fn origin_is_own(origin: &str, host: &str) -> bool {
    let origin = origin.trim().to_ascii_lowercase();
    let authority = origin
        .strip_prefix("http://")
        .or_else(|| origin.strip_prefix("https://"));
    authority.is_some_and(|authority| authority == host.trim().to_ascii_lowercase())
}

/// The value of cookie `name` in a `Cookie` header.
pub(super) fn cookie_value<'a>(header: &'a str, name: &str) -> Option<&'a str> {
    header
        .split(';')
        .filter_map(|pair| pair.trim().split_once('='))
        .find_map(|(key, value)| (key == name).then_some(value))
}

/// The access token printed at start and the sessions issued in exchange for it.
pub(super) struct Sessions {
    token_digest: [u8; 32],
    issued: Mutex<Vec<String>>,
}

impl Sessions {
    pub(super) fn new(token: &str) -> Self {
        Self {
            token_digest: Sha256::digest(token.as_bytes()).into(),
            issued: Mutex::new(Vec::new()),
        }
    }

    /// Whether `offered` is the token. Both sides are hashed first, so how long the comparison takes says nothing
    /// about how much of the token matched.
    pub(super) fn token_matches(&self, offered: &str) -> bool {
        let digest: [u8; 32] = Sha256::digest(offered.as_bytes()).into();
        digest
            .iter()
            .zip(&self.token_digest)
            .fold(0u8, |acc, (a, b)| acc | (a ^ b))
            == 0
    }

    /// Remembers a new session id.
    pub(super) fn remember(&self, id: String) {
        let mut issued = self.issued.lock();
        issued.push(id);
        if issued.len() > MAX_SESSIONS {
            let excess = issued.len() - MAX_SESSIONS;
            issued.drain(..excess);
        }
    }

    /// Whether a request's `Cookie` header carries a session this run issued.
    pub(super) fn recognises(&self, cookie_header: Option<&str>) -> bool {
        let Some(id) = cookie_header.and_then(|header| cookie_value(header, COOKIE)) else {
            return false;
        };
        let issued = self.issued.lock();
        let known: HashSet<&str> = issued.iter().map(String::as_str).collect();
        known.contains(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn allowed(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn loopback_hosts_are_always_allowed_on_any_port() {
        for host in [
            "localhost",
            "localhost:8787",
            "LOCALHOST:1",
            "127.0.0.1:8787",
            "[::1]:8787",
            "[::1]",
        ] {
            assert!(host_allowed(host, &[]), "{host}");
        }
    }

    #[test]
    fn a_rebound_name_is_refused() {
        for host in [
            "attacker.example:8787",
            "localhost.attacker.example",
            "127.0.0.1.attacker.example",
            "",
            "evil:8787@localhost",
        ] {
            assert!(!host_allowed(host, &[]), "{host:?}");
        }
    }

    #[test]
    fn a_named_host_is_allowed_with_its_port_or_on_any_port_when_named_without_one() {
        let with_port = allowed(&["ui.internal:8787"]);
        assert!(host_allowed("ui.internal:8787", &with_port));
        assert!(host_allowed("UI.internal:8787", &with_port));
        assert!(!host_allowed("ui.internal:9999", &with_port));
        assert!(!host_allowed("ui.internal", &with_port));
        let without = allowed(&["ui.internal"]);
        assert!(host_allowed("ui.internal:9999", &without));
        assert!(host_allowed("ui.internal", &without));
        assert!(!host_allowed("other.internal", &without));
    }

    #[test]
    fn an_origin_is_own_only_when_it_is_exactly_the_host_asked() {
        assert!(origin_is_own("http://localhost:8787", "localhost:8787"));
        assert!(origin_is_own("https://ui.internal", "ui.internal"));
        assert!(!origin_is_own("http://attacker.example", "localhost:8787"));
        assert!(!origin_is_own(
            "http://localhost:8787.attacker.example",
            "localhost:8787"
        ));
        assert!(!origin_is_own("null", "localhost:8787"));
        assert!(!origin_is_own("ftp://localhost:8787", "localhost:8787"));
    }

    #[test]
    fn cookies_are_found_by_name() {
        assert_eq!(
            cookie_value("a=1; humpyard_ui=xyz; b=2", COOKIE),
            Some("xyz")
        );
        assert_eq!(cookie_value("a=1", COOKIE), None);
        assert_eq!(cookie_value("xhumpyard_ui=1", COOKIE), None);
    }

    #[test]
    fn the_token_is_compared_whole_and_sessions_are_remembered_and_bounded() {
        let sessions = Sessions::new("right");
        assert!(sessions.token_matches("right"));
        assert!(!sessions.token_matches("righ"));
        assert!(!sessions.token_matches(""));
        sessions.remember("first".into());
        assert!(sessions.recognises(Some("humpyard_ui=first")));
        assert!(!sessions.recognises(Some("humpyard_ui=other")));
        assert!(!sessions.recognises(None));
        for n in 0..MAX_SESSIONS {
            sessions.remember(format!("s{n}"));
        }
        assert!(
            !sessions.recognises(Some("humpyard_ui=first")),
            "the oldest is forgotten"
        );
        assert!(sessions.recognises(Some(&format!("humpyard_ui=s{}", MAX_SESSIONS - 1))));
    }
}
