//! The engine: assemble all rule outputs, sort deterministically, sum the
//! modelled recovery, and digest the snapshot.
//!
//! [`run`] is infallible by design — connectors fail closed long before a
//! `Snapshot` is built, so the engine never needs to reject an input and never
//! panics on one.

use crate::digest::snapshot_digest;
use crate::errors::AuditError;
use crate::model::{AuditReport, Snapshot, GENERATED_BY};
use crate::rules;

/// Run every rule over `snapshot` and produce the sorted [`AuditReport`].
///
/// Ordering: severity descending (`Critical`, `Warning`, `Info`), then the rule
/// wire string, then `subject`. Every rule emits at most one finding per
/// subject, so this is a total order and permuting the snapshot's vectors does
/// not change the findings.
pub fn run(snapshot: &Snapshot) -> AuditReport {
    let mut findings = Vec::new();
    findings.extend(rules::stale_floor::check(snapshot));
    findings.extend(rules::disabled_bidder::check(snapshot));
    findings.extend(rules::missing_floor::check(snapshot));
    findings.extend(rules::reporting_gap::check(snapshot));
    findings.extend(rules::app_ads_txt::check(snapshot));
    findings.sort_by(|a, b| {
        b.severity
            .cmp(&a.severity)
            .then_with(|| a.rule.as_str().cmp(b.rule.as_str()))
            .then_with(|| a.subject.cmp(&b.subject))
    });
    let total = findings.iter().fold(0i128, |acc, finding| {
        acc + finding.estimated_monthly_recovery as i128
    });
    // The reported total is the exact i128 sum CLAMPED into i64: an adversarial
    // snapshot can produce recoveries whose sum exceeds i64::MAX, and the total
    // must saturate rather than wrap (pinned by total_saturates_at_i64_max).
    AuditReport {
        snapshot_digest: snapshot_digest(snapshot),
        findings,
        total_estimated_monthly_recovery: rules::clamp_recovery(total),
        generated_by: GENERATED_BY,
    }
}

/// Serialize a report for the API layer. The only fallible boundary in this
/// crate; the engine itself is infallible.
pub fn report_json(report: &AuditReport) -> Result<String, AuditError> {
    serde_json::to_string(report).map_err(AuditError::from)
}

#[cfg(test)]
mod tests {
    use crate::testkit;

    use super::*;

    #[test]
    fn run_on_empty_snapshot_reports_digest_and_version() {
        let snapshot = testkit::empty_snapshot();
        let report = run(&snapshot);
        assert!(report.findings.is_empty());
        assert_eq!(report.total_estimated_monthly_recovery, 0);
        assert_eq!(report.generated_by, "apex-audit v0.1.0");
        assert_eq!(report.snapshot_digest, snapshot_digest(&snapshot));
        assert_eq!(report.snapshot_digest.len(), 64);
    }

    #[test]
    fn report_json_round_trips_through_serde() {
        let snapshot = testkit::empty_snapshot();
        let report = run(&snapshot);
        let json = report_json(&report).expect("serialize report");
        assert!(json.contains("\"snapshotDigest\""));
        assert!(json.contains("\"generatedBy\":\"apex-audit v0.1.0\""));
        assert!(json.contains("\"totalEstimatedMonthlyRecovery\":0"));
        let value: serde_json::Value = serde_json::from_str(&json).expect("parse json");
        assert_eq!(value["findings"], serde_json::json!([]));
    }
}
