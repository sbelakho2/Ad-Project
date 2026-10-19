//! UTC millisecond timestamps for the event envelope.
//!
//! The workspace has no calendar dependency, and the analytics store only
//! needs one timestamp shape: milliseconds since the Unix epoch in UTC,
//! rendered in the two spellings the boundary uses (RFC 3339 with a `Z`
//! designator for JSON, `YYYY-MM-DD hh:mm:ss.fff` for ClickHouse
//! `DateTime64(3)`). The civil-date conversion is the standard
//! days-to-civil algorithm, pinned by tests against known epoch anchors.

/// Milliseconds in one second.
pub const MILLIS_PER_SECOND: i64 = 1_000;
/// Milliseconds in one day.
pub const MILLIS_PER_DAY: i64 = 86_400_000;

/// A UTC instant at millisecond precision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct UtcMillis(i64);

impl serde::Serialize for UtcMillis {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_rfc3339_millis())
    }
}

impl<'de> serde::Deserialize<'de> for UtcMillis {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Visitor;
        impl serde::de::Visitor<'_> for Visitor {
            type Value = UtcMillis;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("an RFC 3339 UTC timestamp")
            }
            fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<UtcMillis, E> {
                UtcMillis::parse_rfc3339(value).map_err(E::custom)
            }
        }
        deserializer.deserialize_str(Visitor)
    }
}

/// Timestamp parsing and clock errors.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TimeError {
    /// The text is not an RFC 3339 (or ClickHouse-spaced) UTC timestamp.
    #[error("timestamp '{0}' is not an RFC 3339 UTC timestamp")]
    Format(String),
    /// A component of an otherwise well-formed timestamp is out of range.
    #[error("timestamp component '{0}' is out of range")]
    Range(&'static str),
    /// The system clock reads before the Unix epoch.
    #[error("system clock is before the Unix epoch")]
    Clock,
}

impl UtcMillis {
    /// Wrap a raw milliseconds-since-epoch value.
    pub fn from_unix_millis(millis: i64) -> Self {
        Self(millis)
    }

    /// The raw milliseconds-since-epoch value.
    pub fn unix_millis(self) -> i64 {
        self.0
    }

    /// The current instant, or [`TimeError::Clock`] on a pre-epoch clock.
    pub fn system_now() -> Result<Self, TimeError> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| TimeError::Clock)?;
        i64::try_from(now.as_millis())
            .map(Self)
            .map_err(|_| TimeError::Clock)
    }

    /// Parse `YYYY-MM-DDThh:mm:ss[.fraction][Z|+hh:mm|-hh:mm]`.
    ///
    /// A space may stand in for `T` because ClickHouse renders
    /// `DateTime64` that way, and the spaced spelling carries no zone
    /// designator (it is UTC by definition). Fractional digits beyond
    /// milliseconds are truncated and offsets are folded into UTC. A `T`
    /// spelling without a zone designator is a format error: an ambiguous
    /// local time is never silently accepted as UTC.
    pub fn parse_rfc3339(text: &str) -> Result<Self, TimeError> {
        let bad = || TimeError::Format(text.to_string());
        let (date, rest, spaced) = match split_once_byte(text, b'T') {
            Some((date, rest)) => (date, rest, false),
            None => match split_once_byte(text, b' ') {
                Some((date, rest)) => (date, rest, true),
                None => return Err(bad()),
            },
        };
        let (year, month, day) = parse_date(date).ok_or_else(bad)?;
        if !valid_ymd(year, month, day) {
            return Err(TimeError::Range("date"));
        }
        // The RFC 3339 `T` spelling requires an explicit zone; the spaced
        // ClickHouse `DateTime64` rendering is UTC by definition.
        let (time, offset_minutes) = match split_offset(rest) {
            Some(pair) => pair,
            None if spaced => (rest, 0),
            None => return Err(bad()),
        };
        let (hour, minute, second, millis) = parse_time(time).ok_or_else(bad)?;
        if hour > 23 || minute > 59 || second > 59 {
            return Err(TimeError::Range("time"));
        }
        let days = days_from_civil(year, month, day);
        let seconds = days
            .checked_mul(86_400)
            .and_then(|s| s.checked_add((hour as i64) * 3_600))
            .and_then(|s| s.checked_add((minute as i64) * 60))
            .and_then(|s| s.checked_add(second as i64))
            .and_then(|s| s.checked_sub(offset_minutes as i64 * 60))
            .ok_or(TimeError::Range("value"))?;
        seconds
            .checked_mul(MILLIS_PER_SECOND)
            .and_then(|s| s.checked_add(millis))
            .map(Self)
            .ok_or(TimeError::Range("value"))
    }

    /// RFC 3339 rendering at millisecond precision, e.g.
    /// `2026-10-15T09:10:00.000Z`.
    pub fn to_rfc3339_millis(self) -> String {
        let (days, millis_of_day) = self.split_days();
        let (year, month, day) = civil_from_days(days);
        let (hour, minute, second, millis) = split_millis_of_day(millis_of_day);
        format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{millis:03}Z")
    }

    /// ClickHouse `DateTime64(3)` rendering, e.g. `2026-10-15 09:10:00.000`.
    pub fn to_clickhouse(self) -> String {
        let (days, millis_of_day) = self.split_days();
        let (year, month, day) = civil_from_days(days);
        let (hour, minute, second, millis) = split_millis_of_day(millis_of_day);
        format!("{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}:{second:02}.{millis:03}")
    }

    /// The UTC calendar day as `YYYY-MM-DD`.
    pub fn to_day_string(self) -> String {
        let (days, _) = self.split_days();
        let (year, month, day) = civil_from_days(days);
        format!("{year:04}-{month:02}-{day:02}")
    }

    /// The UTC calendar month as `YYYY-MM`. Cohort folds only need the month;
    /// formatting the day first and then slicing it allocated two strings per
    /// event, so this renders the month directly.
    pub fn to_month_string(self) -> String {
        let (days, _) = self.split_days();
        let (year, month, _) = civil_from_days(days);
        if (0..=9_999).contains(&year) {
            return format!("{year:04}-{month:02}");
        }
        // Preserve the historical seven-character projection for years that
        // render with more than four digits.
        let day = self.to_day_string();
        day.get(..7).map_or_else(|| day.clone(), str::to_string)
    }

    /// The calendar day index (days since 1970-01-01) of this instant.
    pub fn day_index(self) -> i64 {
        self.split_days().0
    }

    /// The calendar month as a comparable integer key: `year * 12 + month -
    /// 1`. Cohort folds group by this key and format the `YYYY-MM` label only
    /// for the rows they emit, so a month label is no longer allocated per
    /// event.
    pub fn month_index(self) -> i64 {
        let (days, _) = self.split_days();
        let (year, month, _) = civil_from_days(days);
        year.saturating_mul(12).saturating_add(i64::from(month) - 1)
    }

    fn split_days(self) -> (i64, i64) {
        (
            self.0.div_euclid(MILLIS_PER_DAY),
            self.0.rem_euclid(MILLIS_PER_DAY),
        )
    }
}

fn split_millis_of_day(millis_of_day: i64) -> (i64, i64, i64, i64) {
    let seconds = millis_of_day / MILLIS_PER_SECOND;
    let millis = millis_of_day % MILLIS_PER_SECOND;
    (seconds / 3_600, (seconds / 60) % 60, seconds % 60, millis)
}

fn split_once_byte(text: &str, byte: u8) -> Option<(&str, &str)> {
    let position = text.as_bytes().iter().position(|b| *b == byte)?;
    Some((&text[..position], &text[position + 1..]))
}

fn parse_date(date: &str) -> Option<(i64, u32, u32)> {
    let mut parts = date.split('-');
    let year = parts.next()?.parse::<i64>().ok()?;
    let month = parts.next()?.parse::<u32>().ok()?;
    let day = parts.next()?.parse::<u32>().ok()?;
    if parts.next().is_some() || date.len() != 10 {
        return None;
    }
    Some((year, month, day))
}

fn parse_time(time: &str) -> Option<(u32, u32, u32, i64)> {
    let (whole, fraction) = match split_once_byte(time, b'.') {
        Some((whole, fraction)) => (whole, Some(fraction)),
        None => (time, None),
    };
    let mut parts = whole.split(':');
    let hour = parts.next()?.parse::<u32>().ok()?;
    let minute = parts.next()?.parse::<u32>().ok()?;
    let second = parts.next()?.parse::<u32>().ok()?;
    if parts.next().is_some() || whole.len() != 8 {
        return None;
    }
    let millis = match fraction {
        None => 0,
        Some(fraction) => {
            if fraction.is_empty()
                || fraction.len() > 9
                || !fraction.bytes().all(|b| b.is_ascii_digit())
            {
                return None;
            }
            let mut digits = fraction[..fraction.len().min(3)].to_string();
            while digits.len() < 3 {
                digits.push('0');
            }
            digits.parse::<i64>().ok()?
        }
    };
    Some((hour, minute, second, millis))
}

/// Split a trailing `Z` or `+hh:mm`/`-hh:mm` zone designator.
fn split_offset(rest: &str) -> Option<(&str, i32)> {
    if let Some(time) = rest.strip_suffix('Z') {
        return Some((time, 0));
    }
    let (time, minutes) = if let Some((time, offset)) = split_once_byte(rest, b'+') {
        (time, parse_offset(offset)?)
    } else if let Some((index, _)) = rest.char_indices().skip(1).find(|(_, c)| *c == '-') {
        (&rest[..index], -parse_offset(&rest[index + 1..])?)
    } else {
        return None;
    };
    Some((time, minutes))
}

fn parse_offset(offset: &str) -> Option<i32> {
    let (hours, minutes) = split_once_byte(offset, b':')?;
    let hours = hours.parse::<i32>().ok()?;
    let minutes = minutes.parse::<i32>().ok()?;
    if hours > 23 || minutes > 59 {
        return None;
    }
    Some(hours * 60 + minutes)
}

fn valid_ymd(year: i64, month: u32, day: u32) -> bool {
    (1..=12).contains(&month) && day >= 1 && day <= days_in_month(year, month)
}

fn days_in_month(year: i64, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => 0,
    }
}

fn is_leap_year(year: i64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

/// Days since 1970-01-01 for a proleptic Gregorian date
/// (Howard Hinnant's `days_from_civil`).
fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let month = month as i64;
    let day = day as i64;
    let day_of_year = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// Inverse of [`days_from_civil`] (Hinnant's `civil_from_days`).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let days = days + 719_468;
    let era = if days >= 0 { days } else { days - 146_096 } / 146_097;
    let day_of_era = days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * month_prime + 2) / 5 + 1) as u32;
    let month = (month_prime + if month_prime < 10 { 3 } else { -9 }) as u32;
    let year = if month <= 2 { year + 1 } else { year };
    (year, month, day)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn epoch_anchors_hold() {
        assert_eq!(
            UtcMillis::parse_rfc3339("1970-01-01T00:00:00Z")
                .expect("epoch")
                .unix_millis(),
            0
        );
        assert_eq!(
            UtcMillis::parse_rfc3339("2000-01-01T00:00:00Z")
                .expect("y2k")
                .unix_millis(),
            946_684_800_000
        );
        assert_eq!(
            UtcMillis::parse_rfc3339("2038-01-19T03:14:08Z")
                .expect("y2038")
                .unix_millis(),
            2_147_483_648_000
        );
        assert_eq!(
            UtcMillis::parse_rfc3339("1969-12-31T23:59:59.999Z")
                .expect("pre-epoch")
                .unix_millis(),
            -1
        );
    }

    #[test]
    fn leap_day_round_trips() {
        let instant = UtcMillis::parse_rfc3339("2024-02-29T12:34:56.789Z").expect("leap");
        assert_eq!(instant.to_rfc3339_millis(), "2024-02-29T12:34:56.789Z");
        assert_eq!(instant.to_clickhouse(), "2024-02-29 12:34:56.789");
        assert_eq!(instant.to_day_string(), "2024-02-29");
    }

    #[test]
    fn fractional_seconds_truncate_to_millis() {
        let instant = UtcMillis::parse_rfc3339("2026-10-15T09:10:00.123456789Z").expect("frac");
        assert_eq!(instant.unix_millis() % 1_000, 123);
        let short = UtcMillis::parse_rfc3339("2026-10-15T09:10:00.5Z").expect("short frac");
        assert_eq!(short.unix_millis() % 1_000, 500);
    }

    #[test]
    fn offsets_fold_into_utc() {
        let plus = UtcMillis::parse_rfc3339("2026-10-15T12:10:00+03:00").expect("plus");
        let utc = UtcMillis::parse_rfc3339("2026-10-15T09:10:00Z").expect("utc");
        assert_eq!(plus, utc);
        let minus = UtcMillis::parse_rfc3339("2026-10-15T04:10:00-05:00").expect("minus");
        assert_eq!(minus, utc);
    }

    #[test]
    fn clickhouse_spelling_parses() {
        let spaced = UtcMillis::parse_rfc3339("2026-10-15 09:10:00.000").expect("spaced");
        assert_eq!(spaced.to_rfc3339_millis(), "2026-10-15T09:10:00.000Z");
    }

    #[test]
    fn malformed_timestamps_are_refused() {
        for bad in [
            "",
            "2026-10-15",
            "2026-10-15T09:10:00",
            "2026-13-01T00:00:00Z",
            "2026-02-30T00:00:00Z",
            "2026-10-15T24:00:00Z",
            "2026-10-15T09:60:00Z",
            "2026-10-15T09:10:60Z",
            "2026-10-15T09:10:00.Z",
            "2026-10-15T09:10:0Z",
            "2026-10-15T09:10:00Z+01:00",
            "2026-10-15T09:10:00+25:00",
        ] {
            assert!(
                UtcMillis::parse_rfc3339(bad).is_err(),
                "{bad} must be refused"
            );
        }
    }

    #[test]
    fn display_and_day_index_stay_consistent() {
        let instant = UtcMillis::parse_rfc3339("2026-10-15T09:10:00.000Z").expect("instant");
        assert_eq!(instant.day_index(), days_from_civil(2026, 10, 15));
        assert_eq!(instant.to_day_string(), "2026-10-15");
        assert_eq!(instant.to_month_string(), "2026-10");
        assert_eq!(&instant.to_day_string()[..7], instant.to_month_string());
        assert_eq!(
            UtcMillis::from_unix_millis(instant.unix_millis()).to_rfc3339_millis(),
            "2026-10-15T09:10:00.000Z"
        );
    }

    #[test]
    fn timestamps_round_trip_through_serde() {
        let instant = UtcMillis::parse_rfc3339("2026-10-15T09:10:00.000Z").expect("instant");
        let json = serde_json::to_string(&instant).expect("serialize");
        assert_eq!(json, "\"2026-10-15T09:10:00.000Z\"");
        let back: UtcMillis = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, instant);
        assert!(serde_json::from_str::<UtcMillis>("\"never\"").is_err());
    }

    #[test]
    fn time_errors_name_their_shape() {
        let format = TimeError::Format("x".into());
        assert!(format.to_string().contains("is not an RFC 3339"));
        let range = TimeError::Range("date");
        assert!(range.to_string().contains("out of range"));
        let clock = TimeError::Clock;
        assert!(clock.to_string().contains("Unix epoch"));
        let scaled = UtcMillis::parse_rfc3339("9999-12-31T23:59:59.999Z").expect("far future");
        assert_eq!(scaled.to_day_string(), "9999-12-31");
    }

    #[test]
    fn year_boundary_round_trips() {
        for text in [
            "1999-12-31T23:59:59.999Z",
            "2000-01-01T00:00:00.000Z",
            "2100-03-01T00:00:00.000Z",
            "1970-01-01T00:00:00.001Z",
        ] {
            let instant = UtcMillis::parse_rfc3339(text).expect("parse");
            assert_eq!(instant.to_rfc3339_millis(), text);
        }
    }
}
