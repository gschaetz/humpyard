use serde_json::{Value, json};

use super::{Edit, MAX_EDITS, PatchError, apply, masked_diff};

const SAMPLE: &str = r#"# The gateway.
listen = "127.0.0.1:0"

# The only provider.
[providers.p]
base_url = "http://127.0.0.1:1"   # local
api_key_env = "PATH"

# The first target.
[targets.t]
class = "public"
endpoints = [{ provider = "p", model = "m", context_window = 131072 }]

# The route.
[routes.r]
type = "passthrough"
targets = ["t"]

[keys.alice]
sha256 = "sha256:00000000000000000000000000000000000000000000000000000000000000aa"
allowed_routes = ["r"]
"#;

fn edit(value: Value) -> Edit {
    serde_json::from_value(value).unwrap()
}

fn run(text: &str, edits: Vec<Value>) -> Result<String, PatchError> {
    let edits: Vec<Edit> = edits.into_iter().map(edit).collect();
    apply(text, &edits)
}

fn reason(result: Result<String, PatchError>) -> String {
    match result {
        Err(PatchError::Invalid { reason, .. }) => reason,
        other => panic!("expected an invalid edit, got {other:?}"),
    }
}

/// How the lines of `after` differ from `before`: the lines only in `before`, and the lines only in `after`,
/// each in order. Everything else is the same line in the same place.
fn changed_lines(before: &str, after: &str) -> (Vec<String>, Vec<String>) {
    let diff = similar::TextDiff::from_lines(before, after);
    let mut removed = Vec::new();
    let mut added = Vec::new();
    for change in diff.iter_all_changes() {
        match change.tag() {
            similar::ChangeTag::Delete => removed.push(change.value().to_string()),
            similar::ChangeTag::Insert => added.push(change.value().to_string()),
            similar::ChangeTag::Equal => {}
        }
    }
    (removed, added)
}

fn parses(text: &str) -> toml::Table {
    text.parse().unwrap()
}

#[test]
fn setting_a_new_key_in_an_inline_table_changes_one_line_and_keeps_every_other() {
    let after = run(
        SAMPLE,
        vec![json!({"op": "set", "path": ["targets", "t", "endpoints", 0, "max_request_tokens"], "value": 8000})],
    )
    .unwrap();
    let (removed, added) = changed_lines(SAMPLE, &after);
    assert_eq!(removed.len(), 1, "{removed:?}");
    assert_eq!(added.len(), 1, "{added:?}");
    assert!(added[0].contains("max_request_tokens = 8000"), "{added:?}");
    assert!(added[0].contains("context_window = 131072"), "{added:?}");
    assert_eq!(
        parses(&after)["targets"]["t"]["endpoints"][0]["max_request_tokens"].as_integer(),
        Some(8000)
    );
}

#[test]
fn replacing_a_value_keeps_the_comment_after_it() {
    let after = run(
        SAMPLE,
        vec![json!({"op": "set", "path": ["providers", "p", "base_url"], "value": "http://127.0.0.1:2"})],
    )
    .unwrap();
    let (removed, added) = changed_lines(SAMPLE, &after);
    assert_eq!((removed.len(), added.len()), (1, 1));
    assert!(
        added[0].contains("http://127.0.0.1:2") && added[0].contains("# local"),
        "{added:?}"
    );
    // The comment lines in front of the table are still there.
    assert!(after.contains("# The only provider.\n[providers.p]"));
}

#[test]
fn a_new_key_in_a_standard_table_adds_exactly_one_line() {
    let after = run(
        SAMPLE,
        vec![json!({"op": "set", "path": ["targets", "t", "strengths"], "value": ["general", "all"]})],
    )
    .unwrap();
    let (removed, added) = changed_lines(SAMPLE, &after);
    assert!(removed.is_empty(), "{removed:?}");
    assert_eq!(added.len(), 1, "{added:?}");
    assert_eq!(
        parses(&after)["targets"]["t"]["strengths"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn removing_a_key_and_an_array_element_removes_only_that() {
    let after = run(
        SAMPLE,
        vec![
            json!({"op": "remove", "path": ["targets", "t", "class"]}),
            json!({"op": "remove", "path": ["routes", "r", "targets", 0]}),
        ],
    )
    .unwrap();
    let (removed, added) = changed_lines(SAMPLE, &after);
    assert_eq!(removed.len(), 2, "{removed:?}");
    assert_eq!(added.len(), 1, "{added:?}");
    assert!(added[0].contains("targets = []"), "{added:?}");
    assert!(parses(&after)["targets"]["t"].get("class").is_none());
}

#[test]
fn removing_a_table_takes_its_leading_comment_and_nothing_else() {
    let after = run(
        SAMPLE,
        vec![json!({"op": "remove_table", "path": ["routes", "r"]})],
    )
    .unwrap();
    assert!(
        !after.contains("[routes.r]") && !after.contains("# The route."),
        "{after}"
    );
    assert!(after.contains("# The first target.") && after.contains("[keys.alice]"));
    let (_, added) = changed_lines(SAMPLE, &after);
    assert!(added.is_empty(), "{added:?}");
}

#[test]
fn a_new_table_is_added_beside_its_section_and_nothing_else_changes() {
    let after = run(
        SAMPLE,
        vec![json!({"op": "add_table", "path": ["targets", "n"], "value": {
            "class": "public",
            "endpoints": [{"provider": "p", "model": "x", "price": {"input": 0.0, "output": 1.5}}]
        }})],
    )
    .unwrap();
    // It sits with the other targets, and not a line of the file is changed or lost.
    let (removed, added) = changed_lines(SAMPLE, &after);
    assert!(removed.is_empty(), "{removed:?}");
    assert_eq!(added.len(), 4, "{added:?}");
    assert!(
        after.find("[targets.t]").unwrap() < after.find("[targets.n]").unwrap()
            && after.find("[targets.n]").unwrap() < after.find("[routes.r]").unwrap(),
        "{after}"
    );
    let parsed = parses(&after);
    assert_eq!(parsed["targets"]["n"]["class"].as_str(), Some("public"));
    assert_eq!(
        parsed["targets"]["n"]["endpoints"][0]["price"]["output"].as_float(),
        Some(1.5)
    );
}

#[test]
fn the_first_table_of_an_absent_section_creates_the_section() {
    let after = run(
        SAMPLE,
        vec![json!({"op": "add_table", "path": ["profiles", "trial"], "value": {"daily_usd": 1}})],
    )
    .unwrap();
    assert!(after.starts_with(SAMPLE), "{after}");
    assert_eq!(
        parses(&after)["profiles"]["trial"]["daily_usd"].as_integer(),
        Some(1)
    );
    assert!(after.contains("[profiles.trial]"), "{after}");
}

#[test]
fn array_of_tables_files_are_edited_by_position_too() {
    let text = "[providers.p]\nbase_url = \"http://x\"\napi_key_env = \"PATH\"\n\n\
                [targets.t]\n\n[[targets.t.endpoints]]\nprovider = \"p\"\nmodel = \"m\"\n";
    let after = run(
        text,
        vec![json!({"op": "set", "path": ["targets", "t", "endpoints", 0, "model"], "value": "n"})],
    )
    .unwrap();
    assert_eq!(
        parses(&after)["targets"]["t"]["endpoints"][0]["model"].as_str(),
        Some("n")
    );
    let (removed, added) = changed_lines(text, &after);
    assert_eq!((removed.len(), added.len()), (1, 1));
}

#[test]
fn an_array_position_can_be_replaced() {
    let after = run(
        SAMPLE,
        vec![json!({"op": "set", "path": ["routes", "r", "targets", 0], "value": "u"})],
    )
    .unwrap();
    assert_eq!(
        parses(&after)["routes"]["r"]["targets"][0].as_str(),
        Some("u")
    );
    let (removed, added) = changed_lines(SAMPLE, &after);
    assert_eq!((removed.len(), added.len()), (1, 1));
}

#[test]
fn an_edit_that_changes_nothing_returns_the_text_unchanged() {
    let after = run(
        SAMPLE,
        vec![json!({"op": "set", "path": ["targets", "t", "class"], "value": "public"})],
    )
    .unwrap();
    assert_eq!(after, SAMPLE);
}

#[test]
fn sections_that_are_not_editable_are_refused_naming_the_edit() {
    for path in [
        json!(["admin", "credentials", "gary", "scopes"]),
        json!(["metrics", "enabled"]),
        json!(["ledger", "path"]),
        json!(["discovery", "file"]),
        json!(["listen"]),
        json!(["select", 0, "route"]),
        json!(["shutdown_grace_secs"]),
    ] {
        for op in ["set", "remove"] {
            let mut second = json!({"op": op, "path": path});
            if op == "set" {
                second["value"] = json!(1);
            }
            let edits = vec![
                json!({"op": "set", "path": ["targets", "t", "class"], "value": "private"}),
                second,
            ];
            let result = run(SAMPLE, edits);
            match result {
                Err(PatchError::Invalid { index, reason }) => {
                    assert_eq!(index, 1, "{path}");
                    assert!(
                        reason.contains("cannot be edited") || reason.contains("section"),
                        "{reason}"
                    );
                }
                other => panic!("{path} {op}: {other:?}"),
            }
        }
    }
    let table = reason(run(
        SAMPLE,
        vec![json!({"op": "add_table", "path": ["admin", "x"], "value": {}})],
    ));
    assert!(table.contains("cannot be edited"), "{table}");
}

#[test]
fn a_keys_hash_and_existence_cannot_be_edited() {
    for edit in [
        json!({"op": "set", "path": ["keys", "alice", "sha256"], "value": "sha256:ff"}),
        json!({"op": "remove", "path": ["keys", "alice", "sha256"]}),
        json!({"op": "set", "path": ["keys", "alice", "sha256", 0], "value": "x"}),
        json!({"op": "add_table", "path": ["keys", "bob"], "value": {}}),
        json!({"op": "remove_table", "path": ["keys", "alice"]}),
        json!({"op": "set", "path": ["keys", "alice"], "value": {}}),
        json!({"op": "remove", "path": ["keys", "alice"]}),
        json!({"op": "set", "path": ["keys"], "value": 1}),
    ] {
        assert!(
            matches!(
                run(SAMPLE, vec![edit.clone()]),
                Err(PatchError::Invalid { index: 0, .. })
            ),
            "{edit}"
        );
    }
    // A setting of a key can change.
    let after = run(
        SAMPLE,
        vec![
            json!({"op": "set", "path": ["keys", "alice", "allowed_routes"], "value": ["r", "s"]}),
        ],
    )
    .unwrap();
    assert_eq!(
        parses(&after)["keys"]["alice"]["allowed_routes"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn paths_that_do_not_fit_the_file_are_refused() {
    let cases = [
        (
            json!({"op": "set", "path": ["targets", "zzz", "class"], "value": "x"}),
            "nothing at",
        ),
        (
            json!({"op": "remove", "path": ["targets", "t", "nope"]}),
            "nothing there",
        ),
        (
            json!({"op": "remove", "path": ["targets", "t", "endpoints", 5]}),
            "nothing there",
        ),
        (
            json!({"op": "set", "path": ["targets", "t", "endpoints", 5], "value": 1}),
            "no position",
        ),
        (
            json!({"op": "set", "path": ["targets", "t"], "value": {}}),
            "is a table",
        ),
        (
            json!({"op": "remove", "path": ["targets", "t"]}),
            "remove_table",
        ),
        (
            json!({"op": "remove_table", "path": ["targets", "t", "class"]}),
            "not a table",
        ),
        (
            json!({"op": "remove_table", "path": ["targets", "zzz"]}),
            "no such table",
        ),
        (
            json!({"op": "add_table", "path": ["targets", "t"], "value": {}}),
            "already exists",
        ),
        (
            json!({"op": "add_table", "path": ["targets", "t", "endpoints", 0], "value": {}}),
            "not at an array position",
        ),
        (
            json!({"op": "add_table", "path": ["targets", "n"], "value": 3}),
            "must be an object",
        ),
        (
            json!({"op": "add_table", "path": ["targets", "t", "class", "x"], "value": {}}),
            "inside a standard table",
        ),
        (
            json!({"op": "set", "path": ["targets", "t", "class", "x"], "value": 1}),
            "not a table",
        ),
        (
            json!({"op": "add_table", "path": ["targets", "t", "endpoints", 0, "x"], "value": {}}),
            "inside a standard table",
        ),
    ];
    for (edit, expect) in cases {
        let message = reason(run(SAMPLE, vec![edit.clone()]));
        assert!(message.contains(expect), "{edit}: {message}");
    }
}

#[test]
fn names_and_values_that_could_be_syntax_stay_data() {
    let after = run(
        SAMPLE,
        vec![
            json!({"op": "set", "path": ["providers", "p", "base_url"], "value": "a\"b\nc] = [\n[admin]"}),
            json!({"op": "set", "path": ["providers", "p", "headers"], "value": {"x y": "1", "q\"r": "2"}}),
        ],
    )
    .unwrap();
    let parsed = parses(&after);
    assert_eq!(
        parsed["providers"]["p"]["base_url"].as_str(),
        Some("a\"b\nc] = [\n[admin]")
    );
    assert_eq!(
        parsed["providers"]["p"]["headers"]["x y"].as_str(),
        Some("1")
    );
    assert_eq!(
        parsed["providers"]["p"]["headers"]["q\"r"].as_str(),
        Some("2")
    );
    assert!(parsed.get("admin").is_none());
    // A path segment is a name, never syntax.
    for bad in ["", "a b", "a\"b", "a\nb", "a=b", "[x]", &"n".repeat(129)] {
        let result = run(
            SAMPLE,
            vec![json!({"op": "set", "path": ["targets", bad, "x"], "value": 1})],
        );
        assert!(
            matches!(result, Err(PatchError::Invalid { index: 0, .. })),
            "{bad:?}"
        );
    }
}

#[test]
fn values_have_limits_and_must_have_a_toml_form() {
    let set = |value: Value| {
        run(
            SAMPLE,
            vec![json!({"op": "set", "path": ["targets", "t", "x"], "value": value})],
        )
    };
    assert!(reason(set(Value::Null)).contains("null"));
    assert!(reason(set(json!(u64::MAX))).contains("too large"));
    assert!(set(json!(i64::MAX)).is_ok());
    assert!(set(json!(1.5)).is_ok());
    // Four arrays or objects inside one another is the most.
    assert!(set(json!([[[[1]]]])).is_ok());
    assert!(set(json!({"a": {"b": {"c": {"d": 1}}}})).is_ok());
    assert!(reason(set(json!([[[[[1]]]]]))).contains("deeper than 4"));
    assert!(reason(set(json!({"a": {"b": {"c": {"d": {"e": 1}}}}}))).contains("deeper than 4"));
    assert!(reason(set(json!({"": 1}))).contains("not a valid key"));
}

#[test]
fn the_edit_count_and_the_path_depth_have_limits() {
    let one = json!({"op": "set", "path": ["targets", "t", "class"], "value": "public"});
    assert!(run(SAMPLE, vec![one.clone(); MAX_EDITS]).is_ok());
    assert_eq!(
        run(SAMPLE, vec![one; MAX_EDITS + 1]),
        Err(PatchError::TooManyEdits)
    );
    let deep = |n: usize| {
        let mut path = vec![json!("targets")];
        path.extend((1..n).map(|i| json!(format!("k{i}"))));
        json!({"op": "remove", "path": path})
    };
    // Eight segments is the most; a missing path is a different refusal from a too-deep one.
    assert!(reason(run(SAMPLE, vec![deep(8)])).contains("nothing"));
    assert!(reason(run(SAMPLE, vec![deep(9)])).contains("deeper than 8"));
    assert!(reason(run(SAMPLE, vec![json!({"op": "remove", "path": []})])).contains("empty"));
    assert!(
        reason(run(SAMPLE, vec![json!({"op": "remove", "path": [0]})])).contains("section name")
    );
}

#[test]
fn nothing_applies_unless_every_edit_does() {
    let result = run(
        SAMPLE,
        vec![
            json!({"op": "set", "path": ["targets", "t", "class"], "value": "private"}),
            json!({"op": "remove", "path": ["targets", "t", "nope"]}),
        ],
    );
    assert!(
        matches!(result, Err(PatchError::Invalid { index: 1, .. })),
        "{result:?}"
    );
}

#[test]
fn a_file_that_does_not_parse_is_reported_not_edited() {
    let result = run(
        "[targets\nx = 1\n",
        vec![json!({"op": "remove", "path": ["targets", "x"]})],
    );
    assert!(
        matches!(result, Err(PatchError::Unparsable(_))),
        "{result:?}"
    );
}

#[test]
fn edits_come_from_json_only_in_the_documented_shapes() {
    assert!(
        serde_json::from_value::<Edit>(json!({"op": "set", "path": ["targets", 0], "value": 1}))
            .is_ok()
    );
    for bad in [
        json!({"op": "replace", "path": ["targets"], "value": 1}),
        json!({"op": "set", "path": ["targets"], "value": 1, "extra": true}),
        json!({"op": "set", "path": ["targets", -1], "value": 1}),
        json!({"op": "set", "path": ["targets", 1.5], "value": 1}),
        json!({"op": "set", "path": ["targets", true], "value": 1}),
        json!({"op": "set", "value": 1}),
        json!({"path": ["targets"]}),
    ] {
        assert!(
            serde_json::from_value::<Edit>(bad.clone()).is_err(),
            "{bad}"
        );
    }
}

const WITH_SECRETS: &str = r#"listen = "127.0.0.1:0"

[metrics]
enabled = true
bearer_sha256 = "sha256:1111111111111111111111111111111111111111111111111111111111111111"

[admin.credentials.gary]
sha256 = "sha256:2222222222222222222222222222222222222222222222222222222222222222"
scopes = ["read"]

[providers.p]
base_url = "http://127.0.0.1:1"
api_key_env = "PATH"
headers = { "x-session" = "inline-secret-1", "user-agent" = "humpyard" }

[providers.q]
base_url = "http://127.0.0.1:2"
api_key_env = "PATH"

[providers.q.headers]
x-team = "table-secret-2"

[providers.r]
base_url = "http://127.0.0.1:3"
api_key_env = "PATH"

[targets.t]
endpoints = [{ provider = "p", model = "m" }]

[routes.r]
type = "passthrough"
targets = ["t"]

[keys.alice]
sha256 = "sha256:3333333333333333333333333333333333333333333333333333333333333333"
allowed_routes = ["r"]
"#;

const SECRET_VALUES: [&str; 6] = [
    "1111111111111111",
    "2222222222222222",
    "3333333333333333",
    "inline-secret-1",
    "table-secret-2",
    "humpyard",
];

fn diff_of(before: &str, edits: Vec<Value>) -> String {
    let after = run(before, edits).unwrap();
    masked_diff(before, &after).unwrap()
}

#[test]
fn a_diff_has_three_lines_of_context_and_names_its_two_sides() {
    let diff = diff_of(
        SAMPLE,
        vec![json!({"op": "set", "path": ["targets", "t", "class"], "value": "private"})],
    );
    assert!(diff.starts_with("--- current\n+++ patched\n@@"), "{diff}");
    assert!(
        diff.contains("-class = \"public\"\n+class = \"private\""),
        "{diff}"
    );
    assert!(diff.contains("\n [targets.t]\n"), "{diff}");
    assert!(diff.contains("\n # The first target.\n"), "{diff}");
}

#[test]
fn a_change_that_changes_nothing_has_an_empty_diff_and_the_diff_is_stable() {
    let none = diff_of(
        SAMPLE,
        vec![json!({"op": "set", "path": ["targets", "t", "class"], "value": "public"})],
    );
    assert_eq!(none, "");
    let edits = || vec![json!({"op": "set", "path": ["routes", "r", "type"], "value": "random"})];
    assert_eq!(diff_of(SAMPLE, edits()), diff_of(SAMPLE, edits()));
}

#[test]
fn removing_a_provider_shows_its_header_lines_without_the_values() {
    let mut all = Vec::new();
    for provider in ["p", "q"] {
        all.push(diff_of(
            WITH_SECRETS,
            vec![json!({"op": "remove_table", "path": ["providers", provider]})],
        ));
    }
    assert!(
        all[0].contains(
            "-headers = { \"x-session\" = \"<redacted>\", \"user-agent\" = \"<redacted>\" }"
        ),
        "{}",
        all[0]
    );
    assert!(all[1].contains("-x-team = \"<redacted>\""), "{}", all[1]);
    for diff in &all {
        for secret in SECRET_VALUES {
            assert!(!diff.contains(secret), "{secret} in {diff}");
        }
    }
}

#[test]
fn a_diff_whose_context_holds_a_hash_masks_it() {
    // The context of an edit beside each hash includes the hash line.
    let diff = diff_of(
        WITH_SECRETS,
        vec![
            json!({"op": "set", "path": ["keys", "alice", "allowed_routes"], "value": ["r", "s"]}),
            json!({"op": "set", "path": ["providers", "r", "base_url"], "value": "http://127.0.0.1:9"}),
        ],
    );
    assert!(diff.contains(" sha256 = \"<redacted>\""), "{diff}");
    for secret in SECRET_VALUES {
        assert!(!diff.contains(secret), "{secret} in {diff}");
    }
    // Removing the sections that hold the other hashes shows no hash either.
    for table in [["metrics"].as_slice(), &["admin", "credentials", "gary"]] {
        let path: Vec<Value> = table.iter().map(|s| json!(s)).collect();
        let result = run(
            WITH_SECRETS,
            vec![json!({"op": "remove_table", "path": path})],
        );
        // `metrics` and `admin` are not editable, so the edit is refused and nothing could leak through it.
        assert!(
            matches!(result, Err(PatchError::Invalid { .. })),
            "{table:?}"
        );
    }
    // Their lines are masked in a diff of text that does change them (the file as an operator edits it).
    let edited = WITH_SECRETS.replace("scopes = [\"read\"]", "scopes = [\"read\", \"config\"]");
    let diff = masked_diff(WITH_SECRETS, &edited).unwrap();
    assert!(diff.contains(" sha256 = \"<redacted>\""), "{diff}");
    assert!(!diff.contains("2222222222222222"), "{diff}");
    let moved = WITH_SECRETS.replace("enabled = true", "enabled = false");
    let diff = masked_diff(WITH_SECRETS, &moved).unwrap();
    assert!(diff.contains("bearer_sha256 = \"<redacted>\""), "{diff}");
    assert!(!diff.contains("1111111111111111"), "{diff}");
}

#[test]
fn a_changed_secret_is_marked_changed_and_an_unchanged_one_is_not() {
    let diff = diff_of(
        WITH_SECRETS,
        vec![
            json!({"op": "set", "path": ["providers", "p", "headers", "x-session"], "value": "new-secret"}),
        ],
    );
    assert!(
        !diff.contains("new-secret") && !diff.contains("inline-secret-1"),
        "{diff}"
    );
    assert!(
        diff.contains("\"x-session\" = \"<redacted: changed>\""),
        "{diff}"
    );
    assert!(diff.contains("\"user-agent\" = \"<redacted>\""), "{diff}");
    // The same secret set to the same value is no change at all.
    let none = diff_of(
        WITH_SECRETS,
        vec![
            json!({"op": "set", "path": ["providers", "p", "headers", "x-session"], "value": "inline-secret-1"}),
        ],
    );
    assert_eq!(none, "");
}

#[test]
fn a_diff_of_text_that_does_not_parse_is_an_error() {
    assert!(matches!(
        masked_diff("[x", SAMPLE),
        Err(PatchError::Unparsable(_))
    ));
    assert!(matches!(
        masked_diff(SAMPLE, "[x"),
        Err(PatchError::Unparsable(_))
    ));
}
