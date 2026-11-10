//! The scratch directory of the integration tests (`tests/common`): one directory per test process, and a sweep
//! of those that earlier processes left behind.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // integration tests fail loudly on purpose

mod common;

use std::time::{Duration, SystemTime};

fn age(path: &std::path::Path, by: Duration) {
    let dir = std::fs::File::open(path).unwrap();
    dir.set_modified(SystemTime::now() - by).unwrap();
}

#[test]
fn old_scratch_directories_are_swept_and_recent_ones_and_strangers_are_kept() {
    let base = std::env::temp_dir().join(format!("humpyard-scratch-test-{}", std::process::id()));
    std::fs::create_dir_all(&base).unwrap();
    let stale = base.join("humpyard-tests-1");
    let recent = base.join("humpyard-tests-2");
    let stranger = base.join("someone-elses-directory");
    for dir in [&stale, &recent, &stranger] {
        std::fs::create_dir_all(dir.join("inner")).unwrap();
    }
    age(&stale, Duration::from_hours(2));
    age(&stranger, Duration::from_hours(2));

    common::sweep_stale_scratch(&base);

    assert!(!stale.exists(), "an hour-old scratch directory is removed");
    assert!(recent.exists(), "a recent one may belong to a running test");
    assert!(
        stranger.exists(),
        "only humpyard-tests-* directories are touched"
    );
    std::fs::remove_dir_all(&base).unwrap();
}

#[test]
fn a_process_gets_one_scratch_directory_named_for_it() {
    let root = common::scratch();
    assert!(root.is_dir());
    assert_eq!(
        root.file_name().unwrap().to_string_lossy(),
        format!("humpyard-tests-{}", std::process::id())
    );
    assert_eq!(common::scratch(), root, "the same directory every time");
}
