//! Money, time and number formatting for every Apex surface.
//!
//! Rules this module encodes (from the design document):
//!
//! - Money is integer micros plus an explicit currency. [`money`] renders
//!   `USD 1,250.00`; [`money_fixed`] renders the symbol form `$1,250.00` only
//!   when the currency context is fixed. The current mapping is USD to `$`,
//!   EUR to `€` and GBP to `£` (case-insensitive currency codes); any unknown
//!   code keeps the code form `CODE 1,250.00`.
//! - [`ecpm`] is the same fixed-currency rendering for a currency-per-1000
//!   impression price; callers append the unit (`$4.00 eCPM`).
//! - [`percent`] formats a *ratio* with one decimal (`0.123` becomes `12.3%`).
//!   [`delta`] formats the signed change between two ratio-scale baselines;
//!   a zero baseline cannot be compared and returns
//!   [`or_not_available`] with the reason, never a fabricated zero.
//! - Missing is not zero: [`or_not_available`] renders `Not available (reason)`
//!   for absent data. A real zero displays as `0` with a known denominator.
//! - Timestamps are complete UTC dates with a named timezone; exports keep
//!   UTC. [`csv_field`] is RFC 4180 quoting.

/// The month abbreviations used by the UTC date formats.
const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// Format micros with an explicit currency code: `USD 1,250.00`.
///
/// Rounds to the nearest cent (half away from zero) so exactly two decimals
/// are always shown; the integer micros stay recoverable via
/// [`micros_exact`].
pub fn money(micros: i64, currency: &str) -> String {
    format!("{currency} {}", amount_2dp(micros))
}

/// Format micros in a fixed currency context: `$1,250.00` for USD.
///
/// The symbol mapping is USD to `$`, EUR to `€`, GBP to `£` (codes match
/// case-insensitively). An unknown currency stays in code form,
/// `CODE 1,250.00`, so it is never mistaken for a currency this module
/// knows.
pub fn money_fixed(micros: i64, currency: &str) -> String {
    match currency_symbol(currency) {
        Some(symbol) => format!("{symbol}{}", amount_2dp(micros)),
        None => format!("{currency} {}", amount_2dp(micros)),
    }
}

/// Format the exact integer micros value with thousands separators,
/// `1,250,000,000`. Technical details use this; customer-facing amounts use
/// [`money`].
pub fn micros_exact(micros: i64) -> String {
    let value = i128::from(micros);
    if value < 0 {
        format!("-{}", group(value.unsigned_abs()))
    } else {
        group(value.unsigned_abs())
    }
}

/// Render a Unix millisecond timestamp as `14 Oct 2026, 08:10 UTC`.
pub fn utc_datetime(unix_ms: i64) -> String {
    let (civil, hour, minute) = utc_parts(unix_ms);
    format!(
        "{:02} {} {}, {:02}:{:02} UTC",
        civil.day,
        month_name(civil.month),
        civil.year,
        hour,
        minute
    )
}

/// Render a Unix millisecond timestamp as `14 Oct 2026` (UTC).
pub fn date_short(unix_ms: i64) -> String {
    let (civil, _, _) = utc_parts(unix_ms);
    format!(
        "{:02} {} {}",
        civil.day,
        month_name(civil.month),
        civil.year
    )
}

/// Render a currency-per-1000-impressions price in the fixed currency
/// context, e.g. `$4.00`. Callers append the `eCPM` unit.
pub fn ecpm(micros_per_thousand: i64, currency: &str) -> String {
    money_fixed(micros_per_thousand, currency)
}

/// Format a ratio as a percentage with one decimal: `0.123` becomes
/// `12.3%`.
///
/// Non-finite inputs (NaN, infinities) return [`or_not_available`] rather
/// than a meaningless percentage.
pub fn percent(value: f64) -> String {
    if !value.is_finite() {
        return or_not_available("value is not a finite number");
    }
    if value == 0.0 {
        return "0.0%".to_string();
    }
    let scaled = value * 100.0;
    if !scaled.is_finite() {
        return or_not_available("value is out of range");
    }
    format!("{scaled:.1}%")
}

/// Format the signed change from `from` to `to` as a percentage with one
/// decimal: `+3.0%`, `-12.5%`, `0.0%`.
///
/// Both inputs are ratio-scale baselines. A zero baseline has no percentage
/// change and returns [`or_not_available`] with that reason; non-finite
/// inputs do the same.
pub fn delta(from: f64, to: f64) -> String {
    if !from.is_finite() || !to.is_finite() {
        return or_not_available("baseline or result is not a finite number");
    }
    if from == 0.0 {
        return or_not_available("zero baseline");
    }
    let change = (to - from) / from;
    if !change.is_finite() {
        return or_not_available("change is out of range");
    }
    if change == 0.0 {
        return "0.0%".to_string();
    }
    format!("{:+.1}%", change * 100.0)
}

/// Render absent data as `Not available (reason)`.
///
/// A missing value is not zero: callers pass the reason the value is absent
/// (`zero baseline`, `no data`, `report is provisional`) so the reader can
/// act on it. An empty reason renders the bare `Not available`.
pub fn or_not_available(reason: &str) -> String {
    let reason = reason.trim();
    if reason.is_empty() {
        "Not available".to_string()
    } else {
        format!("Not available ({reason})")
    }
}

/// Quote a CSV field per RFC 4180: fields containing a comma, quote,
/// carriage return or line feed are wrapped in double quotes with inner
/// quotes doubled; everything else passes through unchanged.
pub fn csv_field(value: &str) -> String {
    if value.contains(['"', ',', '\n', '\r']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}

fn currency_symbol(currency: &str) -> Option<&'static str> {
    match currency.to_ascii_uppercase().as_str() {
        "USD" => Some("$"),
        "EUR" => Some("\u{20ac}"),
        "GBP" => Some("\u{a3}"),
        _ => None,
    }
}

/// Cents-precision rendering of micros: sign, grouped units and exactly two
/// decimals, rounded half away from zero.
fn amount_2dp(micros: i64) -> String {
    let value = i128::from(micros);
    let negative = value < 0;
    let abs = value.unsigned_abs();
    let mut units = abs / 1_000_000;
    let remainder = abs % 1_000_000;
    let mut cents = (remainder + 5_000) / 10_000;
    if cents == 100 {
        cents = 0;
        units += 1;
    }
    // A value that rounds to exactly zero is zero, not a negative zero.
    let sign = if negative && (units > 0 || cents > 0) {
        "-"
    } else {
        ""
    };
    format!("{sign}{}.{:02}", group(units), cents)
}

/// Group a non-negative integer with commas in threes.
fn group(value: u128) -> String {
    let digits = value.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    let len = digits.len();
    for (index, ch) in digits.chars().enumerate() {
        if index > 0 && (len - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

/// A civil UTC date.
struct CivilDate {
    year: i64,
    month: usize,
    day: u32,
}

fn month_name(month: usize) -> &'static str {
    MONTHS.get(month.wrapping_sub(1)).copied().unwrap_or("Jan")
}

/// Split Unix milliseconds into the civil UTC date, hour and minute.
fn utc_parts(unix_ms: i64) -> (CivilDate, i64, i64) {
    let ms = i128::from(unix_ms);
    let seconds = ms.div_euclid(1_000);
    let days = seconds.div_euclid(86_400);
    let second_of_day = seconds.rem_euclid(86_400);
    // |days| for any i64 millisecond value stays far inside i64.
    let civil = civil_from_days(days as i64);
    (
        civil,
        (second_of_day / 3_600) as i64,
        ((second_of_day % 3_600) / 60) as i64,
    )
}

/// Howard Hinnant's `civil_from_days`: days since 1970-01-01 to a civil date.
fn civil_from_days(days: i64) -> CivilDate {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    CivilDate {
        year: year + i64::from(month <= 2),
        month: month as usize,
        day: day as u32,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OCT_14_2026_0810: i64 = 1_791_965_400_000;

    #[test]
    fn money_uses_code_grouping_and_two_decimals() {
        assert_eq!(money(1_250_000_000, "USD"), "USD 1,250.00");
        assert_eq!(money(0, "USD"), "USD 0.00");
        assert_eq!(money(1, "USD"), "USD 0.00");
        assert_eq!(money(9_999, "USD"), "USD 0.01");
        assert_eq!(money(10_000, "USD"), "USD 0.01");
        assert_eq!(money(999_995, "USD"), "USD 1.00");
        assert_eq!(money(123_456_789, "EUR"), "EUR 123.46");
        assert_eq!(money(1_000_000, "SEK"), "SEK 1.00");
    }

    #[test]
    fn money_handles_negative_and_extreme_values() {
        assert_eq!(money(-1_250_000_000, "USD"), "USD -1,250.00");
        assert_eq!(money(-1, "USD"), "USD 0.00");
        assert_eq!(money(i64::MAX, "USD"), "USD 9,223,372,036,854.78");
        assert_eq!(money(i64::MIN, "USD"), "USD -9,223,372,036,854.78");
        assert_eq!(money(1_000_000_000_000_000, "USD"), "USD 1,000,000,000.00");
    }

    #[test]
    fn money_fixed_maps_known_symbols_and_keeps_unknown_codes() {
        assert_eq!(money_fixed(1_250_000_000, "USD"), "$1,250.00");
        assert_eq!(money_fixed(1_250_000_000, "usd"), "$1,250.00");
        assert_eq!(money_fixed(1_250_000_000, "EUR"), "\u{20ac}1,250.00");
        assert_eq!(money_fixed(1_250_000_000, "GBP"), "\u{a3}1,250.00");
        assert_eq!(money_fixed(1_250_000_000, "JPY"), "JPY 1,250.00");
        assert_eq!(money_fixed(-4_000_000, "USD"), "$-4.00");
    }

    #[test]
    fn ecpm_is_the_fixed_context_renderer() {
        assert_eq!(ecpm(4_000_000, "USD"), "$4.00");
        assert_eq!(ecpm(3_200_000, "USD"), "$3.20");
        assert_eq!(ecpm(0, "USD"), "$0.00");
        assert_eq!(ecpm(4_000_000, "JPY"), "JPY 4.00");
    }

    #[test]
    fn micros_exact_groups_integers() {
        assert_eq!(micros_exact(0), "0");
        assert_eq!(micros_exact(999), "999");
        assert_eq!(micros_exact(1_000), "1,000");
        assert_eq!(micros_exact(1_250_000_000), "1,250,000,000");
        assert_eq!(micros_exact(-1_250_000_000), "-1,250,000,000");
        assert_eq!(micros_exact(i64::MAX), "9,223,372,036,854,775,807");
        assert_eq!(micros_exact(i64::MIN), "-9,223,372,036,854,775,808");
    }

    #[test]
    fn utc_datetime_and_date_short_render_complete_utc_dates() {
        assert_eq!(utc_datetime(0), "01 Jan 1970, 00:00 UTC");
        assert_eq!(utc_datetime(OCT_14_2026_0810), "14 Oct 2026, 08:10 UTC");
        assert_eq!(date_short(OCT_14_2026_0810), "14 Oct 2026");
    }

    #[test]
    fn utc_datetime_handles_leap_days_pre_epoch_and_sub_minute_ms() {
        // 2024-02-29T23:59:00Z
        assert_eq!(utc_datetime(1_709_251_140_000), "29 Feb 2024, 23:59 UTC");
        // One minute before the epoch.
        assert_eq!(utc_datetime(-60_000), "31 Dec 1969, 23:59 UTC");
        // Milliseconds inside a minute do not leak into the render.
        assert_eq!(
            utc_datetime(OCT_14_2026_0810 + 59_999),
            "14 Oct 2026, 08:10 UTC"
        );
        assert_eq!(
            utc_datetime(OCT_14_2026_0810 + 60_000),
            "14 Oct 2026, 08:11 UTC"
        );
        assert_eq!(date_short(1_709_251_140_000), "29 Feb 2024");
        assert_eq!(date_short(-60_000), "31 Dec 1969");
    }

    #[test]
    fn percent_formats_ratios_with_one_decimal() {
        assert_eq!(percent(0.123), "12.3%");
        assert_eq!(percent(0.0), "0.0%");
        assert_eq!(percent(-0.0), "0.0%");
        assert_eq!(percent(1.0), "100.0%");
        assert_eq!(percent(0.25), "25.0%");
        assert_eq!(percent(-0.04), "-4.0%");
        assert_eq!(
            percent(f64::NAN),
            "Not available (value is not a finite number)"
        );
        assert_eq!(
            percent(f64::INFINITY),
            "Not available (value is not a finite number)"
        );
        assert_eq!(percent(f64::MAX), "Not available (value is out of range)");
    }

    #[test]
    fn delta_is_signed_with_a_reason_for_a_zero_baseline() {
        assert_eq!(delta(1.0, 1.03), "+3.0%");
        assert_eq!(delta(1.0, 0.875), "-12.5%");
        assert_eq!(delta(100.0, 100.0), "0.0%");
        assert_eq!(delta(1.0, 0.0), "-100.0%");
        assert_eq!(delta(0.0, 5.0), "Not available (zero baseline)");
        assert_eq!(delta(-0.0, 5.0), "Not available (zero baseline)");
        assert_eq!(
            delta(f64::NAN, 1.0),
            "Not available (baseline or result is not a finite number)"
        );
    }

    #[test]
    fn or_not_available_keeps_the_reason() {
        assert_eq!(
            or_not_available("report is provisional"),
            "Not available (report is provisional)"
        );
        assert_eq!(or_not_available(""), "Not available");
        assert_eq!(or_not_available("   "), "Not available");
        assert_eq!(or_not_available("  no data  "), "Not available (no data)");
    }

    #[test]
    fn csv_field_quotes_rfc4180() {
        assert_eq!(csv_field("plain"), "plain");
        assert_eq!(csv_field("a,b"), "\"a,b\"");
        assert_eq!(csv_field("say \"hi\""), "\"say \"\"hi\"\"\"");
        assert_eq!(csv_field("line\nbreak"), "\"line\nbreak\"");
        assert_eq!(csv_field("cr\rreturn"), "\"cr\rreturn\"");
        assert_eq!(csv_field(""), "");
    }
}
