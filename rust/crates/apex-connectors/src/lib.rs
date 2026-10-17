//! apex-connectors — management-API clients for AppLovin MAX, LevelPlay and AdMob.
//!
//! Behavioral reference: `backend/src/services/platforms/*.ts` (ported 1:1; the
//! differences are intentional and listed in the crate docs):
//! - money is integer micros everywhere (`apex_core`), platform decimal payloads go
//!   through `decimal::parse_decimal_micros` which fails closed on >6 fractional
//!   digits instead of silently keeping float error;
//! - the transport is a trait so the whole retry/error matrix is unit-testable
//!   without sockets; `ReqwestTransport` is the production implementation.

#![forbid(unsafe_code)]
#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::too_many_lines
    )
)]

pub mod admob;
pub mod app_ads;
pub mod decimal;
pub mod levelplay;
pub mod max;
pub mod snapshot;
pub mod transport;

#[cfg(test)]
mod tests;

pub use admob::AdmobClient;
pub use decimal::parse_decimal_micros;
pub use levelplay::LevelPlayClient;
pub use max::MaxClient;
pub use transport::{
    HttpBody, HttpMethod, HttpRequest, HttpResponse, MeteredTransport, RecordingSleeper,
    ReqwestTransport, RetryTransport, Sleeper, TokioSleeper, Transport, TransportError,
    FORM_CONTENT_TYPE,
};

/// Percent string/number (e.g. "7.5" meaning 7.5%) -> basis points (750).
/// Integer math only; `None` when the value cannot be represented exactly enough.
pub fn percent_to_basis_points(percent: &str) -> Option<i64> {
    let micros = parse_decimal_micros(percent).ok()?; // 1e-6 percent units
    Some(micros / 10_000) // 1 bp = 0.01 percent = 10_000 micro-percent
}

/// Fraction string/number (e.g. "0.98") -> basis points (9800).
pub fn fraction_to_basis_points(fraction: &str) -> Option<i64> {
    let micros = parse_decimal_micros(fraction).ok()?;
    Some(micros / 100)
}

/// Minimal percent-encoding for path segments and query values (RFC 3986
/// unreserved characters kept), so a hostile key cannot forge URL structure.
pub fn encode_query_component(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for b in raw.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod util_tests {
    use super::*;

    #[test]
    fn percent_to_bp() {
        assert_eq!(percent_to_basis_points("7.5"), Some(750));
        assert_eq!(percent_to_basis_points("100"), Some(10_000));
        assert_eq!(percent_to_basis_points("-2.5"), None);
        assert_eq!(percent_to_basis_points("x"), None);
    }

    #[test]
    fn fraction_to_bp() {
        assert_eq!(fraction_to_basis_points("0.98"), Some(9_800));
        assert_eq!(fraction_to_basis_points("1"), Some(10_000));
        assert_eq!(fraction_to_basis_points("bad"), None);
    }

    #[test]
    fn query_encoding() {
        assert_eq!(encode_query_component("abc-._~"), "abc-._~");
        assert_eq!(encode_query_component("a b/c?"), "a%20b%2Fc%3F");
        assert_eq!(encode_query_component("tøken"), "t%C3%B8ken");
    }
}
