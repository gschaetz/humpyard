//! The judge of an answer's four facts (spec: `response-signals`, change `record-response-quality-signals`,
//! ADR 0030), one table row per scenario of the spec, then the bounds on what it reads.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test scaffolding

use humpyard::signals::{End, MAX_ARGUMENT_BYTES, MAX_CALLS, Observation, Signals};

const NONE: Signals = Signals {
    empty_output: false,
    truncated_output: false,
    bad_tool_call: false,
    cut_stream: false,
};

fn only(f: impl FnOnce(&mut Signals)) -> Signals {
    let mut s = NONE;
    f(&mut s);
    s
}

/// A call with a name and `arguments` as one fragment.
fn call(o: &mut Observation, arguments: &str) {
    o.tool_call(0, Some("shell"), arguments);
}

#[test]
fn a_good_answer_carries_no_flag() {
    let mut o = Observation::new();
    o.text("hello");
    o.stop_string("stop");
    let s = o.judge(End::Finished);
    assert_eq!(s, NONE);
    assert!(s.is_clean());
}

#[test]
fn a_good_tool_call_carries_no_flag() {
    let mut o = Observation::new();
    call(&mut o, r#"{"cmd":"ls"}"#);
    assert_eq!(o.judge(End::Finished), NONE);
}

#[test]
fn an_empty_answer_is_empty_and_nothing_else() {
    let o = Observation::new();
    assert_eq!(o.judge(End::Finished), only(|s| s.empty_output = true));
}

#[test]
fn blank_text_is_not_an_answer() {
    let mut o = Observation::new();
    o.text(" \n");
    assert!(o.judge(End::Finished).empty_output);
}

#[test]
fn a_reasoning_only_answer_is_empty() {
    // Reasoning is never fed to the observation: it is not an answer, so there is nothing to feed.
    let o = Observation::new();
    assert!(o.judge(End::Finished).empty_output);
}

#[test]
fn a_cut_stream_is_not_called_empty() {
    for end in [End::Ended, End::Failed] {
        let s = Observation::new().judge(end);
        assert_eq!(s, only(|s| s.cut_stream = true), "{end:?}");
    }
}

#[test]
fn a_length_stop_is_truncated_in_every_spelling() {
    for stop in [
        "length",
        "LENGTH",
        "max_tokens",
        "Max_Tokens",
        "max_output_tokens",
        " length ",
    ] {
        let mut o = Observation::new();
        o.text("partial");
        o.stop_string(stop);
        assert_eq!(
            o.judge(End::Finished),
            only(|s| s.truncated_output = true),
            "{stop}"
        );
    }
    let mut buffered = Observation::new();
    buffered.text("partial");
    buffered.length_stop();
    assert!(buffered.judge(End::Finished).truncated_output);
}

#[test]
fn an_unknown_or_normal_stop_string_is_not_truncated() {
    for stop in [
        "stop",
        "end_turn",
        "tool_calls",
        "content_filter",
        "mystery",
        "",
    ] {
        let mut o = Observation::new();
        o.text("fine");
        o.stop_string(stop);
        assert_eq!(o.judge(End::Finished), NONE, "{stop}");
    }
}

#[test]
fn bad_tool_call_cases() {
    let bad = only(|s| s.bad_tool_call = true);
    for (arguments, want) in [
        (r#"{"cmd":"ls"}"#, NONE),
        ("{}", NONE),
        // Blank arguments are sent for a tool that takes none.
        ("", NONE),
        ("  \n", NONE),
        // Cut off in the middle of an object.
        (r#"{"cmd":["bash","-lc","ls"],"workdir":"/tm"#, bad),
        ("{\"a\":", bad),
        // Valid JSON that is not an object.
        ("[1,2]", bad),
        ("\"text\"", bad),
        ("null", bad),
        ("not json", bad),
    ] {
        let mut o = Observation::new();
        call(&mut o, arguments);
        assert_eq!(o.judge(End::Finished), want, "{arguments:?}");
    }
}

#[test]
fn a_tool_call_with_no_name_is_bad() {
    for name in [None, Some(""), Some("  ")] {
        let mut o = Observation::new();
        o.tool_call(0, name, "{}");
        assert_eq!(
            o.judge(End::Finished),
            only(|s| s.bad_tool_call = true),
            "{name:?}"
        );
    }
}

#[test]
fn a_name_in_a_later_fragment_counts() {
    let mut o = Observation::new();
    o.tool_call(0, None, r#"{"a":"#);
    o.tool_call(0, Some("shell"), "1}");
    assert_eq!(o.judge(End::Finished), NONE);
}

#[test]
fn fragments_of_one_call_are_joined_and_calls_are_kept_apart() {
    let mut o = Observation::new();
    o.tool_call(0, Some("a"), r#"{"x":"#);
    o.tool_call(1, Some("b"), r#"{"y":2}"#);
    o.tool_call(0, None, "1}");
    assert_eq!(o.judge(End::Finished), NONE);

    o.tool_call(1, None, "garbage");
    assert!(o.judge(End::Finished).bad_tool_call);
}

#[test]
fn a_tool_call_is_an_answer_even_with_no_text() {
    let mut o = Observation::new();
    call(&mut o, "{}");
    assert!(!o.judge(End::Finished).empty_output);
}

#[test]
fn a_stream_the_provider_cut_is_flagged_whatever_it_held() {
    let mut o = Observation::new();
    o.text("part of an ans");
    for end in [End::Ended, End::Failed] {
        assert_eq!(o.judge(end), only(|s| s.cut_stream = true), "{end:?}");
    }
}

#[test]
fn a_cut_stream_with_cut_off_arguments_carries_both_facts() {
    let mut o = Observation::new();
    call(&mut o, r#"{"cmd":"l"#);
    assert_eq!(
        o.judge(End::Ended),
        only(|s| {
            s.cut_stream = true;
            s.bad_tool_call = true;
        })
    );
}

#[test]
fn a_call_past_the_byte_bound_is_not_flagged() {
    let mut o = Observation::new();
    let piece = "x".repeat(MAX_ARGUMENT_BYTES / 4);
    o.tool_call(0, Some("shell"), "{\"a\":\"");
    // 1 MiB of a string that never closes: invalid, but past the bound, so not looked at.
    for _ in 0..(1024 * 1024 / piece.len()) {
        o.tool_call(0, None, &piece);
    }
    assert_eq!(o.judge(End::Finished), NONE);
}

#[test]
fn a_call_exactly_at_the_byte_bound_is_still_judged() {
    let mut o = Observation::new();
    o.tool_call(0, Some("shell"), &"x".repeat(MAX_ARGUMENT_BYTES));
    assert!(o.judge(End::Finished).bad_tool_call);
}

#[test]
fn a_call_past_the_byte_bound_still_needs_a_name() {
    let mut o = Observation::new();
    o.tool_call(0, None, &"x".repeat(MAX_ARGUMENT_BYTES + 1));
    assert!(o.judge(End::Finished).bad_tool_call);
}

#[test]
fn calls_past_the_count_bound_are_not_judged_and_do_not_disturb_the_rest() {
    let mut o = Observation::new();
    for i in 0..40 {
        // Every call is malformed; only the first MAX_CALLS are looked at.
        o.tool_call(i, Some("shell"), "{\"a\":");
    }
    assert!(o.judge(End::Finished).bad_tool_call);

    // Valid calls inside the bound, malformed ones only outside it: no flag.
    let mut o = Observation::new();
    for i in 0..40u64 {
        let arguments = if usize::try_from(i).unwrap() < MAX_CALLS {
            "{}"
        } else {
            "{\"a\":"
        };
        o.tool_call(i, Some("shell"), arguments);
    }
    let s = o.judge(End::Finished);
    assert_eq!(s, NONE);
    assert!(!s.empty_output, "a call past the bound is still a call");
}

#[test]
fn the_bounded_state_holds_at_most_the_bound() {
    let mut o = Observation::new();
    for i in 0..1000u64 {
        o.tool_call(i, Some("shell"), &"y".repeat(MAX_ARGUMENT_BYTES));
    }
    let held = format!("{o:?}");
    // The debug form lists every kept call; it cannot hold more than the bound's worth of text.
    assert!(
        held.len() <= MAX_CALLS * (MAX_ARGUMENT_BYTES + 200) + 200,
        "{} bytes kept",
        held.len()
    );
}
