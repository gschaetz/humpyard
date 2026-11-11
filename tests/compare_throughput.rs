//! The throughput comparison of `scripts/compare-throughput.sh`, judged on given figures (`--numbers`), so no
//! benchmark is run: a regression, a noisy machine, and the arguments it refuses.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // integration tests fail loudly on purpose

use std::process::Command;

struct Outcome {
    status: i32,
    stdout: String,
    stderr: String,
}

fn judge(numbers: &str, enforce: bool) -> Outcome {
    let mut command = Command::new("sh");
    command
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/scripts/compare-throughput.sh"
        ))
        .args(["--numbers", numbers])
        .env_remove("HUMPYARD_BUDGET_ENFORCE");
    if enforce {
        command.env("HUMPYARD_BUDGET_ENFORCE", "1");
    }
    let output = command.output().unwrap();
    Outcome {
        status: output.status.code().unwrap(),
        stdout: String::from_utf8(output.stdout).unwrap(),
        stderr: String::from_utf8(output.stderr).unwrap(),
    }
}

/// Previous and current, three pairs: the current build within a percent of the previous.
const LEVEL: &str = "47000,46500,46800,47200,47100,46900";
/// The current build 14 to 15% below the previous in every pair, and steady.
const REGRESSED: &str = "47000,40000,46800,40500,47100,40200";
/// The median ratio is fine, but the current build's own runs differ by 13%.
const NOISY: &str = "47000,46000,46800,40000,47100,45900";

#[test]
fn a_level_comparison_is_ok_and_exits_zero_even_when_enforced() {
    for enforce in [false, true] {
        let o = judge(LEVEL, enforce);
        assert_eq!(o.status, 0, "{}{}", o.stdout, o.stderr);
        assert!(o.stdout.contains("verdict: OK"), "{}", o.stdout);
        assert!(o.stdout.contains("median ratio 0.996"), "{}", o.stdout);
    }
}

#[test]
fn a_regression_is_reported_and_fails_only_when_enforced() {
    let reported = judge(REGRESSED, false);
    assert_eq!(reported.status, 0, "{}", reported.stdout);
    assert!(
        reported.stdout.contains("verdict: REGRESSION"),
        "{}",
        reported.stdout
    );
    assert!(
        reported.stdout.contains("median ratio 0.854"),
        "{}",
        reported.stdout
    );

    let enforced = judge(REGRESSED, true);
    assert_eq!(enforced.status, 1, "{}", enforced.stdout);
    assert!(
        enforced.stdout.contains("verdict: REGRESSION"),
        "{}",
        enforced.stdout
    );
}

#[test]
fn a_noisy_machine_is_inconclusive_and_fails_an_enforced_run() {
    let reported = judge(NOISY, false);
    assert_eq!(reported.status, 0, "{}", reported.stdout);
    assert!(
        reported.stdout.contains("verdict: INCONCLUSIVE"),
        "{}",
        reported.stdout
    );
    assert!(
        reported.stdout.contains("current 13.0%"),
        "{}",
        reported.stdout
    );

    let enforced = judge(NOISY, true);
    assert_eq!(
        enforced.status, 1,
        "an inconclusive comparison must not pass: {}",
        enforced.stdout
    );
}

#[test]
fn a_regression_on_a_noisy_machine_says_both() {
    // 14% down and the current build's own runs 20% apart.
    let o = judge("47000,40000,46800,34000,47100,40200", false);
    assert!(
        o.stdout.contains("REGRESSION (and the runs are noisy)"),
        "{}",
        o.stdout
    );
}

#[test]
fn fewer_than_three_pairs_and_an_odd_count_are_refused() {
    let few = judge("47000,46500,46800,47200", true);
    assert_eq!(few.status, 2, "{}{}", few.stdout, few.stderr);
    assert!(few.stderr.contains("at least 3 pairs"), "{}", few.stderr);
    let odd = judge("1,2,3,4,5", true);
    assert_eq!(odd.status, 2, "{}{}", odd.stdout, odd.stderr);
    assert!(odd.stderr.contains("even count"), "{}", odd.stderr);
}

#[test]
fn an_even_number_of_pairs_uses_the_mean_of_the_middle_two() {
    // Ratios 0.80, 0.90, 1.00, 1.10: the median is 0.95.
    let o = judge("1000,800,1000,900,1000,1000,1000,1100", false);
    assert!(o.stdout.contains("median ratio 0.950"), "{}", o.stdout);
}
