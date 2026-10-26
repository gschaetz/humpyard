//! Making a provider's error text safe to keep in a decision (ADR 0028, ADR 0004).
//!
//! The text is provider-controlled. [`line`] turns it into one bounded line with anything key-shaped replaced,
//! and [`error_text`] picks the provider's message out of an error body first. Neither ever returns more than
//! the limit it is given, and neither looks at the request.

/// What replaces a value that looked like a secret.
const REDACTED: &str = "[redacted]";
/// How much of a text is examined; a key straddling this point is far beyond any limit a caller keeps.
const MAX_INPUT_CHARS: usize = 4096;
/// A body larger than this is not parsed as JSON; its start is used as it is.
const MAX_JSON_BYTES: usize = 64 * 1024;
/// Prefixes of well-known secret formats. A token that starts with one is redacted.
const SECRET_PREFIXES: [&str; 8] = ["sk-", "sk_", "gsk_", "nvapi-", "AIza", "ghp_", "xox", "eyJ"];
/// Names whose value, after `=` or `:`, is a secret.
const SECRET_NAMES: [&str; 9] = [
    "key",
    "api_key",
    "apikey",
    "token",
    "access_token",
    "secret",
    "password",
    "auth",
    "signature",
];
/// The shortest unbroken run of letters and digits that is taken for a secret when it mixes both.
const LONG_RUN: usize = 32;

/// One line of at most `limit` characters: control characters and line breaks become single spaces, and
/// anything key-shaped is replaced with `[redacted]`. Redaction happens before the cut, so a key is never
/// left half showing.
#[must_use]
pub fn line(text: &str, limit: usize) -> String {
    let flat = flatten(text);
    redact(&flat).chars().take(limit).collect()
}

/// The provider's message in an error `body`: the JSON `error.message` (also when the body is a list, as
/// Google's is), `message` or `detail`; otherwise the start of the body. `None` when there is no text.
#[must_use]
pub fn error_text(body: &str, limit: usize) -> Option<String> {
    let from_json = (body.len() <= MAX_JSON_BYTES)
        .then(|| serde_json::from_str::<serde_json::Value>(body).ok())
        .flatten()
        .and_then(|value| message_of(&value));
    let text = line(from_json.as_deref().unwrap_or(body), limit);
    (!text.is_empty()).then_some(text)
}

fn message_of(value: &serde_json::Value) -> Option<String> {
    let value = value
        .as_array()
        .and_then(|list| list.first())
        .unwrap_or(value);
    let text = |candidate: &serde_json::Value| {
        candidate
            .as_str()
            .filter(|t| !t.trim().is_empty())
            .map(str::to_owned)
    };
    text(&value["error"]["message"])
        .or_else(|| text(&value["error"]))
        .or_else(|| text(&value["message"]))
        .or_else(|| text(&value["detail"]))
}

/// Control characters and runs of white space become one space; the ends are trimmed.
fn flatten(text: &str) -> String {
    let mut out = String::new();
    let mut space = true;
    for c in text.chars().take(MAX_INPUT_CHARS) {
        if c.is_control() || c.is_whitespace() {
            if !space {
                out.push(' ');
                space = true;
            }
        } else {
            out.push(c);
            space = false;
        }
    }
    out.truncate(out.trim_end().len());
    out
}

fn is_word(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '-' || c == '_'
}

/// Replaces each secret-looking word, keeping everything between words as it was.
fn redact(text: &str) -> String {
    let mut out = String::new();
    let mut previous_word = String::new();
    let mut gap = String::new();
    let mut rest = text;
    while !rest.is_empty() {
        let split = rest
            .find(|c: char| !is_word(c))
            .map_or(rest.len(), |at| if at == 0 { first_len(rest) } else { at });
        let (piece, tail) = rest.split_at(split);
        rest = tail;
        if piece.chars().next().is_some_and(is_word) {
            if secret(piece, &previous_word, &gap) {
                out.push_str(REDACTED);
            } else {
                out.push_str(piece);
            }
            previous_word = piece.to_ascii_lowercase();
            gap.clear();
        } else {
            out.push_str(piece);
            gap.push_str(piece);
        }
    }
    out
}

fn first_len(text: &str) -> usize {
    text.chars().next().map_or(0, char::len_utf8)
}

/// Whether `word`, coming after `previous` and the separator `gap`, looks like a secret.
fn secret(word: &str, previous: &str, gap: &str) -> bool {
    if previous == "bearer" || previous == "basic" {
        return true;
    }
    let after_name = SECRET_NAMES.contains(&previous)
        && !gap.is_empty()
        && gap
            .chars()
            .all(|c| matches!(c, '=' | ':' | '"' | '\'' | ' '))
        && gap.chars().any(|c| c == '=' || c == ':');
    if after_name {
        return true;
    }
    if word.len() >= 8
        && SECRET_PREFIXES
            .iter()
            .any(|prefix| word.starts_with(prefix))
    {
        return true;
    }
    word.len() >= LONG_RUN
        && word.chars().any(|c| c.is_ascii_alphabetic())
        && word.chars().any(|c| c.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordinary_text_is_left_alone() {
        let text = "Request contains an invalid argument: messages[12].role must be user";
        assert_eq!(line(text, 200), text);
    }

    #[test]
    fn a_bearer_token_is_redacted() {
        assert_eq!(
            line("bad header Authorization: Bearer abc123def456", 200),
            "bad header Authorization: Bearer [redacted]"
        );
    }

    /// A key-shaped value built at run time, so no source line holds one.
    fn shaped(prefix: &str, tail: &str) -> String {
        format!("{prefix}{tail}")
    }

    #[test]
    fn known_key_prefixes_are_redacted() {
        for secret in [
            shaped("sk-", "humpyardABCDEF123"),
            shaped("gsk_", "abcdefghij"),
            shaped("nvapi-", "abcdefgh"),
            shaped("AIza", "SyABCDEFG"),
            shaped("ghp_", "abcdefghijkl"),
            shaped("eyJ", "hbGciOiJIUzI1"),
        ] {
            let out = line(&format!("invalid key {secret} for this model"), 200);
            assert_eq!(out, "invalid key [redacted] for this model", "{secret}");
        }
    }

    #[test]
    fn a_secret_query_parameter_is_redacted_and_the_rest_kept() {
        let out = line(
            "GET https://x.example/v1?model=m&api_key=shortsecret&n=2 failed",
            200,
        );
        assert_eq!(
            out,
            "GET https://x.example/v1?model=m&api_key=[redacted]&n=2 failed"
        );
        assert_eq!(line("token: abcd1234", 200), "token: [redacted]");
        assert_eq!(
            line("the token limit is 8000", 200),
            "the token limit is 8000"
        );
    }

    #[test]
    fn a_long_mixed_run_is_redacted_but_a_long_word_is_not() {
        let id = "a1b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6";
        assert_eq!(
            line(&format!("request {id} failed"), 200),
            "request [redacted] failed"
        );
        let word = "internationalization".repeat(3);
        assert_eq!(line(&word, 200), word);
    }

    #[test]
    fn it_is_one_line_without_control_characters() {
        assert_eq!(line("a\nb\r\n\tc\u{0}\u{7}d  e ", 200), "a b c d e");
    }

    #[test]
    fn it_is_cut_on_a_character_boundary_after_redaction() {
        assert_eq!(line("ééééé", 3), "ééé");
        // A key near the cut is redacted first, never left half showing.
        let out = line(&shaped("sk-", "abcdefghijklmnopqrstuvwxyz0123456789"), 8);
        assert_eq!(out, "[redacte");
        assert!(!out.contains("sk-"));
    }

    #[test]
    fn the_message_is_taken_from_an_openai_shaped_body() {
        let body =
            r#"{"error":{"message":"Invalid value for 'tools'.","type":"invalid_request_error"}}"#;
        assert_eq!(
            error_text(body, 200).as_deref(),
            Some("Invalid value for 'tools'.")
        );
    }

    #[test]
    fn the_message_is_taken_from_a_google_shaped_list_body() {
        let body = r#"[{"error":{"code":400,"message":"Request contains an invalid argument.","status":"INVALID_ARGUMENT"}}]"#;
        assert_eq!(
            error_text(body, 200).as_deref(),
            Some("Request contains an invalid argument.")
        );
    }

    #[test]
    fn a_plain_text_body_is_used_from_its_start() {
        assert_eq!(
            error_text("upstream connect error", 200).as_deref(),
            Some("upstream connect error")
        );
        assert_eq!(
            error_text("x".repeat(500).as_str(), 10).as_deref(),
            Some("xxxxxxxxxx")
        );
    }

    #[test]
    fn a_json_body_without_a_message_falls_back_to_its_text() {
        let body = r#"{"status":"nope"}"#;
        assert_eq!(error_text(body, 200).as_deref(), Some(body));
    }

    #[test]
    fn an_empty_body_has_no_message() {
        assert_eq!(error_text("", 200), None);
        assert_eq!(error_text("  \n\t ", 200), None);
        assert_eq!(
            error_text(r#"{"error":{"message":""}}"#, 200).as_deref(),
            Some(r#"{"error":{"message":""}}"#)
        );
    }

    #[test]
    fn the_message_is_scrubbed_too() {
        let body = format!(
            r#"{{"error":{{"message":"bad key {}"}}}}"#,
            shaped("sk-", "abcdefghijkl1234")
        );
        assert_eq!(
            error_text(&body, 200).as_deref(),
            Some("bad key [redacted]")
        );
    }
}
