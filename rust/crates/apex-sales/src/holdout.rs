//! A randomized holdout: the only honest answer to "how many customers did
//! the emails create?"
//!
//! A fixed share of publishers is assigned, by a salted hash of the
//! publisher key, to a group that is never emailed. Publishers in both groups
//! can still arrive on their own (the free-audit form, word of mouth). The
//! difference in conversion between the emailed and held-out groups is the
//! causal effect of outreach, and Newcombe's hybrid score interval gives its
//! uncertainty. Attribution by "who replied to an email" overstates the
//! effect because some of those publishers would have come anyway.
//!
//! The hash is FNV-1a (64-bit) of `salt ‖ 0x00 ‖ key`: stable across Rust
//! versions and platforms, unlike `std`'s `DefaultHasher`.

use crate::stats::{newcombe_difference, Interval};

/// 64-bit FNV-1a.
pub fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// Holdout assignment.
#[derive(Debug, Clone, PartialEq)]
pub struct Holdout {
    /// Salt; changing it re-randomizes everyone, so keep it fixed.
    pub salt: String,
    /// Share of publishers held out, in `[0, 1)`.
    pub share: f64,
}

impl Default for Holdout {
    fn default() -> Self {
        Self {
            salt: "apex-sales-holdout-v1".into(),
            share: 0.10,
        }
    }
}

impl Holdout {
    /// Uniform position of a publisher in `[0, 1)`.
    pub fn position(&self, publisher_key: &str) -> f64 {
        let mut bytes = self.salt.as_bytes().to_vec();
        bytes.push(0);
        bytes.extend_from_slice(publisher_key.as_bytes());
        (fnv1a(&bytes) >> 11) as f64 / (1u64 << 53) as f64
    }

    /// Whether the publisher is held out.
    pub fn is_held_out(&self, publisher_key: &str) -> bool {
        self.position(publisher_key) < self.share.clamp(0.0, 0.5)
    }
}

/// The causal effect of outreach on a conversion.
#[derive(Debug, Clone, PartialEq)]
pub struct Effect {
    /// Conversion rate among emailed publishers.
    pub treated_rate: f64,
    /// Conversion rate among held-out publishers.
    pub holdout_rate: f64,
    /// Difference (treated − holdout).
    pub difference: f64,
    /// 95% Newcombe interval of the difference.
    pub interval: Interval,
    /// Extra conversions caused, scaled to the treated group.
    pub incremental_conversions: f64,
}

/// Estimate the effect from conversion counts in both groups.
pub fn effect(
    treated_conv: f64,
    treated_n: f64,
    holdout_conv: f64,
    holdout_n: f64,
) -> Option<Effect> {
    if treated_n <= 0.0 || holdout_n <= 0.0 {
        return None;
    }
    let t = treated_conv / treated_n;
    let h = holdout_conv / holdout_n;
    Some(Effect {
        treated_rate: t,
        holdout_rate: h,
        difference: t - h,
        interval: newcombe_difference(treated_conv, treated_n, holdout_conv, holdout_n, 0.95),
        incremental_conversions: (t - h) * treated_n,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fnv_matches_reference_vectors() {
        assert_eq!(fnv1a(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a(b"a"), 0xaf63_dc4c_8601_ec8c);
        assert_eq!(fnv1a(b"foobar"), 0x8594_4171_f739_67e8);
    }

    #[test]
    fn assignment_is_stable_and_near_the_share() {
        let h = Holdout::default();
        let n = 20_000;
        let held = (0..n)
            .filter(|i| h.is_held_out(&format!("studio-{i}.example")))
            .count();
        let share = held as f64 / n as f64;
        assert!((share - 0.10).abs() < 0.01, "{share}");
        assert_eq!(h.is_held_out("x.io"), h.is_held_out("x.io"));
        let other = Holdout {
            salt: "other".into(),
            share: 0.10,
        };
        let moved = (0..1000)
            .filter(|i| h.is_held_out(&format!("k{i}")) != other.is_held_out(&format!("k{i}")))
            .count();
        assert!(moved > 100, "a new salt re-randomizes: {moved}");
        let greedy = Holdout {
            salt: "s".into(),
            share: 0.9,
        };
        let held = (0..2000)
            .filter(|i| greedy.is_held_out(&format!("k{i}")))
            .count();
        assert!(held < 1150, "share is capped at one half: {held}");
    }

    #[test]
    fn effect_reports_the_causal_difference() {
        // 1.8% of emailed publishers converted against 0.33% held out.
        let e = effect(90.0, 5000.0, 2.0, 600.0).unwrap();
        assert!((e.difference - (0.018 - 2.0 / 600.0)).abs() < 1e-12);
        assert!((e.incremental_conversions - 73.333_333).abs() < 1e-3);
        assert!(e.interval.low > 0.0 && e.interval.high > e.difference);
        assert!(effect(1.0, 0.0, 1.0, 10.0).is_none());
    }
}
