//! Checked numeric conversions for money and token arithmetic. Every cast between integer and
//! floating-point types on those paths goes through here, so truncation, sign loss and overflow are
//! decided once, in one tested place, instead of silently by `as`.

/// `u64` to `i64`, saturating at `i64::MAX` (SQLite stores integers as `i64`).
#[must_use]
pub fn to_i64(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

/// `i64` to `u64`, clamping negatives to zero.
#[must_use]
pub fn from_i64(value: i64) -> u64 {
    u64::try_from(value).unwrap_or(0)
}

/// `u64` to `f64`. Exact up to 2^53 (about 9e15); beyond that the nearest representable value,
/// which is far past any realistic token count or micro-USD total.
#[must_use]
#[allow(clippy::cast_precision_loss)] // the single, documented place this lossy cast lives
pub fn f64_from_u64(value: u64) -> f64 {
    value as f64
}

/// A non-negative amount (micro-USD) from a float: rounded to nearest, with NaN and negatives as
/// zero and values beyond `u64::MAX` saturated.
#[must_use]
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // range checked just above
pub fn u64_from_f64_rounded(value: f64) -> u64 {
    // `u64::MAX as f64` rounds up to 2^64, which itself must saturate.
    const LIMIT: f64 = 18_446_744_073_709_551_616.0;
    if value.is_nan() || value <= 0.0 {
        0
    } else if value >= LIMIT {
        u64::MAX
    } else {
        value.round() as u64
    }
}

/// Unix milliseconds from nanoseconds, saturating at the `i64` range.
#[must_use]
pub fn millis_from_nanos(nanos: i128) -> i64 {
    i64::try_from(nanos / 1_000_000).unwrap_or(if nanos < 0 { i64::MIN } else { i64::MAX })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integer_conversions_clamp_instead_of_wrapping() {
        assert_eq!(to_i64(0), 0);
        assert_eq!(to_i64(42), 42);
        assert_eq!(to_i64(u64::MAX), i64::MAX);
        assert_eq!(from_i64(-1), 0);
        assert_eq!(from_i64(i64::MIN), 0);
        assert_eq!(from_i64(i64::MAX), i64::MAX.unsigned_abs());
    }

    #[test]
    fn float_conversion_is_exact_for_realistic_magnitudes() {
        assert_eq!(f64_from_u64(0), 0.0);
        assert_eq!(f64_from_u64(1_000_000), 1_000_000.0);
        assert_eq!(f64_from_u64(1 << 53), 9_007_199_254_740_992.0);
    }

    #[test]
    fn rounding_handles_edges() {
        assert_eq!(u64_from_f64_rounded(0.4), 0);
        assert_eq!(u64_from_f64_rounded(0.5), 1);
        assert_eq!(u64_from_f64_rounded(2.5), 3);
        assert_eq!(u64_from_f64_rounded(-3.0), 0);
        assert_eq!(u64_from_f64_rounded(f64::NAN), 0);
        assert_eq!(u64_from_f64_rounded(f64::NEG_INFINITY), 0);
        assert_eq!(u64_from_f64_rounded(f64::INFINITY), u64::MAX);
        assert_eq!(u64_from_f64_rounded(1e30), u64::MAX);
        assert_eq!(u64_from_f64_rounded(123_456_789.4), 123_456_789);
    }

    #[test]
    fn millisecond_conversion_saturates() {
        assert_eq!(millis_from_nanos(1_500_000), 1);
        assert_eq!(millis_from_nanos(-1_500_000), -1);
        assert_eq!(millis_from_nanos(i128::MAX), i64::MAX);
        assert_eq!(millis_from_nanos(i128::MIN), i64::MIN);
    }
}

#[cfg(test)]
mod properties {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn i64_round_trip_only_loses_what_does_not_fit(n in any::<u64>()) {
            prop_assert_eq!(from_i64(to_i64(n)), n.min(i64::MAX.unsigned_abs()));
        }

        #[test]
        fn rounding_floats_is_monotone_and_never_panics(a in any::<f64>(), b in any::<f64>()) {
            let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
            if !lo.is_nan() && !hi.is_nan() {
                prop_assert!(u64_from_f64_rounded(lo) <= u64_from_f64_rounded(hi));
            }
        }

        #[test]
        fn rounding_matches_f64_round_for_ordinary_amounts(v in 0.0..1.0e15f64) {
            let rounded = f64_from_u64(u64_from_f64_rounded(v));
            prop_assert!((rounded - v.round()).abs() < 1e-9, "{rounded} vs {}", v.round());
        }

        #[test]
        fn millis_conversion_is_monotone(a in any::<i128>(), b in any::<i128>()) {
            let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
            prop_assert!(millis_from_nanos(lo) <= millis_from_nanos(hi));
        }
    }
}
