//! Judging an answer for four facts (ADR 0030): empty, cut by the length limit, a malformed tool call, a stream
//! the provider ended early.
//!
//! An [`Observation`] is fed what a response showed, chunk by chunk or from a whole buffered answer, and
//! [`Observation::judge()`] returns the [`Signals`]. Nothing here does I/O, and nothing keeps answer text,
//! reasoning text, a tool name or any argument beyond the time it takes to judge it: a name is kept as "was there
//! one", and argument text is dropped when the observation is. Memory is bounded by [`MAX_CALLS`] and
//! [`MAX_ARGUMENT_BYTES`] whatever the response size.

use crate::repair;

/// Tool calls looked at in one response; a call with a later index is not judged.
pub const MAX_CALLS: usize = 16;
/// Argument bytes kept per tool call; a call with more is not judged on its arguments.
pub const MAX_ARGUMENT_BYTES: usize = 64 * 1024;

/// How a response ended, as far as the provider's side is concerned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum End {
    /// A buffered answer, or a stream that reached its final chunk.
    Finished,
    /// The provider's stream ended without a final chunk.
    Ended,
    /// The provider's stream failed before its final chunk.
    Failed,
}

/// The four facts about one answer. `false` means the fact does not apply.
// Four independent facts, each stored and counted on its own; a state machine would misstate them.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Signals {
    /// Finished normally with no text and no tool call (reasoning alone is not an answer).
    pub empty_output: bool,
    /// The provider stopped the answer at its output limit.
    pub truncated_output: bool,
    /// A tool call with no name, or with arguments that are neither blank nor a JSON object.
    pub bad_tool_call: bool,
    /// The provider's stream ended or failed before its final chunk.
    pub cut_stream: bool,
}

impl Signals {
    /// No fact applies: a good answer.
    #[must_use]
    pub fn is_clean(self) -> bool {
        self == Self::default()
    }
}

/// One tool call as accumulated from its fragments.
#[derive(Debug, Default)]
struct Call {
    index: u64,
    named: bool,
    arguments: String,
    /// More than [`MAX_ARGUMENT_BYTES`] arrived; the text was dropped and the arguments are not judged.
    oversize: bool,
}

/// What a response has shown so far.
#[derive(Debug, Default)]
pub struct Observation {
    text: bool,
    tool_call: bool,
    length_stop: bool,
    calls: Vec<Call>,
}

impl Observation {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Answer text arrived. Blank text is not an answer.
    pub fn text(&mut self, fragment: &str) {
        if !fragment.trim().is_empty() {
            self.text = true;
        }
    }

    /// Some other content that is an answer by itself (a refusal, an image): the answer is not empty.
    pub fn other_content(&mut self) {
        self.text = true;
    }

    /// A fragment of tool call `index`: its name when the fragment carries one, and a piece of its arguments.
    /// A call with an index past the first [`MAX_CALLS`] distinct ones still counts as a tool call but is not
    /// judged.
    pub fn tool_call(&mut self, index: u64, name: Option<&str>, arguments: &str) {
        self.tool_call = true;
        let at = match self.calls.iter().position(|c| c.index == index) {
            Some(at) => at,
            None if self.calls.len() < MAX_CALLS => {
                self.calls.push(Call {
                    index,
                    ..Call::default()
                });
                self.calls.len() - 1
            }
            None => return,
        };
        let Some(call) = self.calls.get_mut(at) else {
            return;
        };
        if name.is_some_and(|n| !n.trim().is_empty()) {
            call.named = true;
        }
        if call.oversize {
            return;
        }
        if call.arguments.len().saturating_add(arguments.len()) > MAX_ARGUMENT_BYTES {
            call.oversize = true;
            call.arguments = String::new();
        } else {
            call.arguments.push_str(arguments);
        }
    }

    /// The answer was stopped by its output limit (a buffered answer's normalized stop reason).
    pub fn length_stop(&mut self) {
        self.length_stop = true;
    }

    /// The provider's own stop string. The spellings of a length stop are matched case-insensitively; any
    /// other string, known or not, sets nothing.
    pub fn stop_string(&mut self, stop: &str) {
        let stop = stop.trim();
        if ["length", "max_tokens", "max_output_tokens"]
            .iter()
            .any(|s| stop.eq_ignore_ascii_case(s))
        {
            self.length_stop = true;
        }
    }

    /// The four facts, given how the response ended.
    #[must_use]
    pub fn judge(&self, end: End) -> Signals {
        Signals {
            empty_output: end == End::Finished && !self.text && !self.tool_call,
            truncated_output: self.length_stop,
            bad_tool_call: self.calls.iter().any(Call::is_malformed),
            cut_stream: end != End::Finished,
        }
    }
}

impl Call {
    fn is_malformed(&self) -> bool {
        if !self.named {
            return true;
        }
        if self.oversize || self.arguments.trim().is_empty() {
            return false;
        }
        repair::tool_arguments(&self.arguments).is_some()
    }
}
