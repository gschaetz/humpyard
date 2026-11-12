//! The check selector (`scripts/select-checks.py`, spec: `performance`) against the real register: which checks a
//! change to each kind of file triggers.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test scaffolding

use std::process::Command;

fn select(args: &[&str]) -> Vec<String> {
    let out = Command::new("python3")
        .args(["-I", "scripts/select-checks.py"])
        .args(args)
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("python3 is needed to run the selector");
    assert!(
        out.status.success(),
        "selector failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout)
        .unwrap()
        .lines()
        .map(str::to_string)
        .collect()
}

fn files(paths: &[&str]) -> Vec<String> {
    let mut args = vec!["--files"];
    args.extend(paths);
    select(&args)
}

#[test]
fn a_documentation_only_change_selects_nothing() {
    assert_eq!(
        files(&["docs/api.md", "openspec/changes/x/tasks.md", "README.md"]),
        Vec::<String>::new()
    );
}

#[test]
fn a_request_path_module_selects_the_isolation_checks() {
    let found = files(&["src/routing.rs"]);
    assert!(
        found.contains(&"isolation-admin-reads".to_string()),
        "{found:?}"
    );
    assert!(
        found.contains(&"isolation-prune-contention".to_string()),
        "{found:?}"
    );
}

#[test]
fn a_nested_file_counts_for_its_top_level_module() {
    let found = files(&["src/ledger/stats.rs"]);
    assert!(
        found.contains(&"isolation-admin-reads".to_string()),
        "{found:?}"
    );
}

#[test]
fn an_off_path_module_selects_only_the_architecture_rules() {
    assert_eq!(files(&["src/ctl/overrides.rs"]), vec!["architecture-rules"]);
}

#[test]
fn a_dependency_the_toolchain_and_the_harness_select_the_isolation_checks() {
    for path in [
        "Cargo.lock",
        "rust-toolchain.toml",
        "tests/isolation.rs",
        "tests/common/seed.rs",
    ] {
        let found = files(&[path]);
        assert!(
            found.contains(&"isolation-admin-reads".to_string()),
            "{path} selected {found:?}"
        );
    }
}

#[test]
fn a_run_for_everything_selects_every_hosted_check_and_no_reference_check() {
    let found = select(&["--all"]);
    assert_eq!(
        found,
        vec![
            "architecture-rules",
            "isolation-admin-reads",
            "isolation-prune-contention"
        ]
    );
    assert!(
        !found
            .iter()
            .any(|c| c == "overhead-budgets" || c == "throughput-comparison")
    );
}

#[test]
fn the_jobs_of_a_selection_are_named_once() {
    let mut args = vec!["--jobs", "--files", "src/routing.rs"];
    assert_eq!(select(&args), vec!["check", "isolation"]);
    args = vec!["--jobs", "--files", "docs/api.md"];
    assert_eq!(select(&args), Vec::<String>::new());
}
