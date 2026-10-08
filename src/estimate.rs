//! Prompt-size estimates for token counting. No provider offers a count call (they speak the
//! chat-completions protocol), so the gateway answers `count_tokens` locally. The ratio errs high on purpose:
//! overshooting a context window is worse than compacting a little early.

use serde_json::Value;

/// UTF-8 bytes per token assumed for message text. Real tokenizers average 3 to 5 for English and
/// 2 to 3 for code, JSON and non-Latin scripts; the constant is the calibrated high-side choice
/// (see `openspec/changes/archive/*-add-count-tokens/design.md`).
const BYTES_PER_TOKEN: u64 = 3;
/// Chat templates add role markers and separators around every message.
const TOKENS_PER_MESSAGE: u64 = 4;
/// Priming tokens every conversation starts with.
const BASE_TOKENS: u64 = 3;

/// Estimated prompt tokens of a chat-completions request body: its `messages` and `tools`.
#[must_use]
pub fn estimate_tokens(body: &Value) -> u64 {
    let mut total = BASE_TOKENS;
    if let Some(messages) = body.get("messages").and_then(Value::as_array) {
        for message in messages {
            total = total.saturating_add(TOKENS_PER_MESSAGE);
            total = total.saturating_add(tokens_for_bytes(text_bytes(message)));
        }
    }
    if let Some(tools) = body.get("tools") {
        // Tool schemas are tokenized as JSON, structure included.
        total = total.saturating_add(tokens_for_bytes(tools.to_string().len()));
    }
    total
}

/// Estimated tokens for `bytes` of generated text, with the same conservative ratio.
#[must_use]
pub fn tokens_for_bytes(bytes: usize) -> u64 {
    u64::try_from(bytes)
        .unwrap_or(u64::MAX)
        .div_ceil(BYTES_PER_TOKEN)
}

/// Bytes of the string values and object keys that carry meaning inside a message: text, tool
/// names, tool-call arguments and results. Structural noise (`"role":`, braces) is excluded.
fn text_bytes(value: &Value) -> usize {
    match value {
        Value::String(s) => s.len(),
        Value::Array(items) => items.iter().map(text_bytes).sum(),
        Value::Object(map) => map
            .iter()
            .filter(|(key, _)| !matches!(key.as_str(), "role" | "type" | "id" | "tool_call_id"))
            .map(|(_, v)| text_bytes(v))
            .sum(),
        Value::Number(n) => n.to_string().len(),
        Value::Bool(_) | Value::Null => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn chat(content: &str) -> Value {
        json!({"model": "m", "messages": [{"role": "user", "content": content}]})
    }

    #[test]
    fn an_empty_conversation_costs_only_the_base() {
        assert_eq!(estimate_tokens(&json!({})), BASE_TOKENS);
        assert_eq!(estimate_tokens(&json!({"messages": []})), BASE_TOKENS);
    }

    #[test]
    fn text_is_counted_by_bytes_rounded_up() {
        // 3 base + 4 per message + ceil(11 / 3) = 3 + 4 + 4
        assert_eq!(estimate_tokens(&chat("hello world")), 11);
        assert_eq!(estimate_tokens(&chat("")), BASE_TOKENS + TOKENS_PER_MESSAGE);
    }

    #[test]
    fn multi_byte_text_counts_its_bytes_not_its_characters() {
        let ascii = estimate_tokens(&chat("aaaaaa"));
        let accented = estimate_tokens(&chat("éééééé")); // six characters, twelve bytes
        assert!(accented > ascii);
    }

    #[test]
    fn structured_content_tool_calls_and_results_are_counted() {
        let body = json!({"messages": [
            {"role": "assistant", "content": null, "tool_calls": [
                {"id": "c1", "type": "function",
                 "function": {"name": "Bash", "arguments": "{\"command\":\"cargo test\"}"}}]},
            {"role": "tool", "tool_call_id": "c1", "content": "fatal runtime error: out of memory"},
            {"role": "user", "content": [{"type": "text", "text": "and then?"}]}
        ]});
        let with_all = estimate_tokens(&body);
        assert!(
            with_all > BASE_TOKENS + 3 * TOKENS_PER_MESSAGE + 10,
            "{with_all}"
        );
        // Role, type and id strings are structure, not content.
        let bare = json!({"messages": [{"role": "user", "content": "x"}]});
        assert_eq!(estimate_tokens(&bare), BASE_TOKENS + TOKENS_PER_MESSAGE + 1);
    }

    #[test]
    fn tool_definitions_add_their_json_size() {
        let without = estimate_tokens(&chat("hi"));
        let mut with = chat("hi");
        with["tools"] = json!([{"type": "function", "function": {
            "name": "get_weather", "description": "Get weather for a city",
            "parameters": {"type": "object", "properties": {"city": {"type": "string"}}}}}]);
        assert!(estimate_tokens(&with) > without + 20);
    }
}

#[cfg(test)]
mod properties {
    use super::*;
    use proptest::prelude::*;
    use serde_json::json;

    proptest! {
        #[test]
        fn never_panics_on_arbitrary_text(text in ".{0,2000}", extra in ".{0,200}") {
            let body = json!({"messages": [
                {"role": "user", "content": text},
                {"role": "tool", "content": [{"text": extra}]}]});
            let _ = estimate_tokens(&body);
        }

        #[test]
        fn more_content_never_counts_less(a in ".{0,500}", b in ".{0,500}") {
            let short = json!({"messages": [{"role": "user", "content": a.clone()}]});
            let long = json!({"messages": [{"role": "user", "content": format!("{a}{b}")}]});
            prop_assert!(estimate_tokens(&long) >= estimate_tokens(&short));
        }

        #[test]
        fn another_message_always_counts_more(content in ".{0,500}", n in 0usize..8) {
            let messages = |k: usize| json!({"messages": vec![json!({"role": "user", "content": content.clone()}); k]});
            prop_assert!(estimate_tokens(&messages(n + 1)) > estimate_tokens(&messages(n)));
        }
    }
}
