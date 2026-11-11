//! China's days off and the weekend days worked to make up for them, as the General Office of the
//! State Council sets them for each year, usually in the November before. They go by Beijing's
//! dates, wherever the user is.

use chrono::DateTime;
use serde::Serialize;
use ts_rs::TS;

/// Days as `YYYY-MM-DD`, which sort as the days do.
#[derive(Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct Holidays {
    /// Each holiday's days off, from and until which day.
    pub days_off: &'static [(&'static str, &'static str)],
    pub make_up_days: &'static [&'static str],
}

pub const HOLIDAYS: Holidays = Holidays {
    days_off: &[
        // 2026, as the notice of 2025-11-04 set them.
        ("2026-01-01", "2026-01-03"),
        ("2026-02-15", "2026-02-23"),
        ("2026-04-04", "2026-04-06"),
        ("2026-05-01", "2026-05-05"),
        ("2026-06-19", "2026-06-21"),
        ("2026-09-25", "2026-09-27"),
        ("2026-10-01", "2026-10-07"),
        // 2027, as the notice of 2026-11-05 set them.
        ("2027-01-01", "2027-01-03"),
        ("2027-02-05", "2027-02-13"),
        ("2027-04-03", "2027-04-05"),
        ("2027-05-01", "2027-05-05"),
        ("2027-06-09", "2027-06-09"),
        ("2027-09-15", "2027-09-15"),
        ("2027-10-01", "2027-10-07"),
    ],
    make_up_days: &[
        "2026-01-04",
        "2026-02-14",
        "2026-02-28",
        "2026-05-09",
        "2026-09-20",
        "2026-10-10",
        "2027-01-31",
        "2027-02-14",
        "2027-05-08",
        "2027-09-26",
        "2027-10-09",
    ],
};

/// Whether `at`, in UTC milliseconds, falls on one of a holiday's days off. Weekends are not.
pub fn day_off(at: i64) -> bool {
    // Beijing keeps UTC+8 all year.
    DateTime::from_timestamp_millis(at + 8 * 3_600_000).is_some_and(|time| {
        let day = time.date_naive().to_string();
        HOLIDAYS
            .days_off
            .iter()
            .any(|&(from, until)| (from..=until).contains(&day.as_str()))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Datelike, NaiveDate, Weekday};

    /// `at` for a time in UTC.
    fn utc(day: &str, hour: u32, minute: u32) -> i64 {
        let day: NaiveDate = day.parse().unwrap();
        day.and_hms_opt(hour, minute, 0)
            .unwrap()
            .and_utc()
            .timestamp_millis()
    }

    #[test]
    fn days_off_go_by_beijing_dates() {
        // 2026's National Day holiday: 1 to 7 October, 16:00 UTC the day before being midnight.
        assert!(!day_off(utc("2026-09-30", 15, 59)));
        assert!(day_off(utc("2026-09-30", 16, 0)));
        assert!(day_off(utc("2026-10-04", 2, 0)));
        assert!(day_off(utc("2026-10-07", 15, 59)));
        assert!(!day_off(utc("2026-10-07", 16, 0)));
        // A weekend day worked is not one.
        assert!(!day_off(utc("2026-09-20", 2, 0)));
    }

    /// Catches a slip in a year's dates when they are added.
    #[test]
    fn the_table_holds_together() {
        let day = |text: &str| {
            let day: NaiveDate = text.parse().unwrap();
            // Written in full, or it would not sort as the days do.
            assert_eq!(day.to_string(), text);
            day
        };
        for &(from, until) in HOLIDAYS.days_off {
            assert!(day(from) <= day(until), "{from}");
        }
        for &text in HOLIDAYS.make_up_days {
            assert!(
                matches!(day(text).weekday(), Weekday::Sat | Weekday::Sun),
                "{text}"
            );
        }
    }
}
