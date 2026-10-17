//! Rule `app_ads_txt` — app-ads.txt presence, coverage and well-formedness.
//!
//! Line classification is a crate-local port of the sales engine's frozen
//! parser, `apex_sales::signals::parse_app_ads_txt` (see
//! `rust/crates/apex-sales/src/signals.rs`), so the audit and the sales engine
//! report the same errors for the same file. `apex-sales` is deliberately
//! std-only and must not be depended on, so the classification (including the
//! `is_domain` mirror) is duplicated here; the shared corpus at
//! `rust/crates/apex-audit/tests/fixtures/adstxt/` pins the two copies
//! together from both sides: this crate's `tests/app_ads_txt_corpus.rs` and
//! the sales engine's parity test read the same files and the same
//! expected-error JSON.
//!
//! Exact semantics:
//!
//! - `!present` => one **Critical** finding ("no app-ads.txt found"). The
//!   sub-checks (missing networks, malformed lines) are suppressed: there is no
//!   file content to evaluate, and the Critical already demands creating the
//!   file. Pinned by a test.
//! - Expected networks are normalized (trim + ASCII lowercase), deduplicated and
//!   sorted; empty entries are ignored.
//! - Per line (whitespace-trimmed, CRLF-tolerant, leading BOMs stripped):
//!   - `#` starts a comment anywhere on the line; comment-only and blank lines
//!     are ignored;
//!   - a line without a comma whose `NAME=VALUE` name is non-empty and fully
//!     ASCII-alphabetic is a variable (`OWNERDOMAIN=`, `MANAGERDOMAIN=`,
//!     `INVENTORYPARTNERDOMAIN=`, ...), and is never a network;
//!   - otherwise the line is a data record: `<ad system domain>, <seller
//!     account id>, <DIRECT|RESELLER>` with an optional fourth field, the
//!     certification authority id (validated for `google.com`). Fields past the
//!     fourth are ignored, exactly like the sales parser (a five-field line is
//!     accepted when every checked field is valid; pinned by the corpus).
//! - A line is **malformed** — precisely the lines the sales engine records as
//!   an `AdsTxtError` — when the record has fewer than 3 fields, a first field
//!   that is not a domain, an empty account id, a relationship other than
//!   `DIRECT`/`RESELLER`, an exact `(domain, account_id, relationship)`
//!   duplicate, or a `google.com` record whose certification id is missing or
//!   not `f08c47fec0942fa0`.
//! - A network counts as present iff the **first field** of some *accepted*
//!   record equals the expected domain, case-insensitively. This implements the
//!   spec's "a line starts with the network's expected domain" for IAB-format
//!   lines (`domain, publisher_id, DIRECT[, cert_id]`) while refusing two
//!   adversarial evasions: a suffix spoof (`admob.com.evil.example`) and the
//!   domain hiding in a later field. A malformed line cannot grant
//!   authorization, so it does not satisfy presence — except the mirrored
//!   `google.com` wrong-certification-id case, where the sales parser records
//!   the error and still accepts the record (pinned by a test).
//! - Missing networks => one **Warning** listing them (sorted). Malformed lines
//!   => one **Warning** with the 1-based line numbers (sorted).
//! - Recovery: 0 for every variant; messages never claim money.

use std::collections::BTreeSet;

use crate::model::{Finding, RuleId, Severity, Snapshot};
use crate::rules::finding;

/// `google.com`'s certification authority id; mirrored from
/// `apex_sales::signals::GOOGLE_CERT_ID`.
const GOOGLE_CERT_ID: &str = "f08c47fec0942fa0";

/// Mirror of `apex_sales::model::is_domain` (the sales engine is dependency-free
/// by design; the shared corpus pins the two copies together).
fn is_domain(s: &str) -> bool {
    let s = s.trim_end_matches('.');
    if s.len() > 253 || !s.contains('.') {
        return false;
    }
    // Split without collecting per line: the TLD is taken from the back and
    // checked with the same label rules as every other label (the original
    // `all()` covered it too).
    let label_ok = |label: &str| {
        !label.is_empty()
            && label.len() <= 63
            && !label.starts_with('-')
            && !label.ends_with('-')
            && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
    };
    let mut labels = s.split('.');
    let Some(tld) = labels.next_back() else {
        return false;
    };
    let tld_ok = tld.len() >= 2 && tld.chars().all(|c| c.is_ascii_alphabetic());
    tld_ok && label_ok(tld) && labels.all(label_ok)
}

/// What one raw line classified to, mirroring the sales parser's outcomes.
enum LineOutcome {
    /// Comment-only or blank after comment stripping: nothing to report.
    Ignored,
    /// A `NAME=VALUE` variable: recorded nowhere as a network.
    Variable,
    /// A line the shared classification reports as malformed, granting nothing.
    Malformed,
    /// Malformed *and* still accepted for presence: a `google.com` record whose
    /// certification id is missing or wrong. The sales parser records the error
    /// and keeps the record, so presence must see it too.
    MalformedRecord(String),
    /// An accepted record whose first field is this domain (ASCII lowercase).
    Record(String),
}

/// Classify one raw line exactly like `apex_sales::signals::parse_app_ads_txt`.
///
/// `seen` carries the `(domain, account_id, relationship)` keys of accepted
/// records for duplicate detection; the set is keyed so detection is O(1) per
/// line instead of a rescan of every prior record.
fn classify_line(raw: &str, seen: &mut BTreeSet<(String, String, bool)>) -> LineOutcome {
    let line = raw.split('#').next().unwrap_or_default().trim();
    let line = line.trim_start_matches('\u{feff}');
    if line.is_empty() {
        return LineOutcome::Ignored;
    }
    if !line.contains(',') {
        if let Some((name, _value)) = line.split_once('=') {
            let name = name.trim().to_ascii_uppercase();
            if !name.is_empty() && name.chars().all(|c| c.is_ascii_alphabetic()) {
                return LineOutcome::Variable;
            }
        }
    }
    let fields: Vec<&str> = line.split(',').map(str::trim).collect();
    if fields.len() < 3 {
        return LineOutcome::Malformed;
    }
    let domain = fields[0].to_ascii_lowercase();
    if !is_domain(&domain) {
        return LineOutcome::Malformed;
    }
    if fields[1].is_empty() {
        return LineOutcome::Malformed;
    }
    let direct = match fields[2].to_ascii_uppercase().as_str() {
        "DIRECT" => true,
        "RESELLER" => false,
        _ => return LineOutcome::Malformed,
    };
    let key = (domain.clone(), fields[1].to_ascii_lowercase(), direct);
    if !seen.insert(key) {
        return LineOutcome::Malformed;
    }
    if domain == "google.com" {
        let cert_id = fields
            .get(3)
            .map(|field| field.to_ascii_lowercase())
            .filter(|field| !field.is_empty());
        if cert_id.as_deref() != Some(GOOGLE_CERT_ID) {
            return LineOutcome::MalformedRecord(domain);
        }
    }
    LineOutcome::Record(domain)
}

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
    let mut seen: BTreeSet<(String, String, bool)> = BTreeSet::new();
    for (index, raw_line) in txt.lines.iter().enumerate() {
        let line_number = index as u64 + 1;
        match classify_line(raw_line, &mut seen) {
            LineOutcome::Ignored | LineOutcome::Variable => {}
            LineOutcome::Malformed => malformed_lines.push(line_number),
            LineOutcome::MalformedRecord(domain) => {
                malformed_lines.push(line_number);
                well_formed_domains.insert(domain);
            }
            LineOutcome::Record(domain) => {
                well_formed_domains.insert(domain);
            }
        }
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
            &[
                "admob.com, pub-1, DIRECT",
                "metaads.com, pub-2, DIRECT",
                "mintegral.com, pub-3, RESELLER, 0123456789abcdef",
            ],
            &["admob.com", "metaads.com", "mintegral.com"],
        );
        assert!(check(&snapshot).is_empty());
    }

    #[test]
    fn four_field_lines_satisfy_presence() {
        // The regression this rule fix addresses: a network listed only on
        // four-field lines (with a certification authority id) is present, not
        // malformed and not missing.
        let snapshot = snapshot_with(
            true,
            &[
                "  mintegral.com , pub-1, DIRECT, cert-abc  ",
                "google.com, pub-2, DIRECT, f08c47fec0942fa0",
            ],
            &["mintegral.com", "google.com"],
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
                "unityads.com, , DIRECT",
                "google.com, pub-3, FRIEND",
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
    fn blank_and_comment_lines_are_ignored() {
        let snapshot = snapshot_with(
            true,
            &[
                "",
                "   ",
                "# comment",
                "#admob.com, pub-x, DIRECT",
                "   # indented",
            ],
            &[],
        );
        assert!(check(&snapshot).is_empty());
    }

    #[test]
    fn inline_comments_are_stripped_before_parsing() {
        let snapshot = snapshot_with(
            true,
            &[
                "admob.com, pub-1, DIRECT # primary",
                "CONTACT=ads@studio.example # reach us",
            ],
            &["admob.com"],
        );
        assert!(check(&snapshot).is_empty());
    }

    #[test]
    fn variables_are_ignored_and_never_satisfy_presence() {
        let snapshot = snapshot_with(
            true,
            &[
                "OWNERDOMAIN=ads.example.com",
                "MANAGERDOMAIN = mgr.example.com",
                "INVENTORYPARTNERDOMAIN=partner.example",
                "SUBDOMAIN=games.studio.example",
            ],
            &["ads.example.com"],
        );
        let findings = check(&snapshot);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].subject, "app_ads_txt:networks");
        assert_eq!(findings[0].evidence["missing_count"], 1);
    }

    #[test]
    fn variable_like_line_with_non_alpha_name_is_malformed() {
        // `V1` is not a valid variable name, so the line falls through to the
        // record checks and fails the three-field minimum.
        let snapshot = snapshot_with(true, &["v1=thing"], &[]);
        let findings = check(&snapshot);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].subject, "app_ads_txt:lines");
        assert_eq!(
            findings[0].evidence["malformed_line_numbers"],
            serde_json::json!([1])
        );
    }

    #[test]
    fn empty_account_and_bad_relationship_are_malformed() {
        let snapshot = snapshot_with(
            true,
            &[
                ", pub-1, DIRECT",
                "admob.com, , DIRECT",
                "admob.com, pub-2, SIDEWAYS",
                "admob.com, pub-3, DIRECT, ",
            ],
            &["admob.com"],
        );
        let findings = check(&snapshot);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].subject, "app_ads_txt:lines");
        assert_eq!(
            findings[0].evidence["malformed_line_numbers"],
            serde_json::json!([1, 2, 3])
        );
    }

    #[test]
    fn duplicates_are_reported_and_presence_is_kept() {
        let snapshot = snapshot_with(
            true,
            &[
                "admob.com, pub-1, DIRECT",
                "ADMOB.com, PUB-1, direct",
                "admob.com, pub-1, RESELLER",
            ],
            &["admob.com"],
        );
        let findings = check(&snapshot);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].subject, "app_ads_txt:lines");
        assert_eq!(
            findings[0].evidence["malformed_line_numbers"],
            serde_json::json!([2])
        );
    }

    #[test]
    fn google_certification_id_is_tracked() {
        let clean = snapshot_with(
            true,
            &["google.com, pub-1, DIRECT, f08c47fec0942fa0"],
            &["google.com"],
        );
        assert!(check(&clean).is_empty());

        // A missing or wrong certification id is an error for the shared
        // classification, but the record still counts as present (mirrored).
        for lines in [
            &["google.com, pub-1, DIRECT"][..],
            &["google.com, pub-1, DIRECT, WRONGCERT"][..],
        ] {
            let snapshot = snapshot_with(true, lines, &["google.com"]);
            let findings = check(&snapshot);
            assert_eq!(findings.len(), 1);
            assert_eq!(findings[0].subject, "app_ads_txt:lines");
            assert_eq!(
                findings[0].evidence["malformed_line_numbers"],
                serde_json::json!([1])
            );
        }
    }

    #[test]
    fn five_field_records_follow_the_shared_semantics() {
        // The frozen sales parser checks the first four fields and ignores the
        // tail; a five-field line is accepted when every checked field is
        // valid (pinned from both crates by the shared corpus).
        let snapshot = snapshot_with(
            true,
            &[
                "admob.com, pub-1, DIRECT, cert-x, tail",
                "unityads.com, pub-2, SIDEWAYS, cert-y, tail",
            ],
            &["admob.com"],
        );
        let findings = check(&snapshot);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].subject, "app_ads_txt:lines");
        assert_eq!(
            findings[0].evidence["malformed_line_numbers"],
            serde_json::json!([2])
        );
    }

    #[test]
    fn bom_and_crlf_lines_are_tolerated() {
        let snapshot = snapshot_with(
            true,
            &[
                "\u{feff}applovin.com, pub-1, DIRECT\r",
                "\u{feff}\r",
                "  unityads.com, pub-2, RESELLER\r",
            ],
            &["applovin.com", "unityads.com"],
        );
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
    fn ten_thousand_line_file_classifies_with_one_duplicate_at_the_end() {
        // The duplicate scan is keyed: a 10k-line file must classify in one
        // pass, report exactly the one repeated record and keep every unique
        // record present. The pre-key shape rescanned all prior records per
        // line (quadratic at this size).
        let mut lines: Vec<String> = (0..9_999)
            .map(|i| format!("net{i}.example, pub-{i}, DIRECT"))
            .collect();
        lines.push("net0.example, pub-0, DIRECT".to_string());
        let line_refs: Vec<&str> = lines.iter().map(String::as_str).collect();
        let snapshot = snapshot_with(true, &line_refs, &["net0.example", "net9998.example"]);
        let findings = check(&snapshot);
        assert_eq!(findings.len(), 1, "only the duplicate is malformed");
        assert_eq!(findings[0].subject, "app_ads_txt:lines");
        assert_eq!(
            findings[0].evidence["malformed_line_numbers"],
            serde_json::json!([10_000])
        );
        assert_eq!(findings[0].evidence["malformed_count"], 1);
    }

    #[test]
    fn ten_thousand_distinct_lines_yield_no_findings() {
        let lines: Vec<String> = (0..10_000)
            .map(|i| format!("net{i}.example, pub-{i}, RESELLER"))
            .collect();
        let line_refs: Vec<&str> = lines.iter().map(String::as_str).collect();
        let snapshot = snapshot_with(true, &line_refs, &["net9999.example"]);
        assert!(check(&snapshot).is_empty());
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
