//! A/A qualification: the recorded null-run evidence for a publisher and
//! platform.
//!
//! What this module actually is: a signed A/A (null) run whose split passed
//! the randomization check, with an expiry, riding the ledger as an
//! `aaQualification` record. Operators and autonomy policy read it to decide
//! whether an account may write autonomously.
//!
//! What it is *not*: the monthly fee's gate. Holdout billing is gated by the
//! registered bound, the eligible roster, the bounded-outcome certificate and
//! the measurement-scope match — the public contract says exactly that, and
//! production follows it. A missing or expired qualification is surfaced as a
//! report signal; it is never presented as a billing precondition, because no
//! production path enforces one.
//!
//! The measured figures ride the signed ledger as an `aaQualification`
//! record, quantized to integer micro-units so the canonical hash is exact.

use apex_core::PlatformId;
use apex_proof::{LedgerLog, LedgerRecord};

/// The split-randomization gate: a chi-square above this value (1 degree of
/// freedom, p = 0.001, the same convention as the engine's split guardrail)
/// means the A/A arms were not assigned at the planned split, so the run
/// cannot qualify any billing.
pub const MAX_SRM_CHI_SQUARE: f64 = 10.828;

/// How long a recorded qualification stays valid: one month of daily traffic
/// can shift a unit's traffic mix, so the null evidence is re-run monthly.
pub const VALIDITY_DAYS: i64 = 31;

/// One micro (1e-6) in the record's quantized unit.
const MICRO: f64 = 1_000_000.0;

/// The scale limit of the quantized micro-unit fields; anything larger is a
/// malformed measurement, not a figure.
const MICRO_LIMIT: i64 = i64::MAX / 4;

/// The A/A (null-run) qualification of one publisher+platform pair.
#[derive(Debug, Clone, PartialEq)]
pub struct AaQualification {
    pub publisher: String,
    pub platform: String,
    /// The split-randomization chi-square the A/A run measured. A value above
    /// [`MAX_SRM_CHI_SQUARE`] fails the qualification.
    pub srm_chi_square: f64,
    /// The share of player rows whose group label resolved (0.0..=1.0).
    pub label_consistency: f64,
    /// The null-run bias estimate (the measured log-ratio mean, in standard
    /// errors of zero).
    pub estimated_null_bias: f64,
    /// When the qualification stops being valid (inclusive bound is expired).
    pub expires_at_unix_ms: i64,
}

impl AaQualification {
    /// Whether this qualification gates billing in at `now`: unexpired, with
    /// a finite split chi-square at or under [`MAX_SRM_CHI_SQUARE`]. A NaN or
    /// infinite measurement is a malformed reading and never qualifies.
    pub fn is_valid(&self, now: i64) -> bool {
        now < self.expires_at_unix_ms
            && self.srm_chi_square.is_finite()
            && self.srm_chi_square <= MAX_SRM_CHI_SQUARE
    }
}

/// Quantize a measured figure to the record's micro-unit scale. Refuses
/// non-finite and out-of-scale values so a malformed measurement can never
/// enter the signed chain.
pub fn quantize(value: f64) -> Option<i64> {
    if !value.is_finite() || value.abs() > MICRO_LIMIT as f64 {
        return None;
    }
    Some((value * MICRO).round() as i64)
}

/// Decode a quantized micro-unit figure.
fn dequantize(micros: i64) -> f64 {
    micros as f64 / MICRO
}

/// Append the signed `aaQualification` record for one A/A conclusion. The
/// publisher is the runner's billing identity; the expiry is the caller's
/// clock plus [`VALIDITY_DAYS`]. Fails closed on a non-finite or out-of-scale
/// measurement: a qualification that cannot be recorded exactly must not be
/// recorded at all.
pub fn record(
    ledger: &mut dyn LedgerLog,
    publisher: &str,
    platform: PlatformId,
    srm_chi_square: f64,
    label_consistency: f64,
    estimated_null_bias: f64,
    now_unix_ms: i64,
) -> Result<LedgerRecord, String> {
    if publisher.trim().is_empty() {
        return Err("an A/A qualification needs a publisher id".into());
    }
    let srm = quantize(srm_chi_square)
        .ok_or_else(|| "the A/A split chi-square is not a usable figure".to_string())?;
    let labels = quantize(label_consistency)
        .ok_or_else(|| "the A/A label consistency is not a usable figure".to_string())?;
    let bias = quantize(estimated_null_bias)
        .ok_or_else(|| "the A/A null bias is not a usable figure".to_string())?;
    let expires_at_unix_ms = now_unix_ms
        .checked_add(
            VALIDITY_DAYS
                .checked_mul(86_400_000)
                .ok_or("validity overflow")?,
        )
        .ok_or("expiry overflow")?;
    let record = LedgerRecord::aa_qualification(
        now_unix_ms,
        publisher,
        platform,
        srm,
        labels,
        bias,
        expires_at_unix_ms,
    );
    ledger.append(record).map_err(|error| error.to_string())
}

/// The qualification a record carries, when it is one.
pub fn from_record(record: &LedgerRecord) -> Option<AaQualification> {
    let LedgerRecord::AaQualification {
        publisher,
        platform,
        srm_chi_square_micros,
        label_consistency_micros,
        estimated_null_bias_micros,
        expires_at_unix_ms,
        ..
    } = record
    else {
        return None;
    };
    Some(AaQualification {
        publisher: publisher.clone(),
        platform: platform.as_str().to_string(),
        srm_chi_square: dequantize(*srm_chi_square_micros),
        label_consistency: dequantize(*label_consistency_micros),
        estimated_null_bias: dequantize(*estimated_null_bias_micros),
        expires_at_unix_ms: *expires_at_unix_ms,
    })
}

/// The latest valid qualification for `publisher` + `platform` at `now`, read
/// from one ledger pass. Later records supersede earlier ones, so a re-run
/// A/A replaces (and can revoke) an older qualification.
pub fn valid_qualification(
    records: &[&LedgerRecord],
    publisher: &str,
    platform: &str,
    now: i64,
) -> Option<AaQualification> {
    let mut best: Option<(i64, AaQualification)> = None;
    for record in records {
        let Some(qualification) = from_record(record) else {
            continue;
        };
        if qualification.publisher != publisher || qualification.platform != platform {
            continue;
        }
        let at = record.at_unix_ms();
        let take = match &best {
            Some((seen_at, _)) => at >= *seen_at,
            None => true,
        };
        if take {
            best = Some((at, qualification));
        }
    }
    best.map(|(_, qualification)| qualification)
        .filter(|q| q.is_valid(now))
}

/// Whether a qualification names this platform.
pub fn platform_matches(qualification: &AaQualification, platform: PlatformId) -> bool {
    qualification.platform == platform.as_str()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use apex_proof::{InMemoryLedger, Keypair};

    fn test_ledger() -> InMemoryLedger {
        InMemoryLedger::new(Keypair::from_bytes(&[9u8; 32]))
    }

    fn qualification(expires_at_unix_ms: i64, srm_chi_square: f64) -> AaQualification {
        AaQualification {
            publisher: "pub-1".into(),
            platform: "max".into(),
            srm_chi_square,
            label_consistency: 1.0,
            estimated_null_bias: 0.0,
            expires_at_unix_ms,
        }
    }

    #[test]
    fn a_current_run_with_a_clean_split_is_valid() {
        let qualification = qualification(2_000, 4.0);
        assert!(qualification.is_valid(1_000));
        // The boundary is honest: at the expiry instant the qualification no
        // longer gates anything.
        assert!(!qualification.is_valid(2_000));
    }

    #[test]
    fn an_expired_qualification_is_invalid() {
        assert!(!qualification(500, 4.0).is_valid(1_000));
    }

    #[test]
    fn a_skipped_split_never_qualifies() {
        // Above the threshold the split is refused (the guardrail convention:
        // strictly greater fails); a malformed reading (NaN, infinite) is
        // never a pass either.
        assert!(!qualification(2_000, MAX_SRM_CHI_SQUARE + 0.001).is_valid(1_000));
        assert!(!qualification(2_000, 1e6).is_valid(1_000));
        assert!(!qualification(2_000, f64::NAN).is_valid(1_000));
        assert!(!qualification(2_000, f64::INFINITY).is_valid(1_000));
    }

    #[test]
    fn a_clean_split_at_the_threshold_is_valid() {
        assert!(qualification(2_000, MAX_SRM_CHI_SQUARE - 0.001).is_valid(1_000));
    }

    #[test]
    fn quantization_refuses_non_finite_and_oversized_figures() {
        assert!(quantize(4.0).is_some());
        assert!(quantize(f64::NAN).is_none());
        assert!(quantize(f64::NEG_INFINITY).is_none());
        assert!(quantize(1e19).is_none());
    }

    #[test]
    fn recording_appends_a_valid_qualification_and_the_ledger_keeps_it() {
        let mut ledger = test_ledger();
        let now = 1_700_000_000_000;
        record(
            &mut ledger,
            "pub-1",
            PlatformId::AppLovinMax,
            4.0,
            1.0,
            0.0,
            now,
        )
        .expect("record");
        let records = ledger.records();
        let found =
            valid_qualification(&records, "pub-1", "applovin_max", now + 1).expect("qualification");
        assert_eq!(found.publisher, "pub-1");
        assert!((found.srm_chi_square - 4.0).abs() < 1e-9);
        assert!(found.is_valid(now + 86_400_000));
        assert!(
            valid_qualification(&records, "pub-1", "applovin_max", now + 32 * 86_400_000).is_none()
        );
        assert!(valid_qualification(&records, "other", "applovin_max", now + 1).is_none());
    }

    #[test]
    fn a_later_record_supersedes_an_earlier_one() {
        let mut ledger = test_ledger();
        let now = 1_700_000_000_000;
        record(
            &mut ledger,
            "pub-1",
            PlatformId::AppLovinMax,
            4.0,
            1.0,
            0.0,
            now,
        )
        .expect("record");
        // A later run with a skewed split revokes the earlier qualification.
        record(
            &mut ledger,
            "pub-1",
            PlatformId::AppLovinMax,
            MAX_SRM_CHI_SQUARE + 1.0,
            1.0,
            0.0,
            now + 1_000,
        )
        .expect("record");
        let records = ledger.records();
        assert!(valid_qualification(&records, "pub-1", "applovin_max", now + 2_000).is_none());
    }

    #[test]
    fn recording_refuses_a_publisher_less_or_malformed_qualification() {
        let mut ledger = test_ledger();
        assert!(record(&mut ledger, " ", PlatformId::AppLovinMax, 4.0, 1.0, 0.0, 0).is_err());
        assert!(record(
            &mut ledger,
            "pub-1",
            PlatformId::AppLovinMax,
            f64::NAN,
            1.0,
            0.0,
            0
        )
        .is_err());
    }
}
