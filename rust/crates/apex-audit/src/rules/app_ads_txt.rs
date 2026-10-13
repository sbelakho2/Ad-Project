//! Rule `app_ads_txt` — app-ads.txt presence, coverage and well-formedness.
//!
//! Exact semantics:
//!
//! - `!present` => one **Critical** finding ("no app-ads.txt found"). The
//!   sub-checks (missing networks, malformed lines) are suppressed: there is no
//!   file content to evaluate, and the Critical already demands creating the
//!   file. Pinned by a test.
//! - Expected networks are normalized (trim + ASCII lowercase), deduplicated and
//!   sorted; empty entries are ignored.
//! - A line is **malformed** when it is empty after trimming, or does not split
//!   into exactly 3 comma-separated fields. Per the literal spec, "3 fields"
//!   is enough even if a field is empty, and comment lines (`# ...`) are
//!   malformed too — pinned by tests.
//! - A network counts as present iff the **first field** of some *well-formed*
//!   line equals the expected domain, case-insensitively. This implements the
//!   spec's "a line starts with the network's expected domain" for IAB-format
//!   lines (`domain, publisher_id, DIRECT`) while refusing two adversarial
//!   evasions: a suffix spoof (`admob.com.evil.example`) and the domain hiding
//!   in a later field. A malformed line cannot grant authorization, so it does
//!   not satisfy presence.
//! - Missing networks => one **Warning** listing them (sorted). Malformed lines
//!   => one **Warning** with the 1-based line numbers (sorted).
//! - Recovery: 0 for every variant; messages never claim money.

use std::collections::BTreeSet;

use crate::model::{Finding, RuleId, Severity, Snapshot};
use crate::rules::finding;

pub fn check(snapshot: &Snapshot) -> Vec<Finding> {
    let txt = &snapshot.app_ads_txt;
    let expected: BTreeSet<String> = txt
        .expected_networks
        .iter()
        .map(|domain| domain.trim().to_lowercase())
        .filter(|domain| !domain.is_empty())
        .collect();

    if !txt.present {
        let message = format!(
            "no app-ads.txt found; {} expected network listings unverified, no recovery estimated",
            expected.len()
        );
        return vec![finding(
            snapshot,
            RuleId::AppAdsTxt,
            Severity::Critical,
            "app_ads_txt".to_string(),
            serde_json::json!({
                "present": false,
                "expected_networks_count": expected.len(),
                "expected_networks": expected,
                "recovery_model": "not_applicable",
                "estimated_monthly_recovery_micros": 0,
            }),
            message,
            0,
        )];
    }

    let mut malformed_lines: Vec<u64> = Vec::new();
    let mut well_formed_domains: BTreeSet<String> = BTreeSet::new();
    for (index, raw_line) in txt.lines.iter().enumerate() {
        let line_number = index as u64 + 1;
        let trimmed = raw_line.trim();
        if trimmed.is_empty() {
            malformed_lines.push(line_number);
            continue;
        }
        let fields: Vec<&str> = trimmed.split(',').map(str::trim).collect();
        if fields.len() != 3 {
            malformed_lines.push(line_number);
            continue;
        }
        well_formed_domains.insert(fields[0].to_lowercase());
    }

    let mut findings = Vec::new();
    if !malformed_lines.is_empty() {
        let line_list = malformed_lines
            .iter()
            .map(u64::to_string)
            .collect::<Vec<String>>()
            .join(", ");
        let message = format!(
            "app-ads.txt has {} malformed line(s) at line number(s) {line_list}; expected format \
             is 'domain, publisher_id, DIRECT'; no recovery estimated",
            malformed_lines.len()
        );
        findings.push(finding(
            snapshot,
            RuleId::AppAdsTxt,
            Severity::Warning,
            "app_ads_txt:lines".to_string(),
            serde_json::json!({
                "present": true,
                "malformed_count": malformed_lines.len(),
                "malformed_line_numbers": malformed_lines,
                "recovery_model": "not_applicable",
                "estimated_monthly_recovery_micros": 0,
            }),
            message,
            0,
        ));
    }

    let missing: Vec<&String> = expected
        .iter()
        .filter(|domain| !well_formed_domains.contains(*domain))
        .collect();
    if !missing.is_empty() {
        let missing_list = missing
            .iter()
            .map(|domain| format!("`{domain}`"))
            .collect::<Vec<String>>()
            .join(", ");
        let message = format!(
            "app-ads.txt is missing {} of {} expected network lines: {missing_list}; no recovery \
             estimated",
            missing.len(),
            expected.len()
        );
        findings.push(finding(
            snapshot,
            RuleId::AppAdsTxt,
            Severity::Warning,
            "app_ads_txt:networks".to_string(),
            serde_json::json!({
                "present": true,
                "expected_networks_count": expected.len(),
                "missing_count": missing.len(),
                "missing_networks": missing,
                "recovery_model": "not_applicable",
                "estimated_monthly_recovery_micros": 0,
            }),
            message,
            0,
        ));
    }
    findings
}

#[cfg(test)]
mod tests {
    use crate::testkit::{ads_txt, empty_snapshot};

    use super::*;

    fn snapshot_with(present: bool, lines: &[&str], expected: &[&str]) -> Snapshot {
        let mut snapshot = empty_snapshot();
        snapshot.app_ads_txt = ads_txt(present, lines, expected);
        snapshot
    }

    #[test]
    fn absent_is_critical_and_suppresses_sub_findings() {
        let snapshot = snapshot_with(false, &[], &["admob.com", "metaads.com"]);
        let mut findings = check(&snapshot);
        assert_eq!(findings.len(), 1);
        let finding = findings.remove(0);
        assert_eq!(finding.severity, Severity::Critical);
        assert_eq!(finding.subject, "app_ads_txt");
        assert_eq!(finding.evidence["present"], false);
        assert_eq!(finding.evidence["expected_networks_count"], 2);
        assert_eq!(finding.estimated_monthly_recovery, 0);
        assert!(finding.message.contains("no app-ads.txt found"));
        assert!(finding.message.contains("no recovery estimated"));
    }

    #[test]
    fn all_expected_networks_present_yields_no_findings() {
        let snapshot = snapshot_with(
            true,
            &["admob.com, pub-1, DIRECT", "metaads.com, pub-2, DIRECT"],
            &["admob.com", "metaads.com"],
        );
        assert!(check(&snapshot).is_empty());
    }

    #[test]
    fn missing_networks_are_listed_sorted_and_deduped() {
        let snapshot = snapshot_with(
            true,
            &["admob.com, pub-1, DIRECT"],
            &[
                "unityads.com",
                "metaads.com",
                "unityads.com",
                " metaads.com ",
            ],
        );
        let findings = check(&snapshot);
        assert_eq!(findings.len(), 1);
        let finding = &findings[0];
        assert_eq!(finding.severity, Severity::Warning);
        assert_eq!(finding.subject, "app_ads_txt:networks");
        assert_eq!(finding.evidence["expected_networks_count"], 2);
        assert_eq!(finding.evidence["missing_count"], 2);
        assert_eq!(
            finding.evidence["missing_networks"],
            serde_json::json!(["metaads.com", "unityads.com"])
        );
        assert!(finding.message.contains("2 of 2"));
    }

    #[test]
    fn malformed_lines_report_sorted_line_numbers() {
        let snapshot = snapshot_with(
            true,
            &[
                "admob.com, pub-1, DIRECT",
                "broken line",
                "metaads.com, pub-2, DIRECT",
                "too, many, fields, here",
                "unityads.com, pub-3",
            ],
            &["admob.com", "metaads.com"],
        );
        let mut findings = check(&snapshot);
        assert_eq!(findings.len(), 1);
        let finding = findings.remove(0);
        assert_eq!(finding.severity, Severity::Warning);
        assert_eq!(finding.subject, "app_ads_txt:lines");
        assert_eq!(
            finding.evidence["malformed_line_numbers"],
            serde_json::json!([2, 4, 5])
        );
        assert_eq!(finding.evidence["malformed_count"], 3);
        assert!(finding.message.contains("3 malformed line(s)"));
        assert!(finding.message.contains("2, 4, 5"));
    }

    #[test]
    fn empty_line_is_malformed() {
        let snapshot = snapshot_with(true, &["", "   "], &[]);
        let findings = check(&snapshot);
        assert_eq!(findings.len(), 1);
        assert_eq!(
            findings[0].evidence["malformed_line_numbers"],
            serde_json::json!([1, 2])
        );
    }

    #[test]
    fn comment_line_is_malformed_under_strict_spec() {
        // Pinned deliberate strictness: the spec says a line that is not three
        // comma-separated fields is malformed, and does not exempt comments.
        let snapshot = snapshot_with(true, &["# comment"], &[]);
        let findings = check(&snapshot);
        assert_eq!(findings.len(), 1);
        assert_eq!(
            findings[0].evidence["malformed_line_numbers"],
            serde_json::json!([1])
        );
    }

    #[test]
    fn three_fields_with_empty_field_is_not_malformed() {
        // Literal spec: "not 3 fields or empty" is the whole malformed test.
        let snapshot = snapshot_with(true, &["admob.com, , DIRECT"], &[]);
        assert!(check(&snapshot).is_empty());
    }

    #[test]
    fn domain_match_is_case_insensitive() {
        let snapshot = snapshot_with(true, &["  ADMOB.COM, pub-1, Direct  "], &["admob.com"]);
        assert!(check(&snapshot).is_empty());
    }

    #[test]
    fn spoofed_suffix_domain_does_not_count() {
        let snapshot = snapshot_with(
            true,
            &["admob.com.evil.example, pub-1, DIRECT"],
            &["admob.com"],
        );
        let findings = check(&snapshot);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].evidence["missing_count"], 1);
    }

    #[test]
    fn domain_in_a_later_field_does_not_count() {
        let snapshot = snapshot_with(
            true,
            &["reseller.example, admob.com, DIRECT"],
            &["admob.com"],
        );
        let findings = check(&snapshot);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].evidence["missing_count"], 1);
    }

    #[test]
    fn malformed_line_does_not_satisfy_presence() {
        let snapshot = snapshot_with(true, &["admob.com, pub-1"], &["admob.com"]);
        let findings = check(&snapshot);
        assert_eq!(findings.len(), 2);
        assert_eq!(findings[0].subject, "app_ads_txt:lines");
        assert_eq!(findings[1].subject, "app_ads_txt:networks");
        assert_eq!(findings[1].evidence["missing_count"], 1);
    }

    #[test]
    fn empty_file_with_expected_networks_fires_missing() {
        let snapshot = snapshot_with(true, &[], &["admob.com"]);
        let findings = check(&snapshot);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].subject, "app_ads_txt:networks");
    }

    #[test]
    fn all_findings_have_zero_recovery() {
        for snapshot in [
            snapshot_with(false, &[], &["admob.com"]),
            snapshot_with(true, &["broken"], &["admob.com"]),
            snapshot_with(true, &[], &["admob.com"]),
        ] {
            for finding in check(&snapshot) {
                assert_eq!(finding.estimated_monthly_recovery, 0);
                assert!(!finding.message.contains("recoverable"));
                assert!(finding.message.contains("no recovery estimated"));
            }
        }
    }
}
