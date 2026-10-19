//! Test-only builders shared by the rule tests and the cross-cutting tests.

use apex_core::{
    AdFormat, AdUnit, CountryFloor, Mapping, MediationGroup, Micros, PlatformId, WaterfallEntry,
};

use crate::model::{AppAdsTxt, BidderAccount, CountryRevenueRow, RevenueTotals, Snapshot};

pub(crate) fn waterfall_entry(entry_id: &str, ad_unit_id: &str, network: &str) -> WaterfallEntry {
    WaterfallEntry {
        entry_id: entry_id.into(),
        ad_unit_id: ad_unit_id.into(),
        network: network.into(),
        instance_name: format!("instance {entry_id}"),
        position: 1,
        price_ecpm: 2_000_000,
    }
}

pub(crate) fn mediation_group(id: &str, networks: &[&str], ad_unit_id: &str) -> MediationGroup {
    MediationGroup {
        mediation_group_id: id.into(),
        name: format!("group {id}"),
        format: AdFormat::Banner,
        mappings: networks
            .iter()
            .map(|network| Mapping {
                ad_unit_id: ad_unit_id.into(),
                network: (*network).into(),
                ecpm: 1_500_000,
            })
            .collect(),
    }
}

pub(crate) fn floor(country: &str, floor_ecpm: Micros) -> CountryFloor {
    CountryFloor {
        country: country.into(),
        group: None,
        floor_ecpm,
    }
}

pub(crate) fn unit(id: &str, enabled: bool, floors: Vec<CountryFloor>) -> AdUnit {
    AdUnit {
        platform: None,
        package_name: None,
        platform_unit_id: id.into(),
        name: format!("unit {id}"),
        format: AdFormat::Banner,
        enabled,
        country_floors: if floors.is_empty() {
            None
        } else {
            Some(floors)
        },
    }
}

pub(crate) fn row(
    country: &str,
    ad_unit_id: &str,
    revenue_30d: Micros,
    impressions_30d: u64,
) -> CountryRevenueRow {
    CountryRevenueRow {
        country: country.into(),
        ad_unit_id: ad_unit_id.into(),
        revenue_30d,
        impressions_30d,
    }
}

pub(crate) fn totals(revenue_30d: Micros, impressions_30d: u64) -> RevenueTotals {
    RevenueTotals {
        revenue_30d,
        impressions_30d,
    }
}

pub(crate) fn bidder(
    network: &str,
    has_apex_managed_account: bool,
    enabled_in_platform: bool,
) -> BidderAccount {
    BidderAccount {
        network: network.into(),
        has_apex_managed_account,
        enabled_in_platform,
    }
}

pub(crate) fn ads_txt(present: bool, lines: &[&str], expected: &[&str]) -> AppAdsTxt {
    AppAdsTxt {
        present,
        lines: lines.iter().map(|line| (*line).to_string()).collect(),
        expected_networks: expected
            .iter()
            .map(|domain| (*domain).to_string())
            .collect(),
    }
}

/// Empty snapshot with an app-ads.txt present and expected networks known. With
/// every vector empty and both totals zero it must produce zero findings.
pub(crate) fn empty_snapshot() -> Snapshot {
    Snapshot {
        platform: PlatformId::Admob,
        fetched_at_unix_ms: 1_700_000_000_000,
        ad_units: Vec::new(),
        waterfall: Vec::new(),
        mediation_groups: Vec::new(),
        bidder_accounts: Vec::new(),
        country_revenue: Vec::new(),
        app_ads_txt: AppAdsTxt {
            present: true,
            lines: Vec::new(),
            expected_networks: Vec::new(),
        },
        platform_totals_30d: totals(0, 0),
        mediation_totals_30d: totals(0, 0),
        mediation_totals_reported: false,
    }
}
