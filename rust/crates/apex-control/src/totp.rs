//! RFC 6238 time-based one-time passwords over HMAC-SHA1.
//!
//! The operator's second factor is the standard authenticator-code step: a
//! 30-second time step, six digits, HMAC-SHA1 (the algorithm every TOTP app
//! ships) and a drift window of one step in both directions, so a clock a few
//! seconds off does not lock an operator out. The implementation is
//! hand-rolled on `hmac` + `sha1` so the exact truncation rule is tested
//! against the RFC 6238 vectors rather than trusted to a wrapper.

use hmac::{Hmac, Mac};
use sha1::Sha1;

use crate::signing::constant_time_eq;

/// The TOTP time step in seconds (RFC 6238 recommendation, and what every
/// authenticator app uses).
pub const STEP_SECONDS: u64 = 30;
/// Count of code digits.
pub const DIGITS: u32 = 6;
/// Accepted drift in time steps on either side of the current step.
pub const DRIFT_STEPS: u64 = 1;

type HmacSha1 = Hmac<Sha1>;

/// The six-digit code for `secret` at Unix time `unix_seconds`.
///
/// The counter is `floor(unix_seconds / 30)` as a big-endian 64-bit integer;
/// the HMAC-SHA1 digest is truncated per RFC 4226 section 5.4.
pub fn code_at(secret: &[u8], unix_seconds: i64) -> String {
    let counter = unix_seconds.div_euclid(STEP_SECONDS as i64).max(0) as u64;
    hotp(secret, counter)
}

/// Verify `code` against the current step and one step of drift on either
/// side, returning the matched step so the caller can refuse a replay.
/// Comparison is constant-time per candidate step.
pub fn verify_step(secret: &[u8], code: &str, unix_seconds: i64) -> Option<u64> {
    let code = code.trim();
    if code.len() != DIGITS as usize || !code.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let now_step = unix_seconds.div_euclid(STEP_SECONDS as i64).max(0) as u64;
    let mut matched = None;
    for offset in 0..=(2 * DRIFT_STEPS) {
        let step = now_step.saturating_add(DRIFT_STEPS).saturating_sub(offset);
        let candidate = hotp(secret, step);
        // Fold every candidate so the loop time does not depend on the match.
        let equal = constant_time_eq(candidate.as_bytes(), code.as_bytes());
        if equal && matched.is_none() {
            matched = Some(step);
        }
    }
    matched
}

/// Verify `code` without replay protection (kept for callers that do not
/// track the operator's last accepted step).
pub fn verify(secret: &[u8], code: &str, unix_seconds: i64) -> bool {
    verify_step(secret, code, unix_seconds).is_some()
}

/// The HOTP value for one counter value, truncated to [`DIGITS`] digits.
fn hotp(secret: &[u8], counter: u64) -> String {
    let mut mac = match HmacSha1::new_from_slice(secret) {
        Ok(mac) => mac,
        // HMAC accepts any key length; this branch is unreachable in practice
        // and returns an impossible code rather than panicking.
        Err(_) => return "000000".to_string(),
    };
    mac.update(&counter.to_be_bytes());
    let digest = mac.finalize().into_bytes();
    let offset = (digest[19] & 0x0f) as usize;
    let binary = (u32::from(digest[offset]) & 0x7f) << 24
        | u32::from(digest[offset + 1]) << 16
        | u32::from(digest[offset + 2]) << 8
        | u32::from(digest[offset + 3]);
    format!("{:06}", binary % 10u32.pow(DIGITS))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The RFC 6238 SHA-1 test secret `12345678901234567890`.
    const SECRET: &[u8] = b"12345678901234567890";

    #[test]
    fn matches_the_rfc6238_sha1_vectors() {
        // RFC 6238 appendix B, truncated to the six digits the UI accepts.
        for (time, expected) in [
            (59i64, "287082"),
            (1_111_111_109, "081804"),
            (1_111_111_111, "050471"),
            (1_234_567_890, "005924"),
            (2_000_000_000, "279037"),
            (20_000_000_000, "353130"),
        ] {
            assert_eq!(code_at(SECRET, time), expected, "T={time}");
        }
    }

    #[test]
    fn verify_accepts_the_current_step_and_one_step_of_drift() {
        let at = 1_111_111_111i64;
        let current = code_at(SECRET, at);
        assert!(verify(SECRET, &current, at));
        // Codes from the previous and next step are accepted.
        assert!(verify(SECRET, &code_at(SECRET, at - 30), at));
        assert!(verify(SECRET, &code_at(SECRET, at + 30), at));
        // A code two steps away is not.
        assert!(!verify(SECRET, &code_at(SECRET, at - 60), at));
        assert!(!verify(SECRET, &code_at(SECRET, at + 60), at));
    }

    #[test]
    fn verify_refuses_wrong_shapes_and_wrong_secrets() {
        let at = 1_700_000_000i64;
        let code = code_at(SECRET, at);
        assert!(verify(SECRET, &code, at));
        assert!(!verify(b"another secret value", &code, at));
        assert!(!verify(SECRET, "12345", at), "five digits");
        assert!(!verify(SECRET, "1234567", at), "seven digits");
        assert!(!verify(SECRET, "abcdef", at), "non digits");
        assert!(!verify(SECRET, "", at));
        // An all-digit wrong code is refused (this value is not the code).
        assert!(!verify(SECRET, "000000", at) || code == "000000");
    }

    #[test]
    fn codes_roll_over_on_the_step_boundary() {
        let before = code_at(SECRET, 29);
        let after = code_at(SECRET, 30);
        assert_ne!(before, after);
        assert_eq!(code_at(SECRET, 30), code_at(SECRET, 59));
        assert_eq!(code_at(SECRET, 0), code_at(SECRET, 29));
    }

    #[test]
    fn every_code_is_six_ascii_digits() {
        for step in 0..64u64 {
            let code = code_at(SECRET, (step * 30) as i64);
            assert_eq!(code.len(), 6);
            assert!(code.bytes().all(|byte| byte.is_ascii_digit()));
        }
    }
}
