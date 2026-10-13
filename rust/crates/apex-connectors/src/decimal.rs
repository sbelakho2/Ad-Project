//! Decimal -> integer micros conversion for platform payloads (never floats).

use apex_core::PlatformError;

/// Parses a non-negative decimal string into micros (1e-6 units).
///
/// Fail-closed contract (pinned by tests):
/// - rejects empty/whitespace, signs, exponent notation, non-digit characters;
/// - the fractional part may have at most 6 digits (more => error, never silent
///   truncation, because silently dropping precision corrupts money);
/// - "2." (dot without fraction) is rejected; "2" is fine.
pub fn parse_decimal_micros(input: &str) -> Result<i64, PlatformError> {
    let t = input.trim();
    let reject = |why: &str| {
        Err(PlatformError::Validation(format!(
            "decimal '{input}': {why} (expected non-negative number with <=6 decimal places)"
        )))
    };
    if t.is_empty() {
        return reject("empty");
    }
    if t.contains(['e', 'E']) {
        return reject("exponent notation not accepted");
    }
    if t.starts_with(['+', '-']) {
        return reject("sign not accepted");
    }

    let (int_part, frac_part) = match t.split_once('.') {
        Some((int_part, frac)) => {
            if frac.is_empty() {
                return reject("dot without fractional digits");
            }
            (int_part, Some(frac))
        }
        None => (t, None),
    };

    if int_part.is_empty() && frac_part.is_none() {
        return reject("no digits");
    }
    if !int_part.bytes().all(|b| b.is_ascii_digit()) {
        return reject("integer part is not all digits");
    }
    let frac = frac_part.unwrap_or("");
    if !frac.bytes().all(|b| b.is_ascii_digit()) {
        return reject("fractional part is not all digits");
    }
    if frac.len() > 6 {
        return reject("more than 6 fractional digits");
    }

    let int_value: i64 = if int_part.is_empty() {
        0
    } else {
        int_part.parse().map_err(|_| {
            PlatformError::Validation(format!("decimal '{input}': integer part overflows i64"))
        })?
    };
    let mut scaled_frac: i64 = if frac.is_empty() {
        0
    } else {
        frac.parse().map_err(|_| {
            PlatformError::Validation(format!("decimal '{input}': fractional part invalid"))
        })?
    };
    for _ in frac.len()..6 {
        scaled_frac *= 10;
    }

    int_value
        .checked_mul(1_000_000)
        .and_then(|v| v.checked_add(scaled_frac))
        .ok_or_else(|| {
            PlatformError::Validation(format!("decimal '{input}': overflows i64 micros"))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_micros_table() {
        assert_eq!(parse_decimal_micros("2").unwrap(), 2_000_000);
        assert_eq!(parse_decimal_micros("2.35").unwrap(), 2_350_000);
        assert_eq!(parse_decimal_micros("2.345").unwrap(), 2_345_000);
        assert_eq!(parse_decimal_micros("2.3451").unwrap(), 2_345_100);
        assert_eq!(parse_decimal_micros("0.000001").unwrap(), 1);
        assert_eq!(parse_decimal_micros(" 3.5 ").unwrap(), 3_500_000);
        assert_eq!(parse_decimal_micros("0").unwrap(), 0);
        assert_eq!(parse_decimal_micros(".5").unwrap(), 500_000);
    }

    #[test]
    fn fails_closed_on_precision_loss_and_garbage() {
        for bad in [
            "2.34567",   // 5 frac ok; this is 5 -> ok? no: 5 digits ok. Use 7 below.
            "2.3456789", // 7 digits: reject
            "-1",
            "+1",
            "1e3",
            "1E-3",
            "",
            "   ",
            "2.",
            "..",
            "2,5",
            "abc",
            "1 000",
            "NaN",
            "inf",
        ] {
            let result = parse_decimal_micros(bad);
            if bad == "2.34567" {
                assert_eq!(
                    result.unwrap(),
                    2_345_670,
                    "5 fractional digits are allowed"
                );
            } else {
                assert!(result.is_err(), "must reject {bad:?}");
            }
        }
    }

    #[test]
    fn overflow_is_rejected() {
        assert!(parse_decimal_micros("9223372036855").is_err());
        assert!(parse_decimal_micros(&"9".repeat(30)).is_err());
    }
}
