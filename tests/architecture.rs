//! Architecture rules, enforced on the source text. See docs/invariants.md.
//!
//! Core modules know nothing of the Switchyard engine or the HTTP edge; only the engine and edge
//! modules may use Switchyard crates; only the ledger may use sqlx; only `main` prints.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // integration tests fail loudly on purpose

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

/// Crates that wrap the (pre-1.0) Switchyard engine.
const SWITCHYARD: [&str; 5] = [
    "switchyard_libsy",
    "switchyard_protocol",
    "switchyard_translation",
    "switchyard_llm_client",
    "switchyard_runner",
];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Layer {
    /// Pure gateway concepts: no Switchyard, no HTTP edge, no engine modules.
    Core,
    /// May use Switchyard, but not the HTTP edge.
    Engine,
    /// Wires everything together.
    Edge,
}

struct Module {
    layer: Layer,
    /// Other modules of this crate it may reference (`Edge` modules may reference any).
    deps: &'static [&'static str],
    sqlx: bool,
    may_print: bool,
}

const fn module(layer: Layer, deps: &'static [&'static str]) -> Module {
    Module {
        layer,
        deps,
        sqlx: false,
        may_print: false,
    }
}

fn rules() -> BTreeMap<&'static str, Module> {
    use Layer::{Core, Edge, Engine};
    BTreeMap::from([
        ("clock", module(Core, &["num"])),
        ("num", module(Core, &[])),
        ("config", module(Core, &[])),
        ("policy", module(Core, &[])),
        ("auth", module(Core, &["config"])),
        (
            "ledger",
            Module {
                sqlx: true,
                ..module(Core, &["num"])
            },
        ),
        (
            "budget",
            module(Core, &["clock", "config", "ledger", "num", "policy"]),
        ),
        ("error", module(Engine, &[])),
        ("pricing", module(Engine, &["config", "num"])),
        ("pool", module(Engine, &["config"])),
        ("routing", module(Engine, &["config"])),
        (
            "metering",
            module(
                Engine,
                &["budget", "clock", "config", "ledger", "pool", "pricing"],
            ),
        ),
        ("server", module(Edge, &[])),
        ("lib", module(Edge, &[])),
        (
            "main",
            Module {
                may_print: true,
                ..module(Edge, &[])
            },
        ),
    ])
}

/// Drops string literals, comments and the trailing `#[cfg(test)]` module so only production
/// code is scanned.
fn production_code(source: &str) -> String {
    let before_tests = source.split("#[cfg(test)]").next().unwrap_or(source);
    let mut out = String::with_capacity(before_tests.len());
    let chars: Vec<char> = before_tests.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        match (chars[i], chars.get(i + 1)) {
            ('/', Some('/')) => {
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
            }
            ('/', Some('*')) => {
                i += 2;
                while i + 1 < chars.len() && !(chars[i] == '*' && chars[i + 1] == '/') {
                    i += 1;
                }
                i += 2;
            }
            ('"', _) => {
                i += 1;
                while i < chars.len() && chars[i] != '"' {
                    i += if chars[i] == '\\' { 2 } else { 1 };
                }
                i += 1;
            }
            (c, _) => {
                out.push(c);
                i += 1;
            }
        }
    }
    out
}

fn is_ident(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Every module named after `crate::`, including inside `use crate::{a, b::c}` groups.
fn crate_refs(code: &str) -> Vec<String> {
    let mut refs = Vec::new();
    let mut rest = code;
    while let Some(at) = rest.find("crate::") {
        let before_ok = rest[..at].chars().next_back().is_none_or(|c| !is_ident(c));
        rest = &rest[at + "crate::".len()..];
        if !before_ok {
            continue;
        }
        if let Some(group) = rest.strip_prefix('{') {
            let (mut depth, mut item, mut items) = (1, String::new(), Vec::new());
            for c in group.chars() {
                match c {
                    '{' => depth += 1,
                    '}' => {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    ',' if depth == 1 => items.push(std::mem::take(&mut item)),
                    _ => {}
                }
                if depth == 1 && c != ',' && c != '{' {
                    item.push(c);
                }
            }
            items.push(item);
            refs.extend(items.iter().map(|i| {
                i.trim()
                    .chars()
                    .take_while(|c| is_ident(*c))
                    .collect::<String>()
            }));
        } else {
            refs.push(rest.chars().take_while(|c| is_ident(*c)).collect());
        }
    }
    refs.retain(|r| !r.is_empty());
    refs
}

fn mentions(code: &str, word: &str) -> bool {
    code.match_indices(word).any(|(at, _)| {
        let before = code[..at].chars().next_back().is_none_or(|c| !is_ident(c));
        let after = code[at + word.len()..]
            .chars()
            .next()
            .is_none_or(|c| !is_ident(c));
        before && after
    })
}

fn violations(name: &str, source: &str) -> Vec<String> {
    let Some(rule) = rules().remove(name) else {
        return vec![format!(
            "`src/{name}.rs` is not in the architecture rules: add it to `rules()` in tests/architecture.rs with a layer"
        )];
    };
    let code = production_code(source);
    let mut found = Vec::new();
    if rule.layer == Layer::Core {
        for crate_name in SWITCHYARD {
            if mentions(&code, crate_name) {
                found.push(format!("core module `{name}` must not use `{crate_name}` (Switchyard stays in engine/edge modules)"));
            }
        }
    }
    if rule.layer != Layer::Edge {
        for referenced in crate_refs(&code) {
            let allowed = rule.deps.contains(&referenced.as_str()) || referenced == name;
            if !allowed {
                found.push(format!(
                    "`{name}` must not depend on `crate::{referenced}` ({:?} layer; allowed: {:?})",
                    rule.layer, rule.deps
                ));
            }
        }
    }
    if !rule.sqlx && mentions(&code, "sqlx") {
        found.push(format!(
            "`{name}` must not use `sqlx` (only the ledger touches SQL)"
        ));
    }
    if !rule.may_print {
        for mac in ["println!", "print!", "eprintln!", "eprint!", "dbg!"] {
            if code.contains(mac) {
                found.push(format!(
                    "`{name}` must not use `{mac}` (log with tracing; only `main` prints)"
                ));
            }
        }
    }
    found
}

#[test]
fn the_source_tree_obeys_the_architecture_rules() {
    let mut all = Vec::new();
    for entry in fs::read_dir(Path::new(env!("CARGO_MANIFEST_DIR")).join("src")).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|e| e == "rs") {
            let name = path.file_stem().unwrap().to_string_lossy().to_string();
            all.extend(violations(&name, &fs::read_to_string(&path).unwrap()));
        }
    }
    assert!(
        all.is_empty(),
        "architecture violations (see docs/invariants.md):\n  - {}",
        all.join("\n  - ")
    );
}

// ---- the scanner itself: each form of violation we care about must be caught ----

fn flagged(module: &str, injected: &str) -> bool {
    !violations(module, injected).is_empty()
}

#[test]
fn catches_every_form_of_a_forbidden_internal_dependency() {
    for (module, code) in [
        ("config", "use crate::pool::TargetClient;"),
        (
            "config",
            "use crate::pool::{PROVIDER_HEADER, TargetClient};",
        ),
        ("config", "use crate::{pool, routing};"),
        (
            "config",
            "use crate::{pool::PROVIDER_HEADER, routing::Routes};",
        ),
        ("config", "use crate::pool;"),
        ("config", "pub use crate::pool::PROVIDER_HEADER;"),
        ("config", "fn f() { let _ = crate::pool::PROVIDER_HEADER; }"),
        (
            "config",
            "use crate::{ledger::{Entry, Kind}, pool::Served};",
        ),
        ("ledger", "use crate::server::AppState;"),
        ("clock", "use crate::routing::Routes;"),
        ("budget", "use crate::metering::Accounting;"),
        ("metering", "use crate::server::Options;"),
        ("pool", "use crate::auth::KeyRecord;"),
    ] {
        assert!(flagged(module, code), "{module}: should flag `{code}`");
    }
}

#[test]
fn catches_switchyard_in_core_modules_in_every_form() {
    for code in [
        "use switchyard_libsy::Passthrough;",
        "use switchyard_protocol::{Usage, Metadata as M};",
        "fn f() { let _ = switchyard_protocol::Usage::default(); }",
        "use switchyard_translation::decode_request;",
    ] {
        assert!(flagged("auth", code), "auth: should flag `{code}`");
        assert!(flagged("budget", code), "budget: should flag `{code}`");
    }
}

#[test]
fn catches_sqlx_outside_the_ledger_and_prints_outside_main() {
    for module in ["server", "budget", "auth", "metering", "pool"] {
        assert!(flagged(module, "use sqlx::SqlitePool;"), "{module}: sqlx");
        assert!(
            flagged(module, "fn f() { let _ = sqlx::Error::RowNotFound; }"),
            "{module}: inline sqlx"
        );
    }
    assert!(!flagged("ledger", "use sqlx::SqlitePool;"));
    assert!(flagged("server", "fn f() { println!(\"x\"); }"));
    assert!(flagged("pool", "fn f() { dbg!(1); }"));
    assert!(!flagged("main", "fn f() { println!(\"x\"); }"));
}

#[test]
fn allows_what_the_rules_allow_and_ignores_tests_comments_and_strings() {
    assert!(!flagged(
        "metering",
        "use crate::pool::{Served, TargetClient};\nuse switchyard_protocol::Usage;"
    ));
    assert!(!flagged(
        "budget",
        "use crate::{clock::Clock, config::Config, ledger::Ledger, policy::BudgetState};"
    ));
    assert!(!flagged(
        "server",
        "use crate::pool; use crate::{routing, metering}; use switchyard_libsy::Passthrough;"
    ));
    assert!(!flagged(
        "config",
        "// uses crate::pool and switchyard_libsy and sqlx\nlet s = \"crate::pool sqlx println!\";"
    ));
    assert!(!flagged(
        "config",
        "fn real() {}\n#[cfg(test)]\nmod tests { use crate::pool::X; use sqlx::Row; }"
    ));
    assert!(!flagged(
        "config",
        "pub(crate) fn f() {} // crate:: in comment"
    ));
}

#[test]
fn an_unknown_module_fails_closed() {
    let found = violations("brand_new", "fn f() {}");
    assert!(found[0].contains("not in the architecture rules"));
}
