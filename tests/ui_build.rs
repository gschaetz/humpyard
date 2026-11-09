//! The `ui` feature embeds the built admin GUI and fails the build without it (spec: `release-packaging`), so a
//! release can never ship the placeholder by accident (ADR 0032).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test scaffolding

use std::process::Command;

/// Builds the library with the `ui` feature against a directory of GUI files that is missing or empty.
fn build_with_dist(dist: &std::path::Path) -> (bool, String) {
    let output = Command::new(env!("CARGO"))
        .args(["build", "--lib", "--features", "ui", "--locked", "--quiet"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .env("HUMPYARD_UI_DIST", dist)
        .output()
        .unwrap();
    (
        output.status.success(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[test]
fn a_build_with_the_ui_feature_and_no_gui_files_fails_and_says_why() {
    let empty = tempfile::tempdir().unwrap();
    let (ok, stderr) = build_with_dist(empty.path());
    assert!(!ok, "the build succeeded without the GUI files");
    assert!(stderr.contains("has no index.html"), "{stderr}");
    assert!(
        stderr.contains("npm run build"),
        "the message says how to build it: {stderr}"
    );

    let (ok, stderr) = build_with_dist(&empty.path().join("missing"));
    assert!(!ok);
    assert!(stderr.contains("has no index.html"), "{stderr}");
}
