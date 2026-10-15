//! Calendar helpers for the runner: day-number to `YYYY-MM-DD` and back.
//! Both use Howard Hinnant's civil-from-days algorithm (valid over the whole
//! proleptic Gregorian range the runner will ever see).

/// `YYYY-MM-DD` from a day number since the Unix epoch.
pub fn format_day(day: i64) -> String {
    let z = day + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}")
}

/// Inverse of [`format_day`]; `None` for anything that is not `YYYY-MM-DD`.
pub fn day_number(day: &str) -> Option<i64> {
    let mut parts = day.split('-');
    let y: i64 = parts.next()?.parse().ok()?;
    let m: i64 = parts.next()?.parse().ok()?;
    let d: i64 = parts.next()?.parse().ok()?;
    if parts.next().is_some() || !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = if m > 2 { m - 3 } else { m + 9 };
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some(era * 146_097 + doe - 719_468)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn formatting_round_trips_and_rejects_junk() {
        for day in [-1_000_i64, -1, 0, 1, 20_000, 19_723] {
            assert_eq!(day_number(&format_day(day)), Some(day), "{day}");
        }
        assert_eq!(format_day(0), "1970-01-01");
        assert_eq!(day_number("not-a-day"), None);
        assert_eq!(day_number("2026-13-01"), None);
        assert_eq!(day_number("2026-01-01-01"), None);
    }
}
