#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // integration tests fail loudly on purpose
use std::process::Command;

fn run_completions(shell: &str) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_humpyard"))
        .args(["completions", shell])
        .output()
        .unwrap()
}

#[test]
fn completions_generate_scripts_for_known_shells() {
    for shell in ["bash", "elvish", "fish", "powershell", "zsh"] {
        let out = run_completions(shell);
        assert!(out.status.success(), "shell {shell} failed: {out:?}");
        let text = String::from_utf8_lossy(&out.stdout);
        assert!(!text.is_empty(), "completions for {shell} was empty");
        assert!(
            text.contains("ctl") && text.contains("init"),
            "completions for {shell} missing known subcommands: {text}"
        );
    }
}

#[test]
fn completions_rejects_unknown_shell() {
    let out = run_completions("unknown-shell");
    assert!(!out.status.success(), "expected failure on unknown shell");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("invalid value 'unknown-shell'") || err.contains("possible values"),
        "unexpected error message: {err}"
    );
}
