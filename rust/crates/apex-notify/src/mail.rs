//! RFC 5322 composition for the email sink.
//!
//! The conventions mirror the sales engine's composer
//! (`apex-sales/src/compose.rs`): `\r\n` line endings, a UTC `Date:` built
//! from Howard Hinnant's civil-date algorithm (no time-zone database), and a
//! stable `Message-ID` derived from the message identity. The body is a
//! `multipart/alternative` pair so clients pick the frozen HTML part and
//! everything else gets the matching plain-text part.

/// Seconds in a day.
const DAY: i64 = 86_400;

/// `(year, month, day)` for a day count since 1970-01-01 (proleptic
/// Gregorian; Howard Hinnant's `civil_from_days`, exact for all `i64`).
fn civil_from_days(z: i64) -> (i64, i64, i64) {
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
fn weekday(days: i64) -> i64 {
    (days + 4).rem_euclid(7)
}

/// RFC 5322 `Date:` value in UTC, e.g. `Fri, 16 Oct 2026 10:13:00 +0000`.
pub(crate) fn rfc5322_date(unix_secs: i64) -> String {
    const DAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let days = unix_secs.div_euclid(DAY);
    let secs = unix_secs.rem_euclid(DAY);
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

/// FNV-1a over the message identity: stable across runs, short, and never
/// the raw identity itself.
fn fnv1a64(data: &str) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in data.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// Normalize any mixture of line endings to CRLF.
fn crlf(text: &str) -> String {
    text.replace("\r\n", "\n")
        .replace('\r', "\n")
        .replace('\n', "\r\n")
}

/// Compose the RFC 5322 message the email sink pipes to `sendmail`.
///
/// `from_header` is the validated `APEX_NOTIFY_FROM` value, `domain` the
/// domain part of its address (the `Message-ID` right-hand side), and
/// `dedup_key` the message identity the `Message-ID` and MIME boundary
/// derive from. Header values (`from_header`, `to`, `subject`) are refused
/// before composition when they carry control characters, so they can never
/// smuggle a header line.
#[allow(clippy::too_many_arguments)]
pub(crate) fn compose(
    from_header: &str,
    to: &str,
    subject: &str,
    text: &str,
    html: &str,
    dedup_key: &str,
    domain: &str,
    list_unsubscribe: Option<&str>,
    now_unix_secs: i64,
) -> String {
    let hash = fnv1a64(dedup_key);
    let boundary = format!("apex-notify-{hash:016x}");
    let mut headers = vec![
        format!("From: {from_header}"),
        format!("To: {to}"),
        format!("Subject: {subject}"),
        format!("Date: {}", rfc5322_date(now_unix_secs)),
        format!("Message-ID: <apex.{hash:016x}@{domain}>"),
        "MIME-Version: 1.0".to_string(),
        format!("Content-Type: multipart/alternative; boundary=\"{boundary}\""),
    ];
    if let Some(url) = list_unsubscribe.map(str::trim).filter(|url| {
        (url.starts_with("https://") || url.starts_with("http://"))
            && !url.bytes().any(|byte| {
                byte.is_ascii_whitespace()
                    || byte == b'<'
                    || byte == b'>'
                    || byte == b'\r'
                    || byte == b'\n'
            })
    }) {
        headers.push(format!("List-Unsubscribe: <{url}>"));
        headers.push("List-Unsubscribe-Post: List-Unsubscribe=One-Click".to_string());
    }
    let mut raw = headers.join("\r\n");
    raw.push_str("\r\n\r\n");
    for (content_type, part) in [("text/plain", text), ("text/html", html)] {
        raw.push_str(&format!(
            "--{boundary}\r\nContent-Type: {content_type}; charset=utf-8\r\nContent-Transfer-Encoding: 8bit\r\n\r\n{}\r\n",
            crlf(part).trim_end_matches("\r\n")
        ));
    }
    raw.push_str(&format!("--{boundary}--\r\n"));
    raw
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn a_one_click_unsubscribe_url_becomes_both_rfc8058_headers() {
        let raw = compose(
            "Apex <hello@example.com>",
            "ops@example.com",
            "Subject",
            "text",
            "<p>html</p>",
            "watch_alert:pub:hash",
            "example.com",
            Some("https://console.example/watch/cancel?token=abc"),
            0,
        );
        assert!(raw.contains(
            "\r\nList-Unsubscribe: <https://console.example/watch/cancel?token=abc>\r\n"
        ));
        assert!(raw.contains("\r\nList-Unsubscribe-Post: List-Unsubscribe=One-Click\r\n"));
        // A header-like URL is omitted, never injected.
        let hostile = compose(
            "Apex <hello@example.com>",
            "ops@example.com",
            "Subject",
            "text",
            "<p>html</p>",
            "k",
            "example.com",
            Some("https://example.com/\r\nBcc: attacker@example.com"),
            0,
        );
        assert!(!hostile.contains("Bcc:"));
        assert!(!hostile.contains("List-Unsubscribe"));
    }

    #[test]
    fn rfc5322_dates_match_the_sales_engine() {
        assert_eq!(rfc5322_date(0), "Thu, 01 Jan 1970 00:00:00 +0000");
        let t = 20_742 * DAY + 10 * 3600 + 13 * 60;
        assert_eq!(rfc5322_date(t), "Fri, 16 Oct 2026 10:13:00 +0000");
        // Pre-1970 clocks stay in range instead of panicking.
        assert_eq!(rfc5322_date(-1), "Wed, 31 Dec 1969 23:59:59 +0000");
    }

    #[test]
    fn the_composed_message_is_a_crlf_multipart_alternative() {
        let raw = compose(
            "Apex <hello@apex-transactional.example>",
            "ops@example.com",
            "Subject here",
            "text body\n",
            "<p>html body</p>\n",
            "guardrail_stop:exp_1",
            "apex-transactional.example",
            None,
            0,
        );
        assert!(raw.starts_with(
            "From: Apex <hello@apex-transactional.example>\r\nTo: ops@example.com\r\nSubject: Subject here\r\n"
        ));
        assert!(raw.contains("\r\nDate: Thu, 01 Jan 1970 00:00:00 +0000\r\n"));
        assert!(raw.contains("\r\nMessage-ID: <apex."));
        assert!(raw.contains("@apex-transactional.example>\r\n"));
        assert!(raw.contains("\r\nMIME-Version: 1.0\r\n"));
        assert!(raw.contains("\r\nContent-Type: multipart/alternative; boundary=\"apex-notify-"));
        assert!(raw.contains(
            "Content-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: 8bit\r\n\r\ntext body\r\n"
        ));
        assert!(raw.contains(
            "Content-Type: text/html; charset=utf-8\r\nContent-Transfer-Encoding: 8bit\r\n\r\n<p>html body</p>\r\n"
        ));
        assert!(raw.ends_with("--\r\n"));
        assert!(!raw.replace("\r\n", "").contains('\n'), "CRLF only");
        // The identity is stable for the same dedup key and distinct across keys.
        let again = compose(
            "Apex <hello@apex-transactional.example>",
            "ops@example.com",
            "Subject here",
            "text body\n",
            "<p>html body</p>\n",
            "guardrail_stop:exp_1",
            "apex-transactional.example",
            None,
            1,
        );
        let message_id = |raw: &str| {
            raw.lines()
                .find(|line| line.starts_with("Message-ID:"))
                .map(str::to_string)
        };
        assert_eq!(message_id(&raw), message_id(&again));
        let other = compose(
            "Apex <hello@apex-transactional.example>",
            "ops@example.com",
            "Subject here",
            "text body\n",
            "<p>html body</p>\n",
            "guardrail_stop:exp_2",
            "apex-transactional.example",
            None,
            0,
        );
        assert_ne!(message_id(&raw), message_id(&other));
    }
}
