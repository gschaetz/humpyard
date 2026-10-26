//! Making a tool call's arguments a JSON object (ADR 0029).
//!
//! A model can end a turn with a tool call whose arguments are cut off, and the client replays it every turn.
//! [`tool_arguments()`] says whether some argument text needs repair and, when it does, returns text that is a
//! JSON object: the cut-off object completed where that yields one, otherwise `{}`. Text that already is a JSON
//! object is never rewritten. The scan is one pass, takes no more than a bounded number of parses, and cannot
//! panic.

use serde_json::Value;

/// Longer invalid text is not completed; it becomes `{}`.
const MAX_COMPLETE_BYTES: usize = 256 * 1024;
/// Nesting deeper than this is not completed.
const MAX_DEPTH: usize = 64;
/// How many of the latest safe cut points are tried after the whole text fails.
const MAX_CUTS: usize = 16;

/// `None` when `text` already is a JSON object. Otherwise the text to send instead: `text` completed into an
/// object when it is a cut-off one, else `{}` (also for empty text, text that is not JSON and JSON of another
/// kind).
#[must_use]
pub fn tool_arguments(text: &str) -> Option<String> {
    let is_object = |candidate: &str| {
        matches!(
            serde_json::from_str::<Value>(candidate),
            Ok(Value::Object(_))
        )
    };
    if is_object(text) {
        return None;
    }
    let completed =
        if text.len() <= MAX_COMPLETE_BYTES && serde_json::from_str::<Value>(text).is_err() {
            complete(text.trim()).into_iter().find(|c| is_object(c))
        } else {
            None
        };
    Some(completed.unwrap_or_else(|| "{}".to_string()))
}

/// Where the text could be cut and still be closed into valid JSON: a byte offset and the closers that follow.
struct Cut {
    at: usize,
    closers: String,
}

/// Remembers a safe cut point, keeping only the latest [`MAX_CUTS`].
fn keep_cut(cuts: &mut Vec<Cut>, at: usize, stack: &[char]) {
    cuts.push(Cut {
        at,
        closers: stack.iter().rev().collect(),
    });
    if cuts.len() > MAX_CUTS {
        cuts.remove(0);
    }
}

/// Candidates, best first: the whole text closed, then the text cut back to each of the latest commas and opening
/// brackets and closed. Empty when the text is too deeply nested or its brackets do not match.
fn complete(text: &str) -> Vec<String> {
    let mut in_string = false;
    let mut escape_at: Option<usize> = None;
    // The start of a `\u` escape still waiting for hex digits, and how many it has.
    let mut unicode: Option<(usize, u8)> = None;
    let mut stack: Vec<char> = Vec::new();
    let mut cuts: Vec<Cut> = Vec::new();
    for (i, c) in text.char_indices() {
        if in_string {
            if let Some((start, seen)) = unicode {
                if c.is_ascii_hexdigit() {
                    unicode = (seen < 3).then_some((start, seen + 1));
                    continue;
                }
                unicode = None;
            }
            if escape_at.take().is_some() {
                if c == 'u' {
                    unicode = Some((i.saturating_sub(1), 0));
                }
                continue;
            }
            match c {
                '\\' => escape_at = Some(i),
                '"' => in_string = false,
                _ => {}
            }
            continue;
        }
        match c {
            '"' => in_string = true,
            '{' | '[' => {
                if stack.len() >= MAX_DEPTH {
                    return Vec::new();
                }
                stack.push(if c == '{' { '}' } else { ']' });
                keep_cut(&mut cuts, i + 1, &stack);
            }
            '}' | ']' => {
                if stack.pop() != Some(c) {
                    return Vec::new();
                }
            }
            ',' => keep_cut(&mut cuts, i, &stack),
            _ => {}
        }
    }
    // A string cut inside an escape loses the unfinished escape, then is closed.
    let mut end = text.len();
    if in_string {
        if let Some((start, _)) = unicode {
            end = start;
        } else if let Some(at) = escape_at {
            end = at;
        }
    }
    let closers: String = stack.iter().rev().collect();
    let mut candidates = Vec::with_capacity(cuts.len() + 1);
    if let Some(head) = text.get(..end) {
        candidates.push(format!(
            "{head}{}{closers}",
            if in_string { "\"" } else { "" }
        ));
    }
    for cut in cuts.iter().rev() {
        if let Some(head) = text.get(..cut.at) {
            candidates.push(format!("{head}{}", cut.closers));
        }
    }
    candidates
}

#[cfg(test)]
mod tests {
    use super::*;

    fn object(text: &str) -> Value {
        let value: Value = serde_json::from_str(text).unwrap_or(Value::Null);
        assert!(value.is_object(), "not an object: {text}");
        value
    }

    #[test]
    fn a_valid_object_is_left_alone_whatever_its_spacing() {
        for text in [
            "{}",
            r#"{"cmd":["ls","-la"]}"#,
            "  { \"a\" : 1 ,\n \"b\" : { \"c\" : [ ] } }  ",
            r#"{"a":1,"a":2}"#,
        ] {
            assert_eq!(tool_arguments(text), None, "{text}");
        }
    }

    #[test]
    fn a_cut_off_object_is_completed_and_keeps_what_was_whole() {
        let cases = [
            (
                r#"{"cmd":["ls","-la"],"workdir":"/tm"#,
                r#"{"cmd":["ls","-la"],"workdir":"/tm"}"#,
            ),
            (r#"{"a":1,"b":[1,2"#, r#"{"a":1,"b":[1,2]}"#),
            (r#"{"a":{"b":{"c":"d"#, r#"{"a":{"b":{"c":"d"}}}"#),
            (r#"{"a":12"#, r#"{"a":12}"#),
            (r#"{"a":true,"b":nul"#, r#"{"a":true}"#),
            (r#"{"a":1,"b""#, r#"{"a":1}"#),
            (r#"{"a":1,"#, r#"{"a":1}"#),
            (r#"{"a":1,"b":"#, r#"{"a":1}"#),
            (r#"{"a":{"b":"#, r#"{"a":{}}"#),
            (r#"{"a":"x\"#, r#"{"a":"x"}"#),
            (r#"{"a":"x\u12"#, r#"{"a":"x"}"#),
            (r#"{"a":"é"#, r#"{"a":"é"}"#),
        ];
        for (cut, want) in cases {
            let got = tool_arguments(cut).unwrap_or_default();
            assert_eq!(object(&got), object(want), "{cut} -> {got}");
        }
    }

    #[test]
    fn what_cannot_be_completed_becomes_an_empty_object() {
        for text in [
            "",
            "   ",
            "not json at all",
            "[]",
            "[1,2",
            r#""a string""#,
            "3",
            "null",
            "{]",
            r#"{"a":1}}"#,
        ] {
            assert_eq!(tool_arguments(text).as_deref(), Some("{}"), "{text}");
        }
    }

    #[test]
    fn limits_hold_and_nothing_panics() {
        let deep = format!("{}1", "{\"a\":".repeat(MAX_DEPTH + 1));
        assert_eq!(tool_arguments(&deep).as_deref(), Some("{}"));
        let long = format!("{{\"a\":\"{}", "x".repeat(MAX_COMPLETE_BYTES));
        assert_eq!(tool_arguments(&long).as_deref(), Some("{}"));
        for text in [
            "\\",
            "\"",
            "{\"\\u",
            "{\"\\",
            "{\u{0}",
            "é",
            "{\"é\":\"\\ué",
        ] {
            assert!(object(&tool_arguments(text).unwrap_or_default()).is_object());
        }
    }

    #[test]
    fn every_prefix_of_a_valid_object_ends_as_an_object() {
        let whole = [
            r#"{"cmd":["bash","-lc","git status --short"],"workdir":"/tmp/x","timeout_ms":10000}"#,
            r#"{"a":{"b":[{"c":"d\n\"q\"é"},{"e":[1,2.5,-3e2,true,false,null]}]},"z":"é日本"}"#,
            "{ \"a\" : [ 1 , 2 ] , \"b\" : \"x y\" }",
        ];
        for text in whole {
            assert_eq!(tool_arguments(text), None);
            for (end, _) in text.char_indices() {
                let prefix = &text[..end];
                match tool_arguments(prefix) {
                    None => {
                        object(prefix);
                    }
                    Some(fixed) => {
                        object(&fixed);
                    }
                }
            }
        }
    }

    #[test]
    fn a_prefix_keeps_the_keys_that_were_whole() {
        let text = r#"{"first":"one","second":"two","third":"thr"#;
        let fixed = object(&tool_arguments(text).unwrap_or_default());
        assert_eq!(fixed["first"], "one");
        assert_eq!(fixed["second"], "two");
    }
}
