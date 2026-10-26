//! Applying [`crate::repair::tool_arguments()`] to a request (ADR 0029).
//!
//! The text a provider receives comes from two places: the decoded tool calls, which the encoder writes out, and
//! the preserved same-format body, which is sent as the client wrote it. Both are repaired, so the result does not
//! depend on the client's protocol or the provider's. Only an assistant tool call's `arguments` is ever touched.

use serde_json::{Map, Value};
use switchyard_protocol::{ContentBlock, LlmRequest, Role};

use crate::repair::tool_arguments;

/// Makes every assistant tool call's arguments in `request` a JSON object, in the decoded messages and in each
/// preserved body. Returns how many calls were repaired; arguments that already are an object are left as they are
/// and not counted. Both forms describe the same calls, so the count is the larger of the two: a chat request
/// decodes bad text as `{"raw": ...}`, which already is an object, while its preserved body still holds the
/// invalid string that would be sent.
pub fn tool_calls(request: &mut LlmRequest) -> u32 {
    let mut repaired = 0u32;
    for message in request
        .messages
        .iter_mut()
        .filter(|m| m.role == Role::Assistant)
    {
        for block in &mut message.content {
            if let ContentBlock::ToolCall(call) = block
                && repair_decoded(&mut call.arguments)
            {
                repaired = repaired.saturating_add(1);
            }
        }
    }
    let preserved = request
        .preservation
        .requests
        .values_mut()
        .map(repair_body)
        .max()
        .unwrap_or(0);
    repaired.max(preserved)
}

/// A decoded call's arguments: an object stays, text is repaired and parsed, any other value becomes `{}`.
fn repair_decoded(arguments: &mut Value) -> bool {
    let fixed = match arguments {
        Value::Object(_) => return false,
        Value::String(text) => match tool_arguments(text) {
            Some(fixed) => fixed,
            None => return false,
        },
        _ => "{}".to_string(),
    };
    *arguments = serde_json::from_str(&fixed).unwrap_or_else(|_| Value::Object(Map::new()));
    true
}

/// A preserved body: a chat body's assistant `tool_calls[].function.arguments` and a Responses body's
/// `function_call` items. A string that is already an object is not rewritten, and nothing else is changed.
fn repair_body(body: &mut Value) -> u32 {
    let mut repaired = 0u32;
    if let Some(messages) = body.get_mut("messages").and_then(Value::as_array_mut) {
        for message in messages
            .iter_mut()
            .filter(|m| m.get("role").and_then(Value::as_str) == Some("assistant"))
        {
            let calls = message.get_mut("tool_calls").and_then(Value::as_array_mut);
            for call in calls.into_iter().flatten() {
                if let Some(arguments) = call
                    .get_mut("function")
                    .and_then(|f| f.get_mut("arguments"))
                {
                    repaired = repaired.saturating_add(u32::from(repair_text(arguments)));
                }
            }
        }
    }
    if let Some(items) = body.get_mut("input").and_then(Value::as_array_mut) {
        for item in items
            .iter_mut()
            .filter(|i| i.get("type").and_then(Value::as_str) == Some("function_call"))
        {
            if let Some(arguments) = item.get_mut("arguments") {
                repaired = repaired.saturating_add(u32::from(repair_text(arguments)));
            }
        }
    }
    repaired
}

/// Replaces a string that is not an object, and says whether it did.
fn repair_text(arguments: &mut Value) -> bool {
    if let Value::String(text) = arguments
        && let Some(fixed) = tool_arguments(text)
    {
        *arguments = Value::String(fixed);
        return true;
    }
    false
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test scaffolding
mod tests {
    use super::*;
    use serde_json::json;
    use switchyard_protocol::WireFormat;
    use switchyard_translation::{decode_request, encode_request};

    const CUT: &str = r#"{"cmd":["bash","-lc","ls -la"],"workdir":"/tm"#;

    fn responses_request(arguments: &str) -> Value {
        json!({
            "model": "m",
            "input": [
                {"type": "message", "role": "user", "content": [{"type": "input_text", "text": "{\"a\":"}]},
                {"type": "function_call", "call_id": "c1", "name": "shell", "arguments": arguments},
                {"type": "function_call_output", "call_id": "c1", "output": "{\"not\":"},
                {"type": "message", "role": "user", "content": [{"type": "input_text", "text": "go on"}]}
            ]
        })
    }

    fn chat_request(arguments: &str) -> Value {
        json!({
            "model": "m",
            "messages": [
                {"role": "user", "content": "{\"a\":"},
                {"role": "assistant", "content": null, "tool_calls": [
                    {"id": "c1", "type": "function", "function": {"name": "shell", "arguments": arguments}}
                ]},
                {"role": "tool", "tool_call_id": "c1", "content": "{\"not\":"},
                {"role": "user", "content": "go on"}
            ]
        })
    }

    fn decode(format: WireFormat, body: &Value) -> LlmRequest {
        decode_request(format, body).unwrap()
    }

    /// The `arguments` strings of every assistant tool call in a chat-encoded request.
    fn encoded_arguments(request: &LlmRequest) -> Vec<String> {
        let body = encode_request(request, WireFormat::OpenAiChat).unwrap();
        body["messages"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|m| m["tool_calls"].as_array())
            .flatten()
            .map(|c| c["function"]["arguments"].as_str().unwrap().to_string())
            .collect()
    }

    fn is_object(text: &str) -> bool {
        serde_json::from_str::<Value>(text).is_ok_and(|v| v.is_object())
    }

    #[test]
    fn a_cut_off_call_in_a_responses_request_reaches_a_chat_provider_as_an_object() {
        let mut request = decode(WireFormat::OpenAiResponses, &responses_request(CUT));
        assert!(!encoded_arguments(&request).iter().all(|a| is_object(a)));
        assert_eq!(tool_calls(&mut request), 1);
        let sent = encoded_arguments(&request);
        assert_eq!(sent.len(), 1);
        let object: Value = serde_json::from_str(&sent[0]).unwrap();
        assert_eq!(object["workdir"], "/tm");
        assert_eq!(object["cmd"], json!(["bash", "-lc", "ls -la"]));
    }

    #[test]
    fn a_cut_off_call_in_a_chat_request_is_repaired_in_both_forms() {
        let mut request = decode(WireFormat::OpenAiChat, &chat_request(CUT));
        assert_eq!(tool_calls(&mut request), 1);
        assert!(encoded_arguments(&request).iter().all(|a| is_object(a)));
        // The preserved body is what a chat provider is sent as the client wrote it.
        let preserved = &request.preservation.requests[&WireFormat::OpenAiChat.into()];
        let sent = preserved["messages"][1]["tool_calls"][0]["function"]["arguments"]
            .as_str()
            .unwrap();
        assert!(is_object(sent), "{sent}");
        // Nothing else in it moved.
        let mut expected = chat_request(CUT);
        expected["messages"][1]["tool_calls"][0]["function"]["arguments"] = json!(sent);
        assert_eq!(preserved, &expected);
    }

    #[test]
    fn a_preserved_responses_body_is_repaired_and_the_rest_is_as_sent() {
        let mut request = decode(WireFormat::OpenAiResponses, &responses_request(CUT));
        tool_calls(&mut request);
        let preserved = &request.preservation.requests[&WireFormat::OpenAiResponses.into()];
        let sent = preserved["input"][1]["arguments"].as_str().unwrap();
        assert!(is_object(sent), "{sent}");
        let mut expected = responses_request(CUT);
        expected["input"][1]["arguments"] = json!(sent);
        assert_eq!(preserved, &expected);
    }

    #[test]
    fn valid_arguments_are_untouched_and_not_counted() {
        for format in [WireFormat::OpenAiChat, WireFormat::OpenAiResponses] {
            let spaced = "{ \"cmd\" : [\"ls\"] ,\n \"workdir\":\"/tmp\" }";
            let body = if format == WireFormat::OpenAiChat {
                chat_request(spaced)
            } else {
                responses_request(spaced)
            };
            let mut request = decode(format, &body);
            let before = request.clone();
            assert_eq!(tool_calls(&mut request), 0);
            assert_eq!(request.preservation, before.preservation);
            assert_eq!(request.messages, before.messages);
            assert_eq!(encoded_arguments(&request), encoded_arguments(&before));
        }
    }

    #[test]
    fn arguments_that_are_not_an_object_become_an_empty_one() {
        for text in ["", "not json", "[1,2]", "\"x\"", "null"] {
            let mut request = decode(WireFormat::OpenAiResponses, &responses_request(text));
            assert_eq!(tool_calls(&mut request), 1, "{text:?}");
            assert_eq!(
                encoded_arguments(&request),
                vec!["{}".to_string()],
                "{text:?}"
            );
        }
    }

    #[test]
    fn only_tool_call_arguments_are_touched() {
        // Text, a tool result and a user message that look like broken JSON are as the client sent them.
        let mut request = decode(WireFormat::OpenAiChat, &chat_request(r#"{"ok":true}"#));
        let before = request.clone();
        assert_eq!(tool_calls(&mut request), 0);
        assert_eq!(request.preservation, before.preservation);
        let body = &request.preservation.requests[&WireFormat::OpenAiChat.into()];
        assert_eq!(body["messages"][0]["content"], "{\"a\":");
        assert_eq!(body["messages"][2]["content"], "{\"not\":");
    }

    #[test]
    fn a_request_with_no_tool_calls_is_unchanged() {
        let body = json!({"model": "m", "messages": [{"role": "user", "content": "hi"}]});
        let mut request = decode(WireFormat::OpenAiChat, &body);
        let before = request.clone();
        assert_eq!(tool_calls(&mut request), 0);
        assert_eq!(request.preservation, before.preservation);
        assert_eq!(request.messages, before.messages);
    }

    #[test]
    fn each_cut_off_call_in_a_history_is_counted() {
        let body = json!({
            "model": "m",
            "messages": [
                {"role": "user", "content": "go"},
                {"role": "assistant", "content": null, "tool_calls": [
                    {"id": "a", "type": "function", "function": {"name": "f", "arguments": CUT}},
                    {"id": "b", "type": "function", "function": {"name": "f", "arguments": "{\"x\":1}"}},
                    {"id": "c", "type": "function", "function": {"name": "f", "arguments": "{\"y\":"}}
                ]}
            ]
        });
        let mut request = decode(WireFormat::OpenAiChat, &body);
        assert_eq!(tool_calls(&mut request), 2);
        assert!(encoded_arguments(&request).iter().all(|a| is_object(a)));
    }
}
