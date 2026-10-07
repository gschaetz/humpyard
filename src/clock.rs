//! Time and UTC budget periods. The clock is a trait so tests control rollover.

use time::{Date, OffsetDateTime, Time};

use crate::num::millis_from_nanos;

pub trait Clock: Send + Sync {
    /// Unix milliseconds.
    fn now_ms(&self) -> i64;
}

pub struct SystemClock;

impl Clock for SystemClock {
    fn now_ms(&self) -> i64 {
        millis_from_nanos(OffsetDateTime::now_utc().unix_timestamp_nanos())
    }
}

fn at(now_ms: i64) -> OffsetDateTime {
    OffsetDateTime::from_unix_timestamp_nanos(i128::from(now_ms) * 1_000_000)
        .unwrap_or(OffsetDateTime::UNIX_EPOCH)
}

fn start_of(date: Date) -> i64 {
    millis_from_nanos(
        date.with_time(Time::MIDNIGHT)
            .assume_utc()
            .unix_timestamp_nanos(),
    )
}

/// Start of the UTC calendar day containing `now_ms`.
pub fn day_start_ms(now_ms: i64) -> i64 {
    start_of(at(now_ms).date())
}

/// Start of the UTC calendar month containing `now_ms`.
pub fn month_start_ms(now_ms: i64) -> i64 {
    let date = at(now_ms).date();
    // Day 1 exists in every month, so the fallback is unreachable; it avoids a panic path.
    start_of(date.replace_day(1).unwrap_or(date))
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::Month;

    fn ms(year: i32, month: Month, day: u8, h: u8, m: u8, s: u8) -> i64 {
        let date = Date::from_calendar_date(year, month, day).unwrap();
        let time = Time::from_hms(h, m, s).unwrap();
        millis_from_nanos(date.with_time(time).assume_utc().unix_timestamp_nanos())
    }

    #[test]
    fn periods_start_at_utc_midnight_and_first_of_month() {
        let now = ms(2026, Month::October, 7, 13, 45, 12);
        assert_eq!(day_start_ms(now), ms(2026, Month::October, 7, 0, 0, 0));
        assert_eq!(month_start_ms(now), ms(2026, Month::October, 1, 0, 0, 0));
    }

    #[test]
    fn rollover_at_day_and_month_boundaries() {
        let last_ms = ms(2026, Month::October, 31, 23, 59, 59) + 999;
        assert_eq!(day_start_ms(last_ms), ms(2026, Month::October, 31, 0, 0, 0));
        assert_eq!(
            month_start_ms(last_ms),
            ms(2026, Month::October, 1, 0, 0, 0)
        );
        let next = last_ms + 1;
        assert_eq!(day_start_ms(next), ms(2026, Month::November, 1, 0, 0, 0));
        assert_eq!(month_start_ms(next), ms(2026, Month::November, 1, 0, 0, 0));
    }

    #[test]
    fn leap_day_and_year_end() {
        let leap = ms(2028, Month::February, 29, 12, 0, 0);
        assert_eq!(day_start_ms(leap), ms(2028, Month::February, 29, 0, 0, 0));
        assert_eq!(month_start_ms(leap), ms(2028, Month::February, 1, 0, 0, 0));
        let new_year = ms(2027, Month::January, 1, 0, 0, 0);
        assert_eq!(
            month_start_ms(new_year - 1),
            ms(2026, Month::December, 1, 0, 0, 0)
        );
    }

    #[test]
    fn system_clock_is_after_2026() {
        assert!(SystemClock.now_ms() > ms(2026, Month::January, 1, 0, 0, 0));
    }
}
