//! Rule `disabled_bidder_with_account` — a network for which apex holds a
//! managed account is disabled inside the mediation platform.
//!
//! Exact semantics:
//!
//! - Group `bidder_accounts` by network (ASCII case-insensitive). A network is
//!   flagged when **any** account for it has `has_apex_managed_account` and is
//!   not `enabled_in_platform`.
//! - Severity: **Critical** when the network is *known-integrated* — it appears
//!   in any `waterfall` entry or any `mediation_groups` mapping (case-insensitive
//!   match) — otherwise **Info**. The product spec first said "Warning" and then
//!   made the severity explicit ("Critical iff integrated ... else Info"); the
//!   explicit clause wins, so this rule intentionally never emits Warning.
//!   Unknown-integration + disabled is informational only: nothing in the
//!   snapshot suggests revenue is being lost.
//! - Recovery: the snapshot contains **no per-bidder revenue**, so no money
//!   model exists. Recovery is 0, `evidence.recovery_model` is
//!   `"insufficient_data"`, and the message states "insufficient per-network
//!   revenue data" and never claims an amount.
//!
//! The network spelling reported in the subject/evidence is the smallest
//! spelling among that network's accounts in byte order, so the output is
//! independent of account ordering.

use std::collections::{BTreeMap, BTreeSet};

use crate::model::{Finding, RuleId, Severity, Snapshot};
use crate::rules::finding;

#[derive(Debug, Default)]
struct BidderGroup {
    spellings: BTreeSet<String>,
    qualifying_accounts: u32,
}

pub fn check(snapshot: &Snapshot) -> Vec<Finding> {
    let mut groups: BTreeMap<String, BidderGroup> = BTreeMap::new();
    for account in &snapshot.bidder_accounts {
        let group = groups.entry(account.network.to_lowercase()).or_default();
        group.spellings.insert(account.network.clone());
        if account.has_apex_managed_account && !account.enabled_in_platform {
            group.qualifying_accounts += 1;
        }
    }

    let mut findings = Vec::new();
    for (key, group) in groups {
        if group.qualifying_accounts == 0 {
            continue;
        }
        let display = group
            .spellings
            .iter()
            .next()
            .cloned()
            .unwrap_or_else(|| key.clone());
        let waterfall_entries = snapshot
            .waterfall
            .iter()
            .filter(|entry| entry.network.eq_ignore_ascii_case(&display))
            .count();
        let mediation_mappings = snapshot
            .mediation_groups
            .iter()
            .flat_map(|group| group.mappings.iter())
            .filter(|mapping| mapping.network.eq_ignore_ascii_case(&display))
            .count();
        let integrations = waterfall_entries + mediation_mappings;
        let integrated = integrations > 0;
        let severity = if integrated {
            Severity::Critical
        } else {
            Severity::Info
        };
        let message = format!(
            "network `{display}` has an apex-managed account but is disabled in the platform; \
             it is referenced by {integrations} integrations ({waterfall_entries} waterfall \
             entries, {mediation_mappings} mediation mappings); insufficient per-network \
             revenue data, no recovery estimated"
        );
        findings.push(finding(
            snapshot,
            RuleId::DisabledBidderWithAccount,
            severity,
            format!("network:{display}"),
            serde_json::json!({
                "network": display,
                "has_apex_managed_account": true,
                "enabled_in_platform": false,
                "qualifying_accounts": group.qualifying_accounts,
                "integrated": integrated,
                "integrations": integrations,
                "waterfall_entries": waterfall_entries,
                "mediation_mappings": mediation_mappings,
                "recovery_model": "insufficient_data",
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
    use crate::testkit::{bidder, empty_snapshot, mediation_group, waterfall_entry};

    use super::*;

    fn only(snapshot: &Snapshot) -> Finding {
        let mut findings = check(snapshot);
        assert_eq!(findings.len(), 1, "expected exactly one finding");
        findings.remove(0)
    }

    #[test]
    fn critical_when_network_is_in_the_waterfall() {
        let mut snapshot = empty_snapshot();
        snapshot.bidder_accounts = vec![bidder("mintegral", true, false)];
        snapshot.waterfall = vec![waterfall_entry("e1", "uA", "mintegral")];
        let finding = only(&snapshot);
        assert_eq!(finding.severity, Severity::Critical);
    }

    #[test]
    fn critical_when_network_is_in_a_mediation_mapping() {
        let mut snapshot = empty_snapshot();
        snapshot.bidder_accounts = vec![bidder("mintegral", true, false)];
        snapshot.mediation_groups = vec![mediation_group("g1", &["mintegral"], "uA")];
        assert_eq!(only(&snapshot).severity, Severity::Critical);
    }

    #[test]
    fn info_when_not_integrated_anywhere() {
        let mut snapshot = empty_snapshot();
        snapshot.bidder_accounts = vec![bidder("mintegral", true, false)];
        let finding = only(&snapshot);
        assert_eq!(finding.severity, Severity::Info);
        assert_eq!(finding.evidence["integrated"], false);
        assert_eq!(finding.evidence["integrations"], 0);
    }

    #[test]
    fn no_finding_when_enabled_in_platform() {
        let mut snapshot = empty_snapshot();
        snapshot.bidder_accounts = vec![bidder("mintegral", true, true)];
        snapshot.waterfall = vec![waterfall_entry("e1", "uA", "mintegral")];
        assert!(check(&snapshot).is_empty());
    }

    #[test]
    fn no_finding_without_apex_managed_account() {
        let mut snapshot = empty_snapshot();
        snapshot.bidder_accounts = vec![bidder("mintegral", false, false)];
        snapshot.waterfall = vec![waterfall_entry("e1", "uA", "mintegral")];
        assert!(check(&snapshot).is_empty());
    }

    #[test]
    fn integration_matching_is_case_insensitive() {
        let mut snapshot = empty_snapshot();
        snapshot.bidder_accounts = vec![bidder("MetaAds", true, false)];
        snapshot.waterfall = vec![waterfall_entry("e1", "uA", "metaads")];
        let finding = only(&snapshot);
        assert_eq!(finding.severity, Severity::Critical);
        assert_eq!(finding.subject, "network:MetaAds");
    }

    #[test]
    fn recovery_is_zero_with_insufficient_data_model_and_no_money_claim() {
        let mut snapshot = empty_snapshot();
        snapshot.bidder_accounts = vec![bidder("mintegral", true, false)];
        snapshot.waterfall = vec![waterfall_entry("e1", "uA", "mintegral")];
        let finding = only(&snapshot);
        assert_eq!(finding.estimated_monthly_recovery, 0);
        assert_eq!(finding.evidence["recovery_model"], "insufficient_data");
        assert_eq!(finding.evidence["estimated_monthly_recovery_micros"], 0);
        assert!(finding
            .message
            .contains("insufficient per-network revenue data"));
        assert!(finding.message.contains("no recovery estimated"));
    }

    #[test]
    fn duplicate_accounts_collapse_to_one_finding_with_smallest_spelling() {
        let mut snapshot = empty_snapshot();
        snapshot.bidder_accounts = vec![
            bidder("Mintegral", false, true),
            bidder("mintegral", true, false),
            bidder("MINTEGRAL", true, false),
        ];
        snapshot.waterfall = vec![waterfall_entry("e1", "uA", "mintegral")];
        let finding = only(&snapshot);
        // Byte-order smallest spelling wins: the uppercase form sorts first.
        assert_eq!(finding.subject, "network:MINTEGRAL");
        assert_eq!(finding.evidence["qualifying_accounts"], 2);
    }

    #[test]
    fn evidence_counts_each_integration_kind() {
        let mut snapshot = empty_snapshot();
        snapshot.bidder_accounts = vec![bidder("mintegral", true, false)];
        snapshot.waterfall = vec![
            waterfall_entry("e1", "uA", "mintegral"),
            waterfall_entry("e2", "uB", "mintegral"),
        ];
        snapshot.mediation_groups = vec![mediation_group("g1", &["mintegral", "other"], "uA")];
        let finding = only(&snapshot);
        assert_eq!(finding.evidence["waterfall_entries"], 2);
        assert_eq!(finding.evidence["mediation_mappings"], 1);
        assert_eq!(finding.evidence["integrations"], 3);
    }
}
