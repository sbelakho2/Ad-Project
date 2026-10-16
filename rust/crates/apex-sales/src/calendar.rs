//! Calendar arithmetic on Unix seconds, without a time-zone database.
//!
//! Civil-date conversion uses Howard Hinnant's `days_from_civil` and
//! `civil_from_days` algorithms (proleptic Gregorian, exact for all `i64`
//! day counts in range). Local time is a fixed UTC offset in minutes; the
//! engine schedules to the hour, so daylight-saving error is at most one
//! hour and is documented where offsets are chosen.

use crate::model::{UnixSecs, DAY};

/// Days since 1970-01-01 for a proleptic Gregorian date.
pub fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// `(year, month, day)` for a day count since 1970-01-01.
pub fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Day of week for a day count, 0 = Sunday ... 6 = Saturday.
pub fn weekday(days: i64) -> i64 {
    (days + 4).rem_euclid(7)
}

/// Local `(days since epoch, hour, weekday)` at `t` for a UTC offset.
pub fn local_parts(t: UnixSecs, offset_minutes: i32) -> (i64, i64, i64) {
    let local = t + i64::from(offset_minutes) * 60;
    let days = local.div_euclid(DAY);
    let hour = local.rem_euclid(DAY) / 3600;
    (days, hour, weekday(days))
}

/// The earliest instant at or after `after` that falls at the start of local
/// `hour` on a weekday (Monday to Friday), for the given UTC offset.
pub fn next_weekday_hour(after: UnixSecs, offset_minutes: i32, hour: u32) -> UnixSecs {
    let hour = i64::from(hour.min(23));
    let offset = i64::from(offset_minutes) * 60;
    let local_after = after + offset;
    let start = local_after.div_euclid(DAY);
    for day in start..start + 8 {
        let candidate = day * DAY + hour * 3600;
        if candidate >= local_after && (1..=5).contains(&weekday(day)) {
            return candidate - offset;
        }
    }
    after
}

/// RFC 5322 `Date:` value in UTC, e.g. `Fri, 16 Oct 2026 10:13:00 +0000`.
pub fn rfc5322_date(t: UnixSecs) -> String {
    const DAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let days = t.div_euclid(DAY);
    let secs = t.rem_euclid(DAY);
    let (y, m, d) = civil_from_days(days);
    let wd = DAYS.get(weekday(days) as usize).copied().unwrap_or("Mon");
    let mon = MONTHS.get((m - 1) as usize).copied().unwrap_or("Jan");
    format!(
        "{wd}, {d:02} {mon} {y} {:02}:{:02}:{:02} +0000",
        secs / 3600,
        (secs % 3600) / 60,
        secs % 60
    )
}

/// `YYYY-MM-DD` of `t` in UTC.
pub fn ymd(t: UnixSecs) -> String {
    let (y, m, d) = civil_from_days(t.div_euclid(DAY));
    format!("{y:04}-{m:02}-{d:02}")
}

/// Typical standard-time UTC offset in minutes for a country (ISO alpha-2).
/// Countries spanning zones use their main business hub. Unknown countries
/// return `None`; callers fall back to UTC.
pub fn country_offset_minutes(country: &str) -> Option<i32> {
    const TABLE: [(&str, i32); 44] = [
        ("US", -300),
        ("CA", -300),
        ("MX", -360),
        ("BR", -180),
        ("AR", -180),
        ("CL", -240),
        ("CO", -300),
        ("GB", 0),
        ("IE", 0),
        ("PT", 0),
        ("ES", 60),
        ("FR", 60),
        ("DE", 60),
        ("NL", 60),
        ("BE", 60),
        ("IT", 60),
        ("PL", 60),
        ("SE", 60),
        ("DK", 60),
        ("NO", 60),
        ("CH", 60),
        ("AT", 60),
        ("CZ", 60),
        ("FI", 120),
        ("EE", 120),
        ("LT", 120),
        ("LV", 120),
        ("UA", 120),
        ("RO", 120),
        ("GR", 120),
        ("IL", 120),
        ("CY", 120),
        ("TR", 180),
        ("RU", 180),
        ("BY", 180),
        ("AE", 240),
        ("IN", 330),
        ("VN", 420),
        ("TH", 420),
        ("ID", 420),
        ("CN", 480),
        ("SG", 480),
        ("KR", 540),
        ("JP", 540),
    ];
    let c = country.trim().to_ascii_uppercase();
    TABLE.iter().find(|(k, _)| *k == c).map(|(_, v)| *v)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_round_trips_across_eras() {
        for (y, m, d) in [
            (1970, 1, 1),
            (2000, 2, 29),
            (1969, 12, 31),
            (2026, 10, 16),
            (1600, 3, 1),
        ] {
            let z = days_from_civil(y, m, d);
            assert_eq!(civil_from_days(z), (y, m, d));
        }
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(days_from_civil(2026, 10, 16), 20_742);
        assert_eq!(weekday(0), 4);
        assert_eq!(weekday(20_742), 5);
        assert_eq!(weekday(-1), 3);
    }

    #[test]
    fn formats_dates() {
        assert_eq!(rfc5322_date(0), "Thu, 01 Jan 1970 00:00:00 +0000");
        let t = 20_742 * DAY + 10 * 3600 + 13 * 60;
        assert_eq!(rfc5322_date(t), "Fri, 16 Oct 2026 10:13:00 +0000");
        assert_eq!(ymd(t), "2026-10-16");
    }

    #[test]
    fn schedules_the_next_weekday_hour_in_local_time() {
        // Friday 16 Oct 2026 10:13 UTC; Berlin is +60 in this table.
        let now = 20_742 * DAY + 10 * 3600 + 13 * 60;
        let t = next_weekday_hour(now, 60, 14);
        assert_eq!(t, 20_742 * DAY + 13 * 3600);
        // 09:00 local has passed today, so the next is Monday 19 Oct.
        let t = next_weekday_hour(now, 60, 9);
        assert_eq!(t, 20_745 * DAY + 8 * 3600);
        assert_eq!(local_parts(t, 60), (20_745, 9, 1));
        // A negative offset that crosses midnight.
        let t = next_weekday_hour(now, -300, 9);
        assert_eq!(local_parts(t, -300).1, 9);
        assert!(t >= now);
    }

    #[test]
    fn country_offsets() {
        assert_eq!(country_offset_minutes("de"), Some(60));
        assert_eq!(country_offset_minutes("IN"), Some(330));
        assert_eq!(country_offset_minutes("ZZ"), None);
    }
}
