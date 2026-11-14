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
        ("estimate", module(Core, &[])),
        // Observed time per target and the ordering a `fastest` route derives from it (ADR 0020).
        ("latency", module(Core, &["clock", "num"])),
        // Reads a retry time out of an upstream error body; pure text parsing (ADR 0018, amended).
        ("retry_after", module(Core, &[])),
        // Per-key request-rate and concurrency limits: exact sliding window, permit released on drop (`add-key-rate-limits`).
        ("limiter", module(Core, &["clock", "config"])),
        // Makes a provider's error text safe to keep in a decision: one bounded line, key-shaped values redacted (ADR 0028).
        ("scrub", module(Core, &[])),
        // Makes a tool call's arguments a JSON object, completing a cut-off one; pure text (ADR 0029).
        ("repair", module(Core, &[])),
        // Applies it to a request: the decoded tool calls and the preserved same-format bodies (ADR 0029).
        ("repair_request", module(Engine, &["repair"])),
        // Judges an answer for four facts (empty, cut by the length limit, a malformed tool call, a stream cut early); pure (ADR 0030).
        ("signals", module(Core, &["repair"])),
        ("health", module(Core, &["clock", "config"])),
        ("select", module(Core, &["config"])),
        // Expiring runtime overrides: the store only; nothing here changes a plan yet (`add-runtime-overrides`).
        ("overrides", module(Core, &["clock", "config", "ledger"])),
        ("config", module(Core, &[])),
        ("policy", module(Core, &[])),
        ("auth", module(Core, &["config"])),
        // Admin authentication: who a presented secret is on `/admin/v1`, separate from client keys (ADR 0027).
        ("admin", module(Core, &["auth", "config"])),
        (
            "ledger",
            Module {
                sqlx: true,
                ..module(Core, &["num", "scrub", "signals"])
            },
        ),
        (
            "budget",
            module(Core, &["clock", "config", "ledger", "num", "policy"]),
        ),
        ("error", module(Engine, &["config"])),
        ("pricing", module(Engine, &["config", "num"])),
        // Reads price sources for the `prices` command only; see the serving-path test below.
        ("prices", module(Core, &["config"])),
        // Reads the sources for the `models` commands only; see the serving-path tests (ADR 0022).
        ("discover", module(Core, &["config", "num", "prices"])),
        // Asks each provider for its own model list for `models check --retired` only; command-only (ADR 0022).
        ("retired", module(Core, &["config", "discover"])),
        // Reads a ModelRelay setup for the `migrate-modelrelay` command only; see the serving-path test.
        ("migrate", module(Core, &["config", "num"])),
        // Polls provider usage while serving; advisory and fail-open (ADR 0015).
        ("quota", module(Core, &["clock", "config"])),
        // Operational metrics: one registry, bounded labels (ADR 0016).
        ("metrics", module(Core, &["config", "ledger"])),
        (
            "pool",
            module(
                Engine,
                &[
                    "clock",
                    "config",
                    "error",
                    "health",
                    "overrides",
                    "retry_after",
                    "scrub",
                ],
            ),
        ),
        ("routing", module(Engine, &["config", "latency", "num"])),
        (
            "metering",
            module(
                Engine,
                &[
                    "budget", "clock", "config", "error", "estimate", "latency", "ledger",
                    "metrics", "pool", "pricing", "signals",
                ],
            ),
        ),
        ("server", module(Edge, &[])),
        // The `init` command wires config, model fetching and an in-process gateway; `serve` never uses it.
        ("init", module(Edge, &[])),
        // The read-only `ctl` client for a running gateway; `serve` never uses it.
        ("ctl", module(Edge, &[])),
        // The admin GUI's server and its embedded files; `serve` never uses either.
        ("ui", module(Edge, &[])),
        ("ui_assets", module(Edge, &[])),
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

/// The module a source file belongs to: `src/config/schema.rs` and `src/config.rs` are both
/// `config`. Nested files are scanned with their top-level module's rules.
fn module_of(relative: &Path) -> String {
    let first = relative
        .components()
        .next()
        .unwrap()
        .as_os_str()
        .to_string_lossy()
        .to_string();
    first.strip_suffix(".rs").unwrap_or(&first).to_string()
}

/// Every `.rs` file under `dir`, recursively.
fn rust_files(dir: &Path) -> Vec<std::path::PathBuf> {
    let mut files = Vec::new();
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            files.extend(rust_files(&path));
        } else if path.extension().is_some_and(|e| e == "rs") {
            files.push(path);
        }
    }
    files
}

#[test]
fn the_source_tree_obeys_the_architecture_rules() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut all = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for path in rust_files(&src) {
        let module = module_of(path.strip_prefix(&src).unwrap());
        all.extend(violations(&module, &fs::read_to_string(&path).unwrap()));
        seen.insert(module);
    }
    for declared in rules().keys() {
        if !seen.contains(*declared) {
            all.push(format!(
                "rules() lists `{declared}` but there is no such module in src/ (renamed or removed? update rules())"
            ));
        }
    }
    assert!(
        all.is_empty(),
        "architecture violations (see docs/invariants.md):\n  - {}",
        all.join("\n  - ")
    );
}

/// ADR 0014: prices reach the serving process through the config only. Only the `prices` command
/// (`main`) may use the module that reads price sources, so `serve` can never fetch prices.
#[test]
fn only_the_prices_command_uses_the_price_source_module() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut offenders = Vec::new();
    for path in rust_files(&src) {
        let module = module_of(path.strip_prefix(&src).unwrap());
        if matches!(module.as_str(), "prices" | "discover" | "main" | "lib") {
            continue;
        }
        let code = production_code(&fs::read_to_string(&path).unwrap());
        if code.contains("prices::") || code.contains("humpyard::prices") {
            offenders.push(path.display().to_string());
        }
    }
    assert!(
        offenders.is_empty(),
        "these files use the price-source module, which only `humpyard prices` may (ADR 0014): {offenders:?}"
    );
}

/// ADR 0022: only the `models` commands (`main`) may use the module that compares configured endpoints with
/// each provider's own list, so `serve` can never fetch a model list.
#[test]
fn only_the_models_command_uses_the_retired_module() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut offenders = Vec::new();
    for path in rust_files(&src) {
        let module = module_of(path.strip_prefix(&src).unwrap());
        if matches!(module.as_str(), "retired" | "main" | "lib") {
            continue;
        }
        let code = production_code(&fs::read_to_string(&path).unwrap());
        if code.contains("retired::") || code.contains("humpyard::retired") {
            offenders.push(path.display().to_string());
        }
    }
    assert!(
        offenders.is_empty(),
        "these files use the retired-model check, which only `humpyard models` may (ADR 0022): {offenders:?}"
    );
}

/// Only the `init` command (`main`) may use the guided-config module, so `serve` can never reach it.
#[test]
fn only_the_init_command_uses_the_init_module() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut offenders = Vec::new();
    for path in rust_files(&src) {
        let module = module_of(path.strip_prefix(&src).unwrap());
        if matches!(module.as_str(), "init" | "main" | "lib") {
            continue;
        }
        let code = production_code(&fs::read_to_string(&path).unwrap());
        if code.contains("init::") || code.contains("humpyard::init") {
            offenders.push(path.display().to_string());
        }
    }
    assert!(
        offenders.is_empty(),
        "these files use the init module, which only `humpyard init` may: {offenders:?}"
    );
}

/// Only the pool (which skips drained endpoints) and the server (which plans) may read runtime overrides, so
/// nothing else can come to change what a request is allowed to reach (`add-runtime-overrides`, ADR 0025).
#[test]
fn only_the_pool_and_the_server_read_overrides() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut offenders = Vec::new();
    for path in rust_files(&src) {
        let module = module_of(path.strip_prefix(&src).unwrap());
        if matches!(module.as_str(), "overrides" | "pool" | "server" | "lib") {
            continue;
        }
        let code = production_code(&fs::read_to_string(&path).unwrap());
        if code.contains("crate::overrides") || code.contains("humpyard::overrides") {
            offenders.push(path.display().to_string());
        }
    }
    assert!(
        offenders.is_empty(),
        "these files read runtime overrides, which only the pool and the server may: {offenders:?}"
    );
}

/// Admin credential revocations live in the overrides table but are not overrides: only the override store, the
/// admin API and the metrics scrape may touch them, so nothing that plans a request, lists overrides for a client
/// or serves any other endpoint can come to see, create or end one (`add-admin-api`, ADR 0027).
#[test]
fn only_the_override_store_the_admin_api_and_the_scrape_touch_revocations() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let allowed = ["overrides.rs", "server/admin.rs", "server/metrics.rs"];
    let words = [
        "revoke_admin_credential",
        "revoke_principal",
        "end_revocations",
        "is_revoked",
        ".revocations(",
        "Revocation",
    ];
    let mut offenders = Vec::new();
    for path in rust_files(&src) {
        let relative = path
            .strip_prefix(&src)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        if allowed.contains(&relative.as_str()) {
            continue;
        }
        let code = production_code(&fs::read_to_string(&path).unwrap());
        if let Some(word) = words.iter().find(|word| code.contains(**word)) {
            offenders.push(format!("{relative} uses `{word}`"));
        }
    }
    assert!(
        offenders.is_empty(),
        "admin credential revocations may be touched only by the override store, `server/admin.rs` and \
         `server/metrics.rs`: {offenders:?}"
    );
}

/// Only the `ctl` command (`main`) may use the remote-client module, so `serve` can never reach it.
#[test]
fn only_the_ctl_command_uses_the_ctl_module() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut offenders = Vec::new();
    for path in rust_files(&src) {
        let module = module_of(path.strip_prefix(&src).unwrap());
        if matches!(module.as_str(), "ctl" | "main" | "lib") {
            continue;
        }
        let code = production_code(&fs::read_to_string(&path).unwrap());
        if code.contains("ctl::") || code.contains("humpyard::ctl") {
            offenders.push(path.display().to_string());
        }
    }
    assert!(
        offenders.is_empty(),
        "these files use the ctl module, which only `humpyard ctl` may: {offenders:?}"
    );
}

/// Only the `ui` command (`main`) may use the module that serves the admin GUI, and only that module may use the
/// embedded files, so the serving path can never carry a web page or a static-file path (ADR 0032).
#[test]
fn only_the_ui_command_uses_the_gui_modules() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut offenders = Vec::new();
    for path in rust_files(&src) {
        let module = module_of(path.strip_prefix(&src).unwrap());
        let code = production_code(&fs::read_to_string(&path).unwrap());
        let uses = |name: &str| {
            code.contains(&format!("crate::{name}"))
                || code.contains(&format!("humpyard::{name}"))
                || code.contains(&format!("super::{name}"))
        };
        if !matches!(module.as_str(), "ui" | "ui_assets" | "main" | "lib")
            && (uses("ui") || uses("ui_assets"))
        {
            offenders.push(path.display().to_string());
        }
        // The embedded files are reached through `ui` only.
        if module != "ui" && module != "ui_assets" && module != "lib" && uses("ui_assets") {
            offenders.push(format!("{} (ui_assets)", path.display()));
        }
    }
    assert!(
        offenders.is_empty(),
        "these files use the admin GUI modules, which only `humpyard ui` may: {offenders:?}"
    );
}

/// Only the `migrate-modelrelay` command (`main`) may use the module that reads a `ModelRelay` setup, so
/// `serve` can never fetch one.
#[test]
fn only_the_migrate_command_uses_the_migration_module() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut offenders = Vec::new();
    for path in rust_files(&src) {
        let module = module_of(path.strip_prefix(&src).unwrap());
        if matches!(module.as_str(), "migrate" | "main" | "lib") {
            continue;
        }
        let code = production_code(&fs::read_to_string(&path).unwrap());
        if code.contains("migrate::") || code.contains("humpyard::migrate") {
            offenders.push(path.display().to_string());
        }
    }
    assert!(
        offenders.is_empty(),
        "these files use the migration module, which only `humpyard migrate-modelrelay` may: {offenders:?}"
    );
}

/// ADR 0022: only the `models` commands (`main`) may use the module that reads the sources, so `serve` can
/// never fetch a model list; it only reads the file those commands write (inside `config`).
#[test]
fn only_the_models_commands_use_the_discovery_module() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut offenders = Vec::new();
    for path in rust_files(&src) {
        let module = module_of(path.strip_prefix(&src).unwrap());
        // `init` fetches one provider's model list for the guided first config; `retired` fetches each
        // provider's list for `models check --retired`; both are command-only.
        if matches!(
            module.as_str(),
            "discover" | "init" | "retired" | "main" | "lib"
        ) {
            continue;
        }
        let code = production_code(&fs::read_to_string(&path).unwrap());
        if code.contains("discover::") || code.contains("humpyard::discover") {
            offenders.push(path.display().to_string());
        }
    }
    assert!(
        offenders.is_empty(),
        "these files use the discovery module, which only `humpyard models` may: {offenders:?}"
    );
}

#[test]
fn files_map_to_their_top_level_module() {
    assert_eq!(module_of(Path::new("config.rs")), "config");
    assert_eq!(module_of(Path::new("config/mod.rs")), "config");
    assert_eq!(module_of(Path::new("config/schema.rs")), "config");
    assert_eq!(module_of(Path::new("deep/er/file.rs")), "deep");
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

// ---- Heavy ledger reads (ADR 0036, invariant 24) -----------------------------------------------------------------
//
// Statistics, session views and the route explain's recent numbers fold up to 50,000 ledger rows. These rules keep
// that work off the serving runtime, off the writer's connections and behind the gate. Each is a function of source
// text, so a sample that commits the violation can be checked to be caught, as well as the tree being checked clean.

/// The ledger's public reads: each must read through the read pool and never the writer's.
const LEDGER_READS: [&str; 8] = [
    "quota_history",
    "decisions",
    "answer_signals",
    "session_scan",
    "stats",
    "series",
    "spend_since",
    "entries_since",
];

/// The body of the method `name` in `code`: from its signature to the next method, attribute or end of the impl.
fn method_body<'a>(code: &'a str, name: &str) -> Option<&'a str> {
    let start = code.find(&format!("pub async fn {name}("))?;
    let rest = &code[start + 1..];
    let end = [
        "\n    pub ",
        "\n    #[",
        "\n    fn ",
        "\n    async fn ",
        "\n}",
    ]
    .iter()
    .filter_map(|marker| rest.find(marker))
    .min()
    .unwrap_or(rest.len());
    Some(&rest[..end])
}

/// `src/ledger.rs`: every public read uses the read pool, none the writer's.
fn read_pool_violations(ledger: &str) -> Vec<String> {
    let mut out = Vec::new();
    for name in LEDGER_READS {
        match method_body(ledger, name) {
            None => out.push(format!(
                "the ledger has no `{name}` read (or this rule is stale)"
            )),
            Some(body) => {
                if body.contains("self.pool") {
                    out.push(format!("`{name}` reads through the writer's pool"));
                }
                if !body.contains("read_pool") {
                    out.push(format!("`{name}` does not read through the read pool"));
                }
            }
        }
    }
    out
}

/// `src/ledger.rs`: the helper that runs a fold hands it to the blocking pool.
fn dispatch_violations(ledger: &str) -> Vec<String> {
    if ledger.contains("spawn_blocking(") {
        Vec::new()
    } else {
        vec!["no row fold is dispatched with spawn_blocking".to_string()]
    }
}

/// `src/ledger/stats.rs` and `sessions.rs`: the fold is private, and reached only through `off_runtime`.
fn fold_violations(module: &str, code: &str) -> Vec<String> {
    let mut out = Vec::new();
    for (at, _) in code.match_indices("fn fold(") {
        let line_start = code[..at].rfind('\n').map_or(0, |i| i + 1);
        if code[line_start..at].contains("pub") {
            out.push(format!("{module}: the row fold is pub, so code outside the module can run it on the runtime"));
        }
    }
    if !code.contains("fn fold(") {
        out.push(format!("{module}: has no `fold` (or this rule is stale)"));
    }
    if !code.contains("off_runtime(move || fold(") {
        out.push(format!("{module}: the row fold is not run through `off_runtime`, so it can hold a runtime worker"));
    }
    out
}

/// A `server` module that calls a heavy ledger read also names the gate.
fn gate_violations(module: &str, code: &str) -> Vec<String> {
    let heavy =
        code.contains(".stats(") || code.contains(".series(") || code.contains(".session_scan(");
    if heavy && !code.contains(".heavy") {
        vec![format!(
            "{module}: calls a heavy ledger read without going through `state.heavy`, the gate and shared result"
        )]
    } else {
        Vec::new()
    }
}

#[test]
fn heavy_ledger_reads_stay_off_the_runtime_and_behind_the_gate() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let code = |relative: &str| production_code(&fs::read_to_string(src.join(relative)).unwrap());
    let ledger = code("ledger.rs");
    let mut found = read_pool_violations(&ledger);
    found.extend(dispatch_violations(&ledger));
    for module in ["stats", "series", "sessions"] {
        found.extend(fold_violations(
            module,
            &code(&format!("ledger/{module}.rs")),
        ));
    }
    for path in rust_files(&src.join("server")) {
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        let source = production_code(&fs::read_to_string(&path).unwrap());
        found.extend(gate_violations(&format!("server/{name}"), &source));
    }
    // The gate module itself is part of the rule: it must exist, or the rule above passes by checking nothing.
    assert!(
        src.join("server/heavy.rs").exists(),
        "server/heavy.rs, the gate, is missing"
    );
    assert!(found.is_empty(), "{found:#?}");
}

#[test]
fn a_ledger_read_through_the_writers_pool_is_caught() {
    let sample = "impl Ledger {\n    pub async fn stats(&self, q: &Q) -> R {\n        stats::compute(&self.pool, q).await\n    }\n}\n";
    let found = read_pool_violations(sample);
    assert!(
        found
            .iter()
            .any(|v| v.contains("`stats` reads through the writer's pool")),
        "{found:?}"
    );
    assert!(
        found
            .iter()
            .any(|v| v.contains("`stats` does not read through the read pool")),
        "{found:?}"
    );
    // A read that is missing altogether is a stale rule, not a pass.
    assert!(
        found.iter().any(|v| v.contains("no `decisions` read")),
        "{found:?}"
    );
}

#[test]
fn a_fold_that_is_public_or_run_on_the_runtime_is_caught() {
    let public = "pub fn fold(rows: &[Row]) {}\nasync fn compute() { off_runtime(move || fold(&rows)).await }\n";
    let found = fold_violations("stats", public);
    assert!(found.iter().any(|v| v.contains("is pub")), "{found:?}");

    let inline = "fn fold(rows: &[Row]) {}\nasync fn compute() { fold(&rows) }\n";
    let found = fold_violations("stats", inline);
    assert!(
        found
            .iter()
            .any(|v| v.contains("not run through `off_runtime`")),
        "{found:?}"
    );

    let good =
        "fn fold(rows: &[Row]) {}\nasync fn compute() { off_runtime(move || fold(&rows)).await }\n";
    assert_eq!(fold_violations("stats", good), Vec::<String>::new());
    assert_eq!(dispatch_violations("async fn off_runtime() {}").len(), 1);
    assert_eq!(
        dispatch_violations("tokio::task::spawn_blocking(work)"),
        Vec::<String>::new()
    );
}

#[test]
fn a_server_module_that_reads_the_ledger_without_the_gate_is_caught() {
    let ungated = "async fn view(state: &AppState) { ledger.stats(&query).await }";
    let found = gate_violations("server/new.rs", ungated);
    assert!(
        found
            .iter()
            .any(|v| v.contains("without going through `state.heavy`")),
        "{found:?}"
    );
    let sessions = "async fn view() { ledger.session_scan(&q).await }";
    assert_eq!(gate_violations("server/new.rs", sessions).len(), 1);

    let gated = "async fn view(state: &AppState) { state\n        .heavy\n        .run(a, b, c, d, async { ledger.stats(&q).await })\n        .await }";
    assert_eq!(
        gate_violations("server/stats.rs", gated),
        Vec::<String>::new()
    );
    assert_eq!(
        gate_violations("server/other.rs", "async fn view() {}"),
        Vec::<String>::new()
    );
}

// ---- the fitness register (`fitness/register.toml`, docs/fitness-functions.md) ----

#[derive(serde::Deserialize)]
struct Register {
    modules: BTreeMap<String, Class>,
    checks: Vec<Check>,
}

#[derive(serde::Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
enum Class {
    Request,
    Shared,
    Off,
}

#[derive(serde::Deserialize)]
struct Check {
    id: String,
    cadence: Vec<String>,
    r#where: String,
    blocks: bool,
    triggers: Triggers,
}

#[derive(serde::Deserialize)]
struct Triggers {
    classes: Vec<Class>,
    paths: Vec<String>,
}

fn register_violations(
    register: &Register,
    modules: &std::collections::BTreeSet<String>,
) -> Vec<String> {
    let mut out = Vec::new();
    for module in modules {
        if !register.modules.contains_key(module) {
            out.push(format!(
                "module `{module}` has no performance class: add it to [modules] in fitness/register.toml (request, shared or off)"
            ));
        }
    }
    for module in register.modules.keys() {
        if !modules.contains(module) {
            out.push(format!(
                "fitness/register.toml classes `{module}` but there is no such module in src/ (renamed or removed? update [modules])"
            ));
        }
    }
    for check in &register.checks {
        let scheduled = check
            .cadence
            .iter()
            .any(|c| c == "weekly" || c == "release");
        if check.triggers.classes.is_empty() && check.triggers.paths.is_empty() && !scheduled {
            out.push(format!(
                "check `{}` has no trigger and no scheduled cadence, so nothing would ever run it",
                check.id
            ));
        }
        if check.r#where == "reference" && check.cadence.iter().any(|c| c != "release") {
            out.push(format!(
                "check `{}` runs on a reference machine, which only a release can use: its cadence must be [\"release\"]",
                check.id
            ));
        }
    }
    for (module, class) in &register.modules {
        if *class == Class::Off {
            continue;
        }
        let covered = register.checks.iter().any(|c| {
            c.blocks
                && (c.triggers.classes.contains(class)
                    || c.triggers.paths.iter().any(|p| {
                        p == &format!("src/{module}.rs") || p == &format!("src/{module}/*")
                    }))
        });
        if !covered {
            out.push(format!(
                "module `{module}` ({class:?}) is covered by no blocking check: list its class or its files in a check's triggers"
            ));
        }
    }
    out
}

/// The check ids a page lists: the first backticked word of each table row that starts with one.
fn page_checks(page: &str) -> std::collections::BTreeSet<String> {
    page.lines()
        .filter_map(|l| l.strip_prefix("| `"))
        .filter_map(|l| l.split('`').next())
        .filter(|id| id.contains('-'))
        .map(str::to_string)
        .collect()
}

fn page_violations(register: &Register, page: &str) -> Vec<String> {
    let listed = page_checks(page);
    let mut out = Vec::new();
    for check in &register.checks {
        if !listed.contains(&check.id) {
            out.push(format!(
                "check `{}` is in the register but not on docs/fitness-functions.md",
                check.id
            ));
        }
    }
    for id in &listed {
        if !register.checks.iter().any(|c| &c.id == id) {
            out.push(format!(
                "docs/fitness-functions.md lists `{id}` but the register has no such check"
            ));
        }
    }
    out
}

fn load_register() -> Register {
    let text =
        fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("fitness/register.toml"))
            .unwrap();
    toml::from_str(&text).unwrap()
}

fn source_modules() -> std::collections::BTreeSet<String> {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    rust_files(&src)
        .iter()
        .map(|p| module_of(p.strip_prefix(&src).unwrap()))
        .collect()
}

#[test]
fn every_module_is_classed_and_every_request_or_shared_module_is_watched() {
    let problems = register_violations(&load_register(), &source_modules());
    assert!(
        problems.is_empty(),
        "fitness register (docs/fitness-functions.md):\n  - {}",
        problems.join("\n  - ")
    );
}

#[test]
fn the_fitness_page_and_the_register_list_the_same_checks() {
    let page =
        fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/fitness-functions.md"))
            .unwrap();
    let problems = page_violations(&load_register(), &page);
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

fn sample_register(extra: &str) -> Register {
    toml::from_str(&format!(
        r#"[modules]
routing = "request"
ledger = "shared"
ctl = "off"
{extra}
[[checks]]
id = "isolation-test"
cadence = ["change", "weekly"]
where = "ci"
blocks = true
triggers = {{ classes = ["request", "shared"], paths = [] }}
"#
    ))
    .unwrap()
}

fn sample_modules(names: &[&str]) -> std::collections::BTreeSet<String> {
    names.iter().map(|n| (*n).to_string()).collect()
}

#[test]
fn a_consistent_sample_register_has_no_violations() {
    let register = sample_register("");
    assert_eq!(
        register_violations(&register, &sample_modules(&["routing", "ledger", "ctl"])),
        Vec::<String>::new()
    );
}

#[test]
fn an_unclassified_module_is_caught() {
    let found = register_violations(
        &sample_register(""),
        &sample_modules(&["routing", "ledger", "ctl", "newmod"]),
    );
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(
        found[0].contains("`newmod` has no performance class"),
        "{found:?}"
    );
}

#[test]
fn a_stale_entry_is_caught() {
    let found = register_violations(
        &sample_register(""),
        &sample_modules(&["routing", "ledger"]),
    );
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(
        found[0].contains("classes `ctl` but there is no such module"),
        "{found:?}"
    );
}

#[test]
fn a_request_module_no_blocking_check_covers_is_caught() {
    let mut register = sample_register("");
    register.checks[0].triggers.classes = vec![Class::Shared];
    let found = register_violations(&register, &sample_modules(&["routing", "ledger", "ctl"]));
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(
        found[0].contains("`routing` (Request) is covered by no blocking check"),
        "{found:?}"
    );
    // Naming the module's files instead covers it.
    register.checks[0].triggers.paths = vec!["src/routing/*".into()];
    assert_eq!(
        register_violations(&register, &sample_modules(&["routing", "ledger", "ctl"])),
        Vec::<String>::new()
    );
    // A check that does not block covers nothing.
    register.checks[0].blocks = false;
    assert_eq!(
        register_violations(&register, &sample_modules(&["routing", "ledger", "ctl"])).len(),
        2
    );
}

#[test]
fn a_check_nothing_would_run_and_a_per_change_reference_check_are_caught() {
    let mut register = sample_register("");
    register.checks[0].triggers = Triggers {
        classes: vec![],
        paths: vec![],
    };
    register.checks[0].cadence = vec!["change".into()];
    let found = register_violations(&register, &sample_modules(&["routing", "ledger", "ctl"]));
    assert!(
        found
            .iter()
            .any(|f| f.contains("nothing would ever run it")),
        "{found:?}"
    );
    register.checks[0].cadence = vec!["change".into(), "release".into()];
    register.checks[0].r#where = "reference".into();
    let found = register_violations(&register, &sample_modules(&["routing", "ledger", "ctl"]));
    assert!(
        found.iter().any(|f| f.contains("only a release can use")),
        "{found:?}"
    );
}

#[test]
fn a_page_that_disagrees_with_the_register_is_caught() {
    let register = sample_register("");
    assert_eq!(
        page_violations(&register, "| `isolation-test` | x |\n"),
        Vec::<String>::new()
    );
    let missing = page_violations(&register, "| nothing |\n");
    assert!(
        missing[0].contains("`isolation-test` is in the register but not on"),
        "{missing:?}"
    );
    let extra = page_violations(
        &register,
        "| `isolation-test` | x |\n| `made-up-check` | y |\n",
    );
    assert_eq!(extra.len(), 1, "{extra:?}");
    assert!(extra[0].contains("lists `made-up-check`"), "{extra:?}");
}

// ---- Actors are labels (ADR 0037, invariant 25) -------------------------------------------------------------------
//
// An actor and its owner say who a key is for. They are recorded on decisions and shown and counted by the statistics
// and the configuration view; they never decide what a request may do. These rules keep it so: outside the places
// that carry, record or display them, no source text names an actor or an owner, so nothing on the request path can
// authorize, limit, budget or route by one. Like the rules above, each is a function of source text.

/// Files that may name an actor or an owner: the configuration that defines them, the key record that carries them,
/// the ledger that stores and counts them, and the handlers that display or group by them.
const ACTOR_ATTRIBUTION: [&str; 5] = [
    "config/",
    "auth.rs",
    "ledger",
    "server/config_view.rs",
    "server/decisions.rs",
];

/// The statistics handlers name them only to accept `group_by=actor` and `group_by=owner`.
const ACTOR_GROUPING: [&str; 2] = ["server/stats.rs", "server/series.rs"];

/// `server.rs` serves requests, so it may name them only on these four lines (string literals are stripped before the
/// scan, so the key info's `"actor": caller.actor,` reads as `: caller.actor,`): two that write them onto the
/// decision and two that show them in the key's own info.
const ACTOR_SERVER_LINES: [&str; 4] = [
    "guard.decision.actor = caller.as_ref().and_then(|k| k.actor.clone());",
    "guard.decision.owner = caller.as_ref().and_then(|k| k.owner.clone());",
    ": caller.actor,",
    ": caller.owner,",
];

fn names_an_actor(line: &str) -> bool {
    line.split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .any(|word| {
            let lower = word.to_lowercase();
            matches!(lower.as_str(), "actor" | "actors" | "owner" | "owners")
                || word.starts_with("Actor")
                || word.starts_with("Owner")
        })
}

/// The lines of `code` (production code of `relative`, a path under `src/`) that name an actor or an owner where
/// that is not allowed.
fn actor_violations(relative: &str, code: &str) -> Vec<String> {
    let allowed_file = ACTOR_ATTRIBUTION
        .iter()
        .chain(ACTOR_GROUPING.iter())
        .any(|prefix| relative.starts_with(prefix));
    if allowed_file {
        return Vec::new();
    }
    code.lines()
        .filter(|line| names_an_actor(line))
        .filter(|line| relative != "server.rs" || !ACTOR_SERVER_LINES.contains(&line.trim()))
        .map(|line| format!("{relative}: `{}` names an actor or an owner", line.trim()))
        .collect()
}

#[test]
fn no_module_reads_an_actor_or_an_owner_to_decide_anything() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut found = Vec::new();
    for path in rust_files(&src) {
        let relative = path
            .strip_prefix(&src)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        let code = production_code(&fs::read_to_string(&path).unwrap());
        found.extend(actor_violations(&relative, &code));
    }
    assert!(
        found.is_empty(),
        "an actor is a label and grants nothing (ADR 0037); only the attribution code may name one:\n  - {}",
        found.join("\n  - ")
    );
}

#[test]
fn the_files_that_may_name_an_actor_exist_so_the_rule_cannot_pass_by_checking_nothing() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    for relative in ACTOR_ATTRIBUTION.iter().chain(ACTOR_GROUPING.iter()) {
        let found = src.join(relative).exists() || src.join(format!("{relative}.rs")).exists();
        assert!(
            found,
            "src/{relative} is listed as attribution code but is missing (renamed? update the list)"
        );
    }
    // And the real tree does name actors where the rule allows it, so the scan is looking at something.
    let config = production_code(&fs::read_to_string(src.join("config/actors.rs")).unwrap());
    assert!(names_an_actor(&config));
}

#[test]
fn a_policy_that_reads_an_actor_is_caught() {
    let sample =
        "fn allows(key: &KeyRecord) -> bool {\n    key.actor.as_deref() != Some(\"blocked\")\n}\n";
    assert_eq!(actor_violations("policy.rs", sample).len(), 1);
}

#[test]
fn a_limiter_or_budget_that_reads_an_owner_is_caught() {
    for module in [
        "limiter.rs",
        "budget.rs",
        "select.rs",
        "pool.rs",
        "routing.rs",
        "metering.rs",
        "server/overrides.rs",
    ] {
        let sample = "let cap = if key.owner.is_some() { 10 } else { 1 };\n";
        assert_eq!(actor_violations(module, sample).len(), 1, "{module}");
    }
}

#[test]
fn a_request_handler_that_branches_on_an_actor_is_caught_but_recording_one_is_not() {
    // The four lines `server.rs` has today pass, exactly as written.
    let allowed = ACTOR_SERVER_LINES
        .iter()
        .fold(String::new(), |all, line| all + "    " + line + "\n");
    assert_eq!(
        actor_violations("server.rs", &allowed),
        Vec::<String>::new()
    );
    // Anything else naming one on the request path is caught, however close to those lines it sits.
    for hostile in [
        "    let tier = if key.actor.is_some() { 1 } else { 2 };\n",
        "    if caller.as_ref().is_some_and(|k| k.actor.is_some()) {\n",
        "    guard.decision.actor = caller.actor.clone().or(fallback.actor);\n",
        "    let wide = caller.owner.is_some();\n",
    ] {
        assert_eq!(actor_violations("server.rs", hostile).len(), 1, "{hostile}");
    }
    let other_handler = "    let who = record.owner.clone();\n";
    assert_eq!(
        actor_violations("server/explain.rs", other_handler).len(),
        1
    );
}

#[test]
fn the_attribution_code_may_name_an_actor_and_other_words_are_not_mistaken_for_one() {
    for relative in [
        "config/actors.rs",
        "auth.rs",
        "ledger/stats.rs",
        "server/config_view.rs",
        "server/decisions.rs",
        "server/stats.rs",
    ] {
        assert!(
            actor_violations(relative, "let a = k.actor.clone();\n").is_empty(),
            "{relative}"
        );
    }
    // "factor" and "ownership" are not "actor" and "owner".
    assert!(!names_an_actor("let factor = 2; // ownership of the lock"));
    assert!(!names_an_actor("let contractor = 1;"));
    assert!(names_an_actor("GroupBy::Actor => x,"));
    assert!(names_an_actor("let o = ActorKind::Human;"));
}
