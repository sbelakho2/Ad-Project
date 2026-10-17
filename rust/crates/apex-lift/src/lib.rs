//! The Apex Lift Engine: revenue-per-user log-ratio measurement, the six keep
//! conditions, the guardrails, and the 12-month lift billing, implemented
//! exactly as the design spec (Oct 14, 2026) defines them.
//!
//! Module map to the spec:
//!
//! - [`stats`]: the estimator (mean daily log ratio, one-sided 90% and 99%
//!   lower bounds with t quantiles), the block bootstrap cross-check and the
//!   duration planner.
//! - [`window`]: burn-in, day finality (two pulls 24 hours apart within
//!   0.5%), whole-week windows, incident exclusion and per-arm daily values.
//! - [`guardrails`]: the daily guardrails that can only stop a test.
//! - [`decision`]: the six keep conditions, the stricter interim look and the
//!   one pre-registered extension; every other path rolls back or voids.
//! - [`billing`]: the monthly fee formulas over active kept changes, the
//!   12-month term, publisher-override pro-rating and the month-6 re-check
//!   rule that can only lower the billed lift.
//! - [`switchback`]: the randomized two-day block fallback and its billing
//!   floor.
//! - [`priors`]: the hierarchical prior, Thompson-sampling picker and its
//!   score formula.
//! - [`records`]: the six signed record types, in the order the spec lists.
//!
//! Invariants enforced here: bill the lower bound and never the point
//! estimate, never bill without a full final window, fail closed on bad data,
//! and let the user-experience guardrails outrank revenue.

/// The analysis-code version that runs the decision. It is pinned into every
/// pre-registration so a publisher can reproduce the exact code that judged
/// the test.
pub const ENGINE_VERSION: &str = env!("CARGO_PKG_VERSION");

pub mod billing;
pub mod calibration;
pub mod candidates;
pub mod decision;
pub mod emit;
pub mod error;
pub mod guardrails;
pub mod policy;
pub mod postbacks;
pub mod priors;
pub mod recheck;
pub mod records;
pub mod rerun;
pub mod rng;
pub mod run;
pub mod scoreboard;
pub mod settings;
pub mod stats;
pub mod switchback;
pub mod window;

pub use decision::{decide, Decision, DecisionInput, GuardrailThresholds, PreRegistration};

/// Parse a plain decimal currency amount into micros exactly, the way the
/// platform exports render money (`12.50`, `0.006`).
pub(crate) fn parse_revenue_micros(text: &str) -> Option<Micros> {
    let text = text.trim();
    if text.is_empty() || text.starts_with('-') {
        return None;
    }
    let (whole, fraction) = match text.split_once('.') {
        Some((whole, fraction)) => (whole, fraction),
        None => (text, ""),
    };
    if whole.is_empty() && fraction.is_empty() {
        return None;
    }
    if !whole.chars().all(|c| c.is_ascii_digit()) || !fraction.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    if fraction.len() > 6 {
        return None;
    }
    let whole: Micros = whole.parse().ok()?;
    let mut padded = fraction.to_string();
    while padded.len() < 6 {
        padded.push('0');
    }
    let fraction: Micros = padded.parse().ok()?;
    whole.checked_mul(1_000_000)?.checked_add(fraction)
}

use apex_core::Micros;
pub use error::LiftError;
pub use stats::{estimate, plan_duration, Estimate, Plan, Side};
pub use window::{AnalysisWindow, ArmDay, WindowProblem};

#[cfg(test)]
mod parser_tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::parse_revenue_micros;

    #[test]
    fn revenue_parsing_is_exact_and_strict() {
        assert_eq!(parse_revenue_micros("12.50"), Some(12_500_000));
        assert_eq!(parse_revenue_micros("0.006"), Some(6_000));
        assert_eq!(parse_revenue_micros("7"), Some(7_000_000));
        assert_eq!(parse_revenue_micros(" 1.5 "), Some(1_500_000));
        assert_eq!(parse_revenue_micros(""), None);
        assert_eq!(parse_revenue_micros("-1.00"), None);
        assert_eq!(parse_revenue_micros("abc"), None);
        assert_eq!(parse_revenue_micros("1.2345678"), None);
        assert_eq!(parse_revenue_micros("1..2"), None);
        assert_eq!(
            parse_revenue_micros("."),
            None,
            "a bare separator has no digits at all"
        );
        assert_eq!(
            parse_revenue_micros(".5"),
            None,
            "exports never render a bare fraction"
        );
        assert_eq!(
            parse_revenue_micros("92233720368547758.07"),
            None,
            "overflow refuses"
        );
    }
}
