//! Request paths, normalised before anything is matched against them (ADR 0032).
//!
//! A path is split on `/`, each segment is percent-decoded once, and the path is refused if any segment is a dot
//! segment or holds a separator or control character once decoded. So `/admin/v1/../v1/chat`, `/admin/v1/%2e%2e/x`
//! and `/admin/v1/..%2fx` are all refused, not resolved: there is no path that leaves a prefix because none is
//! ever resolved. Repeated slashes collapse.

use std::fmt::Write as _;

/// The decoded segments of `raw` (a request path, still percent-encoded), or `None` when it must be refused.
pub(super) fn segments(raw: &str) -> Option<Vec<String>> {
    let mut out = Vec::new();
    for part in raw.split('/').filter(|part| !part.is_empty()) {
        let decoded = decode(part)?;
        let bad = decoded == "."
            || decoded == ".."
            || decoded
                .chars()
                .any(|c| matches!(c, '/' | '\\') || c.is_control());
        if bad {
            return None;
        }
        out.push(decoded);
    }
    Some(out)
}

/// Percent-decodes one segment, once. A malformed escape or bytes that are not UTF-8 refuse the segment.
fn decode(segment: &str) -> Option<String> {
    let bytes = segment.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] == b'%' {
            let pair = bytes.get(at + 1..at + 3)?;
            let value = u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok()?;
            out.push(value);
            at += 3;
        } else {
            out.push(bytes[at]);
            at += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// A segment as it is written into a forwarded path: everything but unreserved characters is percent-encoded.
pub(super) fn encode(segment: &str) -> String {
    let mut out = String::with_capacity(segment.len());
    for byte in segment.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            out.push(char::from(byte));
        } else {
            let _ = write!(out, "%{byte:02X}");
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_paths_split_and_repeated_slashes_collapse() {
        assert_eq!(
            segments("/admin/v1/health"),
            Some(vec!["admin".into(), "v1".into(), "health".into()])
        );
        assert_eq!(
            segments("//admin///v1/"),
            Some(vec!["admin".into(), "v1".into()])
        );
        assert_eq!(segments("/"), Some(vec![]));
    }

    #[test]
    fn dot_segments_are_refused_in_every_spelling() {
        for path in [
            "/admin/v1/../v1/chat/completions",
            "/admin/v1/./health",
            "/admin/v1/%2e%2e/v1/chat",
            "/admin/v1/%2E%2E/x",
            "/admin/v1/.%2e/x",
            "/admin/v1/%2e/x",
            "/../../etc/passwd",
            "/admin/v1/..%2fx",
            "/admin/v1/..%5cx",
            "/admin/v1/a%2fb",
            "/admin/v1/a%5cb",
            "/admin/v1/a\\b",
            "/admin/v1/%00",
            "/admin/v1/%0a",
        ] {
            assert_eq!(segments(path), None, "{path}");
        }
    }

    #[test]
    fn a_malformed_escape_or_invalid_utf8_is_refused() {
        for path in ["/a/%", "/a/%4", "/a/%zz", "/a/%ff", "/a/%c0%af"] {
            assert_eq!(segments(path), None, "{path}");
        }
    }

    #[test]
    fn a_double_encoded_dot_segment_stays_a_harmless_name() {
        // Decoded once it is the literal text `%2e%2e`, which is written back encoded, never as `..`.
        let parts = segments("/admin/v1/%252e%252e").unwrap();
        assert_eq!(parts[2], "%2e%2e");
        assert_eq!(encode(&parts[2]), "%252e%252e");
    }

    #[test]
    fn encoding_keeps_unreserved_characters_and_escapes_the_rest() {
        assert_eq!(encode("a-b_c.d~e"), "a-b_c.d~e");
        assert_eq!(encode("a b/é"), "a%20b%2F%C3%A9");
    }
}
