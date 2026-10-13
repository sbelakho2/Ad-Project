//! The clock boundary.
//!
//! Timestamps enter the API in exactly one place (Stripe signature freshness),
//! so the clock is injected rather than read inline: tests pin a
//! [`FixedClock`] and production uses [`SystemClock`].

use std::time::{SystemTime, UNIX_EPOCH};

/// A source of "now" as Unix seconds.
pub trait Clock: Send + Sync + std::fmt::Debug {
    /// Current Unix time in seconds.
    fn now_unix_s(&self) -> i64;
}

/// Wall-clock time as reported by the operating system.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_unix_s(&self) -> i64 {
        // Before 1970 or beyond i64 seconds is not a representable "now"; the
        // saturating conversion keeps the function total without panicking.
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|elapsed| i64::try_from(elapsed.as_secs()).unwrap_or(i64::MAX))
            .unwrap_or(0)
    }
}

/// A clock frozen at one instant; deterministic tests and fixtures use it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FixedClock {
    now_unix_s: i64,
}

impl FixedClock {
    /// A clock that always reports `now_unix_s`.
    pub const fn new(now_unix_s: i64) -> Self {
        Self { now_unix_s }
    }
}

impl Clock for FixedClock {
    fn now_unix_s(&self) -> i64 {
        self.now_unix_s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_clock_returns_exactly_its_instant() {
        assert_eq!(FixedClock::new(0).now_unix_s(), 0);
        assert_eq!(FixedClock::new(-5).now_unix_s(), -5);
        assert_eq!(FixedClock::new(1_700_000_000).now_unix_s(), 1_700_000_000);
    }

    #[test]
    fn system_clock_is_plausibly_recent() {
        // Sanity bound, not a schedule: anything before 2020 or after 2100
        // means the environment clock is broken, which the test should flag.
        let now = SystemClock.now_unix_s();
        assert!(now > 1_577_836_800, "got {now}");
        assert!(now < 4_102_444_800, "got {now}");
    }
}
