//! Route selection: ordered rules that choose the route from facts about the request (the key,
//! the requested model, client headers, agent metadata). Pure matching with no HTTP or Switchyard
//! types, so it is easy to test. See ADR 0011: clients can narrow, never widen.

use std::collections::HashMap;

use crate::config::{SelectorSpec, When};

/// The standard header carrying a client's free-form profile name.
pub const PROFILE_HEADER: &str = "x-humpyard-profile";
/// Prefix of the standard tag headers: `x-humpyard-tag-<name>`.
pub const TAG_HEADER_PREFIX: &str = "x-humpyard-tag-";

/// What a rule may look at.
pub struct Facts<'a> {
    /// The model (route or target) the client asked for.
    pub model: &'a str,
    /// The authenticated key's id; `None` when the gateway is open.
    pub key: Option<&'a str>,
    /// Request headers, names lower-cased, credential headers already removed.
    pub headers: &'a HashMap<String, String>,
    pub agent: Option<&'a str>,
    pub task: Option<&'a str>,
    pub subagent: bool,
    pub stream: bool,
}

/// What one rule did for a request: for `explain`, so operators can see why a rule did or did not
/// apply.
#[derive(Debug, PartialEq, Eq)]
pub struct RuleTrace<'a> {
    pub rule: &'a str,
    pub route: &'a str,
    /// Why the conditions failed; empty when they all held.
    pub mismatches: Vec<String>,
    /// For a matching rule, whether the key may use its route; `None` when it did not match.
    pub permitted: Option<bool>,
}

/// The rule that applied and the route it chose.
#[derive(Debug, PartialEq, Eq)]
pub struct Chosen<'a> {
    pub rule: &'a str,
    pub route: &'a str,
}

struct Rule {
    label: String,
    when: When,
    route: String,
}

pub struct Selectors {
    rules: Vec<Rule>,
}

impl Selectors {
    pub fn new(specs: &[SelectorSpec]) -> Self {
        let rules = specs
            .iter()
            .enumerate()
            .map(|(index, spec)| Rule {
                label: spec
                    .name
                    .clone()
                    .unwrap_or_else(|| format!("select[{index}]")),
                when: normalized(&spec.when),
                route: spec.route.clone(),
            })
            .collect();
        Self { rules }
    }

    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    /// Every rule's outcome for `facts`, in order. The chosen rule is the first with no mismatches
    /// and `permitted == Some(true)`.
    pub fn trace<'a>(
        &'a self,
        facts: &Facts<'_>,
        permitted: impl Fn(&str) -> bool,
    ) -> Vec<RuleTrace<'a>> {
        self.rules
            .iter()
            .map(|rule| {
                let mismatches = mismatches(&rule.when, facts);
                let permitted = mismatches.is_empty().then(|| permitted(&rule.route));
                RuleTrace {
                    rule: &rule.label,
                    route: &rule.route,
                    mismatches,
                    permitted,
                }
            })
            .collect()
    }

    /// The first rule that matches `facts` and whose route `permitted` accepts. A matching rule
    /// whose route is not permitted is skipped, never an error: a client's headers must not turn a
    /// working request into a refusal or reach a route the key may not use.
    pub fn choose<'a>(
        &'a self,
        facts: &Facts<'_>,
        permitted: impl Fn(&str) -> bool,
    ) -> Option<Chosen<'a>> {
        self.rules
            .iter()
            .find(|rule| matches(&rule.when, facts) && permitted(&rule.route))
            .map(|rule| Chosen {
                rule: &rule.label,
                route: &rule.route,
            })
    }
}

/// Header and tag names are compared lower-cased.
fn normalized(when: &When) -> When {
    let mut when = when.clone();
    when.header = when
        .header
        .into_iter()
        .map(|(k, v)| (k.to_ascii_lowercase(), v))
        .collect();
    when.tag = when
        .tag
        .into_iter()
        .map(|(k, v)| (k.to_ascii_lowercase(), v))
        .collect();
    when
}

fn matches(when: &When, facts: &Facts<'_>) -> bool {
    mismatches(when, facts).is_empty()
}

/// Why a text condition fails, or `None` when it holds or is not set.
fn text_mismatch(label: &str, pattern: Option<&str>, value: Option<&str>) -> Option<String> {
    let pattern = pattern?;
    if value.is_some_and(|v| glob(pattern, v)) {
        return None;
    }
    Some(match value {
        Some(v) => format!("{label}: wanted `{pattern}`, got `{v}`"),
        None => format!("{label}: wanted `{pattern}`, but the request has none"),
    })
}

/// Which conditions of `when` do not hold for `facts`, each described in a sentence fragment.
fn mismatches(when: &When, facts: &Facts<'_>) -> Vec<String> {
    let header = |name: &str| facts.headers.get(name).map(String::as_str);
    let mut out: Vec<String> = [
        text_mismatch("model", when.model.as_deref(), Some(facts.model)),
        text_mismatch("key", when.key.as_deref(), facts.key),
        text_mismatch("profile", when.profile.as_deref(), header(PROFILE_HEADER)),
        text_mismatch("agent", when.agent.as_deref(), facts.agent),
        text_mismatch("task", when.task.as_deref(), facts.task),
    ]
    .into_iter()
    .flatten()
    .collect();
    for (label, wanted, actual) in [
        ("subagent", when.subagent, facts.subagent),
        ("stream", when.stream, facts.stream),
    ] {
        if let Some(wanted) = wanted
            && wanted != actual
        {
            out.push(format!("{label}: wanted {wanted}, got {actual}"));
        }
    }
    out.extend(when.header.iter().filter_map(|(name, pattern)| {
        text_mismatch(&format!("header {name}"), Some(pattern), header(name))
    }));
    out.extend(when.tag.iter().filter_map(|(name, pattern)| {
        let header_name = format!("{TAG_HEADER_PREFIX}{name}");
        text_mismatch(&format!("tag {name}"), Some(pattern), header(&header_name))
    }));
    out
}

/// Case-sensitive match where `*` stands for any run of characters (including none).
#[must_use]
pub fn glob(pattern: &str, text: &str) -> bool {
    let parts: Vec<&str> = pattern.split('*').collect();
    if parts.len() == 1 {
        return pattern == text;
    }
    let (first, last) = (parts[0], parts[parts.len() - 1]);
    if !text.starts_with(first) || text.len() < first.len() + last.len() || !text.ends_with(last) {
        return false;
    }
    let mut rest = &text[first.len()..text.len() - last.len()];
    for part in &parts[1..parts.len() - 1] {
        match rest.find(part) {
            Some(at) => rest = &rest[at + part.len()..],
            None => return false,
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    fn rule(name: &str, when: When, route: &str) -> SelectorSpec {
        SelectorSpec {
            name: Some(name.into()),
            when,
            route: route.into(),
        }
    }

    fn facts(headers: &HashMap<String, String>) -> Facts<'_> {
        Facts {
            model: "agent",
            key: Some("alice"),
            headers,
            agent: Some("planner"),
            task: Some("t-1"),
            subagent: false,
            stream: true,
        }
    }

    fn headers(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).into(), (*v).into()))
            .collect()
    }

    #[test]
    fn globs_match_the_way_operators_expect() {
        assert!(glob("ci-*", "ci-nightly"));
        assert!(glob("*-bot", "deploy-bot"));
        assert!(glob("a*b*c", "a--b--c"));
        assert!(glob("*", ""));
        assert!(glob("exact", "exact"));
        assert!(!glob("exact", "exactly"));
        assert!(!glob("ci-*", "ui-nightly"));
        assert!(!glob("a*b", "ab-"));
        assert!(!glob("ab*ba", "aba"), "prefix and suffix must not overlap");
        assert!(glob("ab*ba", "abba"));
        assert!(!glob("x*", "X"), "case-sensitive");
    }

    #[test]
    fn the_first_matching_rule_wins_and_no_match_is_none() {
        let specs = [
            rule(
                "a",
                When {
                    key: Some("bob".into()),
                    ..When::default()
                },
                "r1",
            ),
            rule(
                "b",
                When {
                    key: Some("al*".into()),
                    ..When::default()
                },
                "r2",
            ),
            rule(
                "c",
                When {
                    key: Some("a*".into()),
                    ..When::default()
                },
                "r3",
            ),
        ];
        let s = Selectors::new(&specs);
        let h = HashMap::new();
        assert_eq!(
            s.choose(&facts(&h), |_| true),
            Some(Chosen {
                rule: "b",
                route: "r2"
            })
        );
        let mut nobody = facts(&h);
        nobody.key = Some("zed");
        assert_eq!(s.choose(&nobody, |_| true), None);
        nobody.key = None;
        assert_eq!(
            s.choose(&nobody, |_| true),
            None,
            "a key condition needs a key"
        );
    }

    #[test]
    fn all_conditions_of_a_rule_must_hold() {
        let when = When {
            key: Some("alice".into()),
            subagent: Some(true),
            header: BTreeMap::from([("X-Client".into(), "open*".into())]),
            ..When::default()
        };
        let s = Selectors::new(&[rule("r", when, "cheap")]);
        let h = headers(&[("x-client", "openclaw")]);
        let mut f = facts(&h);
        assert_eq!(s.choose(&f, |_| true), None, "not a subagent");
        f.subagent = true;
        assert!(s.choose(&f, |_| true).is_some());
        let other = headers(&[("x-client", "curl")]);
        let mut f = facts(&other);
        f.subagent = true;
        assert_eq!(s.choose(&f, |_| true), None, "header differs");
        let none = HashMap::new();
        let mut f = facts(&none);
        f.subagent = true;
        assert_eq!(s.choose(&f, |_| true), None, "header absent");
    }

    #[test]
    fn profile_tag_agent_task_model_and_stream_conditions() {
        let when = |w: When| Selectors::new(&[rule("r", w, "x")]);
        let h = headers(&[
            ("x-humpyard-profile", "cheap"),
            ("x-humpyard-tag-team", "infra"),
        ]);
        let f = facts(&h);
        let hit = |w: When| when(w).choose(&f, |_| true).is_some();
        assert!(hit(When {
            profile: Some("cheap".into()),
            ..When::default()
        }));
        assert!(!hit(When {
            profile: Some("fast".into()),
            ..When::default()
        }));
        assert!(hit(When {
            tag: BTreeMap::from([("team".into(), "inf*".into())]),
            ..When::default()
        }));
        assert!(!hit(When {
            tag: BTreeMap::from([("team".into(), "sec".into())]),
            ..When::default()
        }));
        assert!(hit(When {
            agent: Some("plan*".into()),
            ..When::default()
        }));
        assert!(hit(When {
            task: Some("t-1".into()),
            ..When::default()
        }));
        assert!(hit(When {
            model: Some("agent".into()),
            ..When::default()
        }));
        assert!(!hit(When {
            model: Some("other".into()),
            ..When::default()
        }));
        assert!(hit(When {
            stream: Some(true),
            ..When::default()
        }));
        assert!(!hit(When {
            stream: Some(false),
            ..When::default()
        }));
        assert!(hit(When::default()), "no conditions: matches everything");
    }

    #[test]
    fn a_matching_rule_for_a_forbidden_route_is_skipped_not_an_error() {
        let specs = [
            rule(
                "expensive",
                When {
                    profile: Some("deep".into()),
                    ..When::default()
                },
                "paid-only",
            ),
            rule("fallback", When::default(), "cheap"),
        ];
        let s = Selectors::new(&specs);
        let h = headers(&[("x-humpyard-profile", "deep")]);
        let f = facts(&h);
        assert_eq!(s.choose(&f, |_| true).unwrap().route, "paid-only");
        assert_eq!(
            s.choose(&f, |route| route != "paid-only"),
            Some(Chosen {
                rule: "fallback",
                route: "cheap"
            }),
            "the restricted key falls through to the next applicable rule"
        );
        assert_eq!(s.choose(&f, |_| false), None);
    }

    #[test]
    fn the_trace_says_why_each_rule_did_or_did_not_apply() {
        let specs = [
            rule(
                "ci",
                When {
                    key: Some("ci-*".into()),
                    subagent: Some(true),
                    ..When::default()
                },
                "paid",
            ),
            rule(
                "deep",
                When {
                    profile: Some("deep".into()),
                    ..When::default()
                },
                "premium",
            ),
            rule("rest", When::default(), "plain"),
        ];
        let s = Selectors::new(&specs);
        let h = headers(&[("x-humpyard-profile", "deep")]);
        let f = facts(&h);
        let trace = s.trace(&f, |route| route != "premium");
        assert_eq!(trace.len(), 3);
        assert_eq!(
            trace[0].mismatches,
            [
                "key: wanted `ci-*`, got `alice`",
                "subagent: wanted true, got false"
            ]
        );
        assert_eq!(trace[0].permitted, None);
        assert_eq!(trace[1].mismatches, Vec::<String>::new());
        assert_eq!(
            trace[1].permitted,
            Some(false),
            "matched, but the key may not use it"
        );
        assert_eq!(trace[2].permitted, Some(true));
        let none = HashMap::new();
        let absent = s.trace(&facts(&none), |_| true);
        assert_eq!(
            absent[1].mismatches,
            ["profile: wanted `deep`, but the request has none"]
        );
    }

    #[test]
    fn unnamed_rules_are_labelled_by_position() {
        let s = Selectors::new(&[
            SelectorSpec {
                name: None,
                when: When {
                    key: Some("nobody".into()),
                    ..When::default()
                },
                route: "a".into(),
            },
            SelectorSpec {
                name: None,
                when: When::default(),
                route: "b".into(),
            },
        ]);
        let h = HashMap::new();
        assert_eq!(s.choose(&facts(&h), |_| true).unwrap().rule, "select[1]");
    }

    mod props {
        use proptest::prelude::*;

        use super::super::glob;

        proptest! {
            #[test]
            fn a_pattern_without_stars_matches_only_itself(a in "[a-z0-9-]{0,12}", b in "[a-z0-9-]{0,12}") {
                prop_assert_eq!(glob(&a, &b), a == b);
            }

            #[test]
            fn a_lone_star_matches_everything(text in ".{0,30}") {
                prop_assert!(glob("*", &text));
            }

            #[test]
            fn prefix_star_suffix_matches_exactly_the_wrapped_texts(
                p in "[a-z]{0,5}", mid in "[a-z0-9]{0,8}", s in "[a-z]{0,5}"
            ) {
                let pattern = format!("{p}*{s}");
                let text = format!("{p}{mid}{s}");
                prop_assert!(glob(&pattern, &text));
            }

            #[test]
            fn glob_never_panics(pattern in ".{0,20}", text in ".{0,20}") {
                let _ = glob(&pattern, &text);
            }
        }
    }
}
