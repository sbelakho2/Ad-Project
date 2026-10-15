//! Helpers for the `apex-runner` command line: argument and environment
//! parsing, the signing-key seed decoder and the month math. They live here,
//! not in the binary, so they are unit tested and stay covered.

/// Read a required environment value, trimming blanks.
pub fn required(env: impl Fn(&str) -> Option<String>, name: &str) -> Result<String, String> {
    env(name)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| format!("{name} is required"))
}

/// The value of `--flag value` in `args`, if present.
pub fn flag(args: &[String], name: &str) -> Option<String> {
    let index = args.iter().position(|arg| arg == name)?;
    args.get(index + 1).cloned()
}

/// Decode a 64-character hex ed25519 seed.
pub fn parse_seed(hex: &str) -> Result<[u8; 32], String> {
    let hex = hex.trim();
    if hex.len() != 64 {
        return Err("the signing key must be 64 hex characters".into());
    }
    let bytes = hex.as_bytes();
    let mut seed = [0u8; 32];
    for (index, byte) in seed.iter_mut().enumerate() {
        let high = nibble(bytes[2 * index]).ok_or("the signing key is not hex")?;
        let low = nibble(bytes[2 * index + 1]).ok_or("the signing key is not hex")?;
        *byte = (high << 4) | low;
    }
    Ok(seed)
}

fn nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

/// The calendar month just closed, as `(YYYY-MM, YYYY-MM-01, day count)`.
pub fn previous_month(now_unix_ms: i64) -> Result<(String, String, u32), String> {
    let today = now_unix_ms.div_euclid(86_400_000);
    let (year, month) =
        parse_year_month(&crate::calendar::format_day(today)).ok_or("system clock out of range")?;
    let (prev_year, prev_month) = if month == 1 {
        (year - 1, 12)
    } else {
        (year, month - 1)
    };
    let start = format!("{prev_year:04}-{prev_month:02}-01");
    let start_day = crate::calendar::day_number(&start).ok_or("bad month start")?;
    let this_start = crate::calendar::day_number(&format!("{year:04}-{month:02}-01"))
        .ok_or("bad month start")?;
    let days = u32::try_from(this_start - start_day).map_err(|_| "bad month length")?;
    Ok((format!("{prev_year:04}-{prev_month:02}"), start, days))
}

/// The `(year, month)` of a `YYYY-MM-DD` day.
pub fn parse_year_month(text: &str) -> Option<(i64, u32)> {
    let mut parts = text.split('-');
    let year = parts.next()?.parse().ok()?;
    let month = parts.next()?.parse().ok()?;
    Some((year, month))
}

/// Unix milliseconds from the system clock.
pub fn now_unix_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn required_trims_and_refuses_blanks() {
        let env = |name: &str| match name {
            "SET" => Some("value".to_string()),
            "BLANK" => Some("   ".to_string()),
            _ => None,
        };
        assert_eq!(required(env, "SET").unwrap(), "value");
        assert!(required(env, "BLANK").is_err());
        assert!(required(env, "MISSING").is_err());
    }

    #[test]
    fn flag_reads_the_following_argument() {
        let args = vec![
            "monthly".to_string(),
            "--month".to_string(),
            "2026-10".to_string(),
        ];
        assert_eq!(flag(&args, "--month").as_deref(), Some("2026-10"));
        assert_eq!(flag(&args, "--start"), None);
        assert_eq!(flag(&args, "--month"), Some("2026-10".to_string()));
    }

    #[test]
    fn parse_seed_decodes_hex_and_refuses_bad_input() {
        let seed = parse_seed(&"0a".repeat(32)).expect("seed");
        assert_eq!(seed[0], 0x0a);
        assert_eq!(seed[31], 0x0a);
        assert!(parse_seed("short").is_err());
        assert!(parse_seed(&"zz".repeat(32)).is_err());
    }

    #[test]
    fn previous_month_handles_the_year_boundary() {
        // 2026-01-15 -> 2025-12 (31 days).
        let day = crate::calendar::day_number("2026-01-15").unwrap();
        let (month, start, days) = previous_month(day * 86_400_000).unwrap();
        assert_eq!(month, "2025-12");
        assert_eq!(start, "2025-12-01");
        assert_eq!(days, 31);
        // 2026-03-05 -> 2026-02 (28 days in a non leap year).
        let day = crate::calendar::day_number("2026-03-05").unwrap();
        let (month, _, days) = previous_month(day * 86_400_000).unwrap();
        assert_eq!(month, "2026-02");
        assert_eq!(days, 28);
    }

    #[test]
    fn parse_year_month_reads_the_prefix() {
        assert_eq!(parse_year_month("2026-10-01"), Some((2026, 10)));
        assert_eq!(parse_year_month("bad"), None);
    }

    #[test]
    fn now_is_after_the_product_epoch() {
        assert!(now_unix_ms() > 1_700_000_000_000);
    }
}
