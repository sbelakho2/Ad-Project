//! Money and date formatting for transactional messages.
//!
//! Section 06 page 92 requires "A 600 px email body, safe HTML tables,
//! system-font fallback and a matching plain-text part" with "Amounts and
//! status come from durable billing/payment records." The formatter here is
//! deliberately local and small: `<CODE> 300.00` for integer micros in the
//! amount's own ISO-4217 currency and `20 October 2026` for `YYYY-MM-DD`
//! days, independent of `apex-ui`.
//!
//! Currency discipline: the amount's currency code is always the one the
//! durable event carried. A missing or malformed code renders
//! [`CURRENCY_UNAVAILABLE`] rather than guessing USD from an unlabelled
//! integer.

/// Why a value could not be formatted.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FormatError {
    /// A money amount was negative where only positive amounts are rendered.
    #[error("amount {micros} micros is negative; notification amounts are positive")]
    NegativeAmount {
        /// The refused amount.
        micros: i64,
    },
    /// A date was not a `YYYY-MM-DD` calendar day.
    #[error("'{value}' is not a YYYY-MM-DD day")]
    InvalidDay {
        /// The refused value.
        value: String,
    },
    /// The amount's currency code was absent or not an ISO-4217 shape.
    #[error("currency '{currency}' is not available; the amount is not rendered as USD")]
    CurrencyUnavailable {
        /// The refused code.
        currency: String,
    },
}

/// The not-available rendering for an amount whose currency is absent: the
/// design's rule is "Currency unavailable", never a guessed USD. The raw
/// micros value belongs to details views only and is not rendered here.
pub const CURRENCY_UNAVAILABLE: &str = "Not available (currency unavailable)";

/// Month names for the `20 October 2026` rendering.
const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

/// Format integer micros as `USD 300.00` (or `USD 1,250.00`): two decimals,
/// thousands grouped. The workspace's billing currency is a fixed constant,
/// so this stays for the paths whose records are denominated in USD.
pub fn format_usd(micros: i64) -> Result<String, FormatError> {
    format_money("USD", micros)
}

/// Format integer micros in `currency` as `EUR 3.00`: the event's own
/// currency code, two decimals, thousands grouped. The code must be three
/// ASCII uppercase letters (the ISO-4217 shape the provider events use); an
/// absent or malformed code is a typed refusal, never a guessed USD.
///
/// The rounding mirrors `apex-ui::format::money` exactly (i128 math, round
/// half away from zero, the cents carry folded into the integer part), so a
/// non-negative amount in a known currency renders identically on every
/// surface, up to `i64::MAX` micros without overflowing.
pub fn format_money(currency: &str, micros: i64) -> Result<String, FormatError> {
    if micros < 0 {
        return Err(FormatError::NegativeAmount { micros });
    }
    if !is_currency_code(currency) {
        return Err(FormatError::CurrencyUnavailable {
            currency: currency.to_string(),
        });
    }
    Ok(format!("{currency} {}", amount_2dp(micros)))
}

/// Format an amount whose currency may be absent: a missing or malformed
/// code renders [`CURRENCY_UNAVAILABLE`] (with the reason), never a guessed
/// USD. A negative amount is still refused: that is corrupt data, not
/// missing data.
pub fn format_money_or_not_available(
    currency: Option<&str>,
    micros: i64,
) -> Result<String, FormatError> {
    if micros < 0 {
        return Err(FormatError::NegativeAmount { micros });
    }
    match currency {
        Some(code) if is_currency_code(code.trim()) => format_money(code.trim(), micros),
        _ => Ok(CURRENCY_UNAVAILABLE.to_string()),
    }
}

/// Whether `code` has the ISO-4217 shape the provider events carry: exactly
/// three ASCII uppercase letters.
pub fn is_currency_code(code: &str) -> bool {
    code.len() == 3 && code.bytes().all(|byte| byte.is_ascii_uppercase())
}

/// Cents-precision rendering of non-negative micros, the same algorithm as
/// `apex-ui::format::money`: grouped whole units and exactly two decimals,
/// rounded half away from zero. The arithmetic is i128, so the full i64
/// range (up to `i64::MAX` micros) rounds and renders without overflowing;
/// the caller has already refused the negative case.
fn amount_2dp(micros: i64) -> String {
    let value = i128::from(micros);
    let mut units = value / 1_000_000;
    let remainder = value % 1_000_000;
    let mut cents = (remainder + 5_000) / 10_000;
    if cents == 100 {
        // A remainder that rounds up to a whole cent carries into the units.
        cents = 0;
        units += 1;
    }
    format!("{}.{:02}", group_digits(units), cents)
}

/// `1250` becomes `1,250`; `300` stays `300`. The value is non-negative by
/// construction (`amount_2dp` runs after the negative refusal).
fn group_digits(value: i128) -> String {
    let digits = value.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

/// Format a `YYYY-MM-DD` day as `20 October 2026`.
pub fn format_day(day: &str) -> Result<String, FormatError> {
    let invalid = || FormatError::InvalidDay {
        value: day.to_string(),
    };
    let mut parts = day.split('-');
    let year: i64 = parts
        .next()
        .ok_or_else(invalid)?
        .parse()
        .map_err(|_| invalid())?;
    let month: usize = parts
        .next()
        .ok_or_else(invalid)?
        .parse()
        .map_err(|_| invalid())?;
    let day_number: u32 = parts
        .next()
        .ok_or_else(invalid)?
        .parse()
        .map_err(|_| invalid())?;
    if parts.next().is_some() || !(1..=12).contains(&month) || !(1..=31).contains(&day_number) {
        return Err(invalid());
    }
    if !(1970..=9999).contains(&year) {
        return Err(invalid());
    }
    Ok(format!("{day_number} {} {year}", MONTHS[month - 1]))
}

/// The month name of a `YYYY-MM` month label, e.g. `2026-10` becomes
/// `October` (used by the fee-preview subject and body).
pub fn month_name(month: &str) -> Result<String, FormatError> {
    let invalid = || FormatError::InvalidDay {
        value: month.to_string(),
    };
    let mut parts = month.split('-');
    let year: i64 = parts
        .next()
        .ok_or_else(invalid)?
        .parse()
        .map_err(|_| invalid())?;
    let month_number: usize = parts
        .next()
        .ok_or_else(invalid)?
        .parse()
        .map_err(|_| invalid())?;
    if parts.next().is_some() || !(1..=12).contains(&month_number) || !(1970..=9999).contains(&year)
    {
        return Err(invalid());
    }
    Ok(MONTHS[month_number - 1].to_string())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn usd_amounts_render_two_decimals_with_grouping() {
        assert_eq!(format_usd(300_000_000).unwrap(), "USD 300.00");
        assert_eq!(format_usd(0).unwrap(), "USD 0.00");
        assert_eq!(format_usd(1_250_000_000).unwrap(), "USD 1,250.00");
        assert_eq!(format_usd(51_500_000_000).unwrap(), "USD 51,500.00");
        assert_eq!(format_usd(10_000).unwrap(), "USD 0.01");
        assert_eq!(format_usd(5_000).unwrap(), "USD 0.01");
        assert_eq!(format_usd(4_999).unwrap(), "USD 0.00");
        assert_eq!(format_usd(999_999_999).unwrap(), "USD 1,000.00");
    }

    #[test]
    fn extreme_amounts_render_without_overflow_or_panic() {
        // The review's parity set: exact strings, in debug too (the release
        // wrap used to render "USD -9,223,372,036,854.-77").
        assert_eq!(format_usd(0).unwrap(), "USD 0.00");
        assert_eq!(format_usd(4_999).unwrap(), "USD 0.00");
        assert_eq!(format_usd(5_000).unwrap(), "USD 0.01");
        assert_eq!(format_usd(999_999).unwrap(), "USD 1.00");
        assert_eq!(
            format_usd(999_999_999_999_999).unwrap(),
            "USD 1,000,000,000.00"
        );
        assert_eq!(format_usd(i64::MAX).unwrap(), "USD 9,223,372,036,854.78");
        assert_eq!(
            format_usd(i64::MAX - 4_999).unwrap(),
            "USD 9,223,372,036,854.77"
        );
    }

    #[test]
    fn money_renders_identically_to_apex_ui_for_the_shared_range() {
        // Same input, same output as the shared renderer for every
        // non-negative amount in a known currency.
        for micros in [
            0i64,
            1,
            4_999,
            5_000,
            9_999,
            10_000,
            999_999,
            300_000_000,
            1_250_000_000,
            999_999_999_999_999,
            i64::MAX - 4_999,
            i64::MAX,
        ] {
            for currency in ["USD", "EUR", "GBP", "SEK"] {
                assert_eq!(
                    format_money(currency, micros).unwrap(),
                    apex_ui::format::money(micros, currency),
                    "{currency} {micros}"
                );
            }
        }
    }

    #[test]
    fn negative_amounts_are_refused_not_rendered() {
        let error = format_usd(-1).expect_err("negative");
        assert!(matches!(error, FormatError::NegativeAmount { .. }));
        assert!(error.to_string().contains("negative"));
    }

    #[test]
    fn money_renders_with_the_events_own_currency() {
        assert_eq!(format_money("EUR", 300_000_000).unwrap(), "EUR 300.00");
        assert_eq!(format_money("GBP", 1_250_000_000).unwrap(), "GBP 1,250.00");
        assert_eq!(format_money("USD", 5_000).unwrap(), "USD 0.01");
        // A malformed code is a typed refusal, never a guessed USD.
        for bad in ["", "usd", "US", "USDD", "U$D", "€"] {
            let error = format_money(bad, 100).expect_err(bad);
            assert!(
                matches!(error, FormatError::CurrencyUnavailable { .. }),
                "{bad}"
            );
        }
        assert!(is_currency_code("JPY"));
        assert!(!is_currency_code("1SD"));
    }

    #[test]
    fn absent_currency_renders_the_not_available_reason() {
        assert_eq!(
            format_money_or_not_available(None, 300_000_000).unwrap(),
            "Not available (currency unavailable)"
        );
        assert_eq!(
            format_money_or_not_available(Some("  "), 300_000_000).unwrap(),
            "Not available (currency unavailable)"
        );
        assert_eq!(
            format_money_or_not_available(Some("eur"), 300_000_000).unwrap(),
            "Not available (currency unavailable)"
        );
        assert_eq!(
            format_money_or_not_available(Some("EUR"), 300_000_000).unwrap(),
            "EUR 300.00"
        );
        // Corrupt data is still refused: missing is not negative.
        let error = format_money_or_not_available(None, -1).expect_err("negative");
        assert!(matches!(error, FormatError::NegativeAmount { .. }));
    }

    #[test]
    fn days_render_with_an_absolute_month_name() {
        assert_eq!(format_day("2026-10-20").unwrap(), "20 October 2026");
        assert_eq!(format_day("2026-01-01").unwrap(), "1 January 2026");
        assert_eq!(format_day("2026-12-31").unwrap(), "31 December 2026");
    }

    #[test]
    fn malformed_days_are_refused() {
        for bad in [
            "2026-13-01",
            "2026-00-01",
            "2026-10-00",
            "2026-10-32",
            "2026-10",
            "2026-10-20-1",
            "not-a-day",
            "1969-12-31",
        ] {
            let error = format_day(bad).expect_err(bad);
            assert!(matches!(error, FormatError::InvalidDay { .. }), "{bad}");
        }
    }

    #[test]
    fn month_names_resolve_for_the_fee_preview() {
        assert_eq!(month_name("2026-10").unwrap(), "October");
        assert_eq!(month_name("2026-01").unwrap(), "January");
        assert!(month_name("2026-13").is_err());
        assert!(month_name("October").is_err());
    }
}
