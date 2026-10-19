//! Snapshot inputs and report outputs. Field names mirror the wire contract in
//! camelCase (matching `apex-core`), so fixtures port 1:1 between the Rust
//! engine and the API layer.
//!
//! The snapshot deliberately models raw management-API state: all quantities are
//! integers (no floats anywhere), and "missing" data is represented explicitly
//! (`present: false`, empty vectors, `Option<Vec<CountryFloor>>`), which is what
//! lets [`crate::engine::run`] stay infallible.

use apex_core::{AdUnit, MediationGroup, Micros, PlatformId, WaterfallEntry};
use serde::{Deserialize, Serialize};

/// `generated_by` value stamped onto every report; pinned by a test.
pub const GENERATED_BY: &str = concat!("apex-audit v", env!("CARGO_PKG_VERSION"));

/// Everything the audit engine reads. Built by `apex-connectors` from platform
/// management APIs; the engine performs no I/O of its own.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub platform: PlatformId,
    pub fetched_at_unix_ms: i64,
    pub ad_units: Vec<AdUnit>,
    pub waterfall: Vec<WaterfallEntry>,
    pub mediation_groups: Vec<MediationGroup>,
    pub bidder_accounts: Vec<BidderAccount>,
    pub country_revenue: Vec<CountryRevenueRow>,
    pub app_ads_txt: AppAdsTxt,
    pub platform_totals_30d: RevenueTotals,
    pub mediation_totals_30d: RevenueTotals,
    /// Whether the mediation-side total was actually reported by the
    /// provider. Without it (the default), a zero `mediation_totals_30d` means
    /// "not measured", never "zero revenue"; the gap rule must not fire on an
    /// unknown value. This is a transient in-memory signal, not part of the
    /// stored snapshot contract (the pinned snapshot digest must not change).
    #[serde(skip)]
    pub mediation_totals_reported: bool,
}

/// A network account as seen by the apex control plane and the platform.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BidderAccount {
    pub network: String,
    pub has_apex_managed_account: bool,
    pub enabled_in_platform: bool,
}

/// 30-day revenue/impressions for one (country, ad unit) pair.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CountryRevenueRow {
    pub country: String,
    pub ad_unit_id: String,
    pub revenue_30d: Micros,
    pub impressions_30d: u64,
}

/// The publisher's app-ads.txt as fetched (or not) from the app's website.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppAdsTxt {
    pub present: bool,
    pub lines: Vec<String>,
    /// Full expected domains, e.g. `admob.com`.
    pub expected_networks: Vec<String>,
}

/// Revenue/impressions totals for the last 30 days.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RevenueTotals {
    pub revenue_30d: Micros,
    pub impressions_30d: u64,
}

/// Finding severity. `Ord` grows with urgency (`Info < Warning < Critical`),
/// which is what the report sort relies on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Info,
    Warning,
    Critical,
}

impl Severity {
    pub fn as_str(self) -> &'static str {
        match self {
            Severity::Info => "info",
            Severity::Warning => "warning",
            Severity::Critical => "critical",
        }
    }
}

/// Stable rule identifiers; the wire strings are part of the contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuleId {
    StaleFloor,
    DisabledBidderWithAccount,
    MissingCountryFloor,
    ReportingGap,
    AppAdsTxt,
}

impl RuleId {
    pub const ALL: [RuleId; 5] = [
        RuleId::StaleFloor,
        RuleId::DisabledBidderWithAccount,
        RuleId::MissingCountryFloor,
        RuleId::ReportingGap,
        RuleId::AppAdsTxt,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            RuleId::StaleFloor => "stale_floor",
            RuleId::DisabledBidderWithAccount => "disabled_bidder_with_account",
            RuleId::MissingCountryFloor => "missing_country_floor",
            RuleId::ReportingGap => "reporting_gap",
            RuleId::AppAdsTxt => "app_ads_txt",
        }
    }
}

/// One audit result. `evidence` holds the exact numbers; `message` is the human
/// rendering and must not contain a standalone number absent from `evidence`
/// (enforced by the parity test).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Finding {
    pub rule: RuleId,
    pub severity: Severity,
    pub platform: PlatformId,
    /// Stable machine id, e.g. `ad_unit:u1:country:EE` or `country:EE`.
    pub subject: String,
    /// Exact numbers backing the finding, as nested JSON.
    pub evidence: serde_json::Value,
    pub message: String,
    /// Modelled monthly recovery in micros; 0 when no model applies.
    pub estimated_monthly_recovery: Micros,
}

/// The engine output. `Serialize` only: `generated_by` is a `&'static str`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditReport {
    /// SHA-256 (lowercase hex) of the canonical JSON of the audited snapshot.
    pub snapshot_digest: String,
    /// Sorted: severity desc, then `rule`, then `subject`.
    pub findings: Vec<Finding>,
    pub total_estimated_monthly_recovery: Micros,
    pub generated_by: &'static str,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn severity_is_ordered_by_urgency() {
        assert!(Severity::Info < Severity::Warning);
        assert!(Severity::Warning < Severity::Critical);
        assert_eq!(Severity::Critical.as_str(), "critical");
        assert_eq!(Severity::Info.as_str(), "info");
    }

    #[test]
    fn rule_id_wire_strings_are_pinned() {
        assert_eq!(RuleId::StaleFloor.as_str(), "stale_floor");
        assert_eq!(
            RuleId::DisabledBidderWithAccount.as_str(),
            "disabled_bidder_with_account"
        );
        assert_eq!(
            RuleId::MissingCountryFloor.as_str(),
            "missing_country_floor"
        );
        assert_eq!(RuleId::ReportingGap.as_str(), "reporting_gap");
        assert_eq!(RuleId::AppAdsTxt.as_str(), "app_ads_txt");
        let json = serde_json::to_string(&RuleId::MissingCountryFloor).expect("serialize");
        assert_eq!(json, "\"missing_country_floor\"");
        let back: RuleId = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, RuleId::MissingCountryFloor);
    }

    #[test]
    fn snapshot_roundtrips_with_camel_case_keys() {
        let mut snapshot = crate::testkit::empty_snapshot();
        snapshot.ad_units = vec![crate::testkit::unit(
            "u1",
            true,
            vec![crate::testkit::floor("EE", 2_500_000)],
        )];
        snapshot.waterfall = vec![crate::testkit::waterfall_entry("e1", "u1", "admob")];
        snapshot.mediation_groups = vec![crate::testkit::mediation_group("g1", &["admob"], "u1")];
        snapshot.bidder_accounts = vec![crate::testkit::bidder("admob", true, false)];
        snapshot.country_revenue = vec![crate::testkit::row("EE", "u1", 0, 50_000)];
        let json = serde_json::to_string(&snapshot).expect("serialize");
        for key in [
            "\"fetchedAtUnixMs\"",
            "\"adUnits\"",
            "\"mediationGroups\"",
            "\"bidderAccounts\"",
            "\"countryRevenue\"",
            "\"appAdsTxt\"",
            "\"platformTotals30d\"",
            "\"mediationTotals30d\"",
            "\"hasApexManagedAccount\"",
            "\"enabledInPlatform\"",
            "\"platformUnitId\"",
            "\"adUnitId\"",
            "\"mediationGroupId\"",
            "\"entryId\"",
            "\"priceEcpm\"",
            "\"countryFloors\"",
            "\"floorEcpm\"",
            "\"revenue30d\"",
            "\"impressions30d\"",
            "\"expectedNetworks\"",
        ] {
            assert!(json.contains(key), "missing {key} in {json}");
        }
        let back: Snapshot = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, snapshot);
    }

    #[test]
    fn generated_by_is_pinned_to_crate_version() {
        assert_eq!(GENERATED_BY, "apex-audit v0.1.0");
    }
}
