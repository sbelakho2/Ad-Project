//! Finding records, their canonical hash and the typed diff.
//!
//! The watch stores every finding as a small record: the stable subject, the
//! severity name and the human message the rule produced. The records of one
//! check serialize to canonical JSON (sorted by subject, then severity, then
//! message) and hash with SHA-256, so the stored `last_findings_hash` is
//! reproducible and the typed diff can name what moved.
//!
//! [`diff_findings`] reports one [`Change`] per current finding whose subject
//! is new, whose severity rose or whose message moved. A finding that
//! disappeared contributes nothing: the alert copy is about what is wrong
//! now, not about what was wrong before.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use apex_audit::Finding;

/// The subject for authorization line identities that disappeared from a
/// watched app-ads.txt. Losing every authorization is the condition the watch
/// must remember even while the file is absent or empty.
pub const APP_ADS_AUTHORIZE_LOST_SUBJECT: &str = "app_ads_txt:authorizations";
/// The severity for lost authorization identities: the publisher's ads stop
/// being authorized, which is customer-blocking.
pub const APP_ADS_AUTHORIZE_LOST_SEVERITY: &str = "critical";

/// Count canonical authorization line identities that disappeared between two
/// parsed app-ads.txt line sets. Only authorization records count: comments,
/// blanks and comma-less garbage are not identities, so losing one never
/// reports "authorizations disappeared". Replacing five records with five
/// unrelated records loses five, not zero; an empty current set loses every
/// prior authorization.
#[must_use]
pub fn app_ads_identities_lost(previous: &[String], current: &[String]) -> u32 {
    let current: std::collections::BTreeSet<&str> = current
        .iter()
        .filter(|line| apex_audit::rules::app_ads_txt::is_authorization_line(line))
        .map(String::as_str)
        .collect();
    let lost = previous
        .iter()
        .filter(|line| {
            apex_audit::rules::app_ads_txt::is_authorization_line(line)
                && !current.contains(line.as_str())
        })
        .count();
    u32::try_from(lost).unwrap_or(u32::MAX)
}

/// The severity recorded for a fetch that never produced a verdict. The
/// watch cannot prove a leak when the file cannot be read, so the finding is
/// a warning carrying the typed reason.
pub const UNREACHABLE_SUBJECT: &str = "unreachable";
/// The severity name used for [`UNREACHABLE_SUBJECT`].
pub const UNREACHABLE_SEVERITY: &str = "warning";

/// One stored finding of one check.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredFinding {
    /// The stable machine subject (`app_ads_txt`, `app_ads_txt:lines`, ...).
    pub subject: String,
    /// `info`, `warning` or `critical`.
    pub severity: String,
    /// The human message (the rule's message or the typed fetch reason).
    pub message: String,
}

impl StoredFinding {
    /// Convert one rule finding.
    pub fn from_rule(finding: &Finding) -> Self {
        Self {
            subject: finding.subject.clone(),
            severity: finding.severity.as_str().to_string(),
            message: finding.message.clone(),
        }
    }

    /// The `unreachable` finding for a typed fetch or guard reason.
    pub fn unreachable(reason: &str) -> Self {
        Self {
            subject: UNREACHABLE_SUBJECT.to_string(),
            severity: UNREACHABLE_SEVERITY.to_string(),
            message: format!("app-ads.txt could not be read: {reason}"),
        }
    }

    /// The ordinal of a stored severity name. Unknown names rank lowest so a
    /// corrupt row can never fake an urgent alarm.
    pub fn rank(&self) -> u8 {
        severity_rank(&self.severity)
    }
}

/// The ordinal of a severity name.
pub fn severity_rank(name: &str) -> u8 {
    match name {
        "critical" => 2,
        "warning" => 1,
        _ => 0,
    }
}

/// SHA-256 (lowercase hex) of the canonical JSON of `findings`.
pub fn findings_hash(findings: &[StoredFinding]) -> String {
    let mut sorted: Vec<StoredFinding> = findings.to_vec();
    sorted.sort_by(|a, b| {
        (&a.subject, &a.severity, &a.message).cmp(&(&b.subject, &b.severity, &b.message))
    });
    let canonical = serde_json::to_vec(&sorted).unwrap_or_default();
    let mut hasher = Sha256::new();
    hasher.update(&canonical);
    format!("{:x}", hasher.finalize())
}

/// One material difference between the previous check and the current one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    /// The subject did not exist in the previous check.
    New(StoredFinding),
    /// The subject existed and its severity rose.
    Worsened {
        /// The previous record.
        previous: StoredFinding,
        /// The current record.
        current: StoredFinding,
    },
    /// The subject existed with the same severity and its message moved (for
    /// example new malformed line numbers).
    Changed {
        /// The previous record.
        previous: StoredFinding,
        /// The current record.
        current: StoredFinding,
    },
}

impl Change {
    /// The finding to show in the alert copy.
    pub fn current(&self) -> &StoredFinding {
        match self {
            Change::New(finding) => finding,
            Change::Worsened { current, .. } | Change::Changed { current, .. } => current,
        }
    }

    /// One alert line for this change.
    pub fn alert_line(&self) -> String {
        match self {
            Change::New(finding) => {
                format!("- [{}] {}", finding.severity, finding.message)
            }
            Change::Worsened { previous, current } => format!(
                "- [{} -> {}] {}",
                previous.severity, current.severity, current.message
            ),
            Change::Changed { current, .. } => {
                format!("- [changed] {}", current.message)
            }
        }
    }
}

/// Diff the current findings against the previous ones by subject.
pub fn diff_findings(previous: &[StoredFinding], current: &[StoredFinding]) -> Vec<Change> {
    // Index once instead of scanning `previous` for every current finding:
    // replaying a large stored baseline was O(n^2). The first occurrence of a
    // duplicated subject wins, exactly like the previous `iter().find`.
    let mut index: std::collections::HashMap<&str, &StoredFinding> =
        std::collections::HashMap::with_capacity(previous.len());
    for finding in previous {
        index.entry(finding.subject.as_str()).or_insert(finding);
    }
    let mut changes = Vec::new();
    for finding in current {
        let Some(old) = index.get(finding.subject.as_str()).copied() else {
            changes.push(Change::New(finding.clone()));
            continue;
        };
        if finding.rank() > old.rank() {
            changes.push(Change::Worsened {
                previous: old.clone(),
                current: finding.clone(),
            });
        } else if finding.rank() == old.rank() && finding.message != old.message {
            changes.push(Change::Changed {
                previous: old.clone(),
                current: finding.clone(),
            });
        }
    }
    changes
}

#[cfg(test)]
mod tests {
    use super::*;
    use apex_audit::model::{AppAdsTxt, RuleId, Severity, Snapshot};
    use apex_core::PlatformId;

    fn finding(severity: Severity, message: &str) -> Finding {
        Finding {
            rule: RuleId::AppAdsTxt,
            severity,
            platform: PlatformId::Billing,
            subject: "app_ads_txt".to_string(),
            evidence: serde_json::json!({}),
            message: message.to_string(),
            estimated_monthly_recovery: 0,
        }
    }

    fn stored(answer: bool) -> Vec<StoredFinding> {
        let snapshot = Snapshot {
            platform: PlatformId::Billing,
            fetched_at_unix_ms: 0,
            ad_units: Vec::new(),
            waterfall: Vec::new(),
            mediation_groups: Vec::new(),
            bidder_accounts: Vec::new(),
            country_revenue: Vec::new(),
            app_ads_txt: AppAdsTxt {
                present: answer,
                lines: Vec::new(),
                expected_networks: Vec::new(),
            },
            platform_totals_30d: apex_audit::RevenueTotals {
                revenue_30d: 0,
                impressions_30d: 0,
            },
            mediation_totals_30d: apex_audit::RevenueTotals {
                revenue_30d: 0,
                impressions_30d: 0,
            },
            mediation_totals_reported: false,
        };
        apex_audit::rules::app_ads_txt::check(&snapshot)
            .iter()
            .map(StoredFinding::from_rule)
            .collect()
    }

    #[test]
    fn stored_findings_mirror_the_rule_output() {
        let finding = finding(Severity::Critical, "no app-ads.txt found");
        let stored = StoredFinding::from_rule(&finding);
        assert_eq!(stored.subject, "app_ads_txt");
        assert_eq!(stored.severity, "critical");
        assert_eq!(stored.message, "no app-ads.txt found");
        assert_eq!(stored.rank(), 2);
    }

    #[test]
    fn the_hash_is_stable_and_independent_of_order() {
        let a = StoredFinding {
            subject: "b".into(),
            severity: "warning".into(),
            message: "second".into(),
        };
        let b = StoredFinding {
            subject: "a".into(),
            severity: "critical".into(),
            message: "first".into(),
        };
        let forward = findings_hash(&[a.clone(), b.clone()]);
        let backward = findings_hash(&[b, a]);
        assert_eq!(forward, backward);
        assert_eq!(forward.len(), 64);
        assert!(forward.bytes().all(|byte| byte.is_ascii_hexdigit()));
        assert!(findings_hash(&[]).len() == 64, "the empty set still hashes");
    }

    #[test]
    fn diff_reports_new_worsened_and_changed_only() {
        let previous = stored(false);
        assert_eq!(previous.len(), 1);
        assert_eq!(previous[0].severity, "critical");

        // The same check again: no changes.
        assert!(diff_findings(&previous, &previous).is_empty());

        // Recovery: the file exists with no findings against the previous
        // Critical: nothing to alert.
        let clean = stored(true);
        assert!(diff_findings(&previous, &clean).is_empty());

        // A new problem: New.
        let changes = diff_findings(&clean, &previous);
        assert_eq!(changes.len(), 1);
        assert!(matches!(&changes[0], Change::New(f) if f.severity == "critical"));

        // Severity rise on the same subject: Worsened.
        let warning = vec![StoredFinding {
            subject: "app_ads_txt".into(),
            severity: "warning".into(),
            message: "same text".into(),
        }];
        let critical = vec![StoredFinding {
            subject: "app_ads_txt".into(),
            severity: "critical".into(),
            message: "same text".into(),
        }];
        let changes = diff_findings(&warning, &critical);
        assert_eq!(changes.len(), 1);
        assert!(matches!(&changes[0], Change::Worsened { .. }));

        // Same severity, moved message: Changed.
        let moved = vec![StoredFinding {
            subject: "app_ads_txt".into(),
            severity: "warning".into(),
            message: "different text".into(),
        }];
        let changes = diff_findings(&warning, &moved);
        assert_eq!(changes.len(), 1);
        assert!(matches!(&changes[0], Change::Changed { .. }));
        assert_eq!(
            changes[0].alert_line(),
            "- [changed] different text".to_string()
        );

        // Severity drop: nothing to alarm about.
        assert!(diff_findings(&critical, &warning).is_empty());
    }

    #[test]
    fn unreachable_findings_carry_the_typed_reason() {
        let stored = StoredFinding::unreachable("the transport failed: dns error");
        assert_eq!(stored.subject, UNREACHABLE_SUBJECT);
        assert_eq!(stored.severity, "warning");
        assert!(stored.message.contains("dns error"));
        assert_eq!(
            Change::New(stored).alert_line(),
            "- [warning] app-ads.txt could not be read: the transport failed: dns error"
        );
    }
}
