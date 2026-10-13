//! Rule `missing_country_floor` — a material country (share of platform
//! revenue) has no country floor configured on any ad unit that serves it.
//!
//! Exact semantics (thresholds fire AT the boundary):
//!
//! - Aggregate `country_revenue` rows by country: `revenue`, `impressions`, and
//!   the set of ad units with `impressions_30d > 0` ("serving" units).
//! - Skip everything when `platform_totals_30d.revenue_30d <= 0` (no
//!   denominator, no percentage — division-by-zero protection).
//! - Country share in basis points: `revenue * 10_000 / platform_revenue`
//!   (truncating). Fire at `>= 500` bp (Warning), `>= 1500` bp (Critical).
//! - A country is *covered* when at least one serving unit is enabled and has a
//!   `CountryFloor` entry for that country. Country-level scope is deliberate:
//!   the rule reports countries with **no** floor on **any** serving unit, so a
//!   partially covered country (one unit floored, another not) does not fire —
//!   that per-unit gap is out of scope for this rule and is pinned by a test.
//! - A country with no serving unit at all (revenue rows but zero impressions)
//!   is skipped: with no impressions there is nothing to protect and no eCPM.
//!
//! Presence of a floor is entry-based: a `CountryFloor` entry with
//! `floor_ecpm == 0` still counts as "configured" (the literal spec), pinned by
//! a test. Floors on disabled units do not cover anything — disabled units
//! cannot serve.
//!
//! Recovery model: `country_ecpm * country_impressions * 500 bp / 10_000`,
//! i.e. the same revenue the country already earns, times a conservative 5%
//! fill-protection uplift: floors (not lifted here, but made *present*) raise
//! the price of low-value fill, and the conservative expected effect is 5% of
//! country revenue. Golden-tested with hand-computed micros. Zero or unknown
//! country eCPM => recovery 0 (the severity is unchanged: this rule's severity
//! has no documented downgrade).

use std::collections::{BTreeMap, BTreeSet};

use crate::model::{Finding, RuleId, Severity, Snapshot};
use crate::rules::{clamp_i64, ecpm_i128, finding, normalize_country, recovery_from_ecpm};

const WARN_SHARE_BP: i128 = 500; // 5.00%
const CRITICAL_SHARE_BP: i128 = 1500; // 15.00%
const UPLIFT_BP: i64 = 500; // 5.00% conservative fill-protection uplift

#[derive(Debug, Default)]
struct CountryAggregate {
    revenue: i128,
    impressions: u64,
    serving_units: BTreeSet<String>,
}

pub fn check(snapshot: &Snapshot) -> Vec<Finding> {
    let platform_revenue = snapshot.platform_totals_30d.revenue_30d;
    if platform_revenue <= 0 {
        return Vec::new();
    }

    // Determinism under duplicate ids: merge with OR (any enabled occurrence wins)
    // so findings do not depend on vector order (verifier finding A1).
    let mut enabled: BTreeMap<&str, bool> = BTreeMap::new();
    for unit in &snapshot.ad_units {
        *enabled
            .entry(unit.platform_unit_id.as_str())
            .or_insert(false) |= unit.enabled;
    }
    let mut floors_by_unit: BTreeMap<&str, BTreeSet<String>> = BTreeMap::new();
    for unit in &snapshot.ad_units {
        let Some(country_floors) = &unit.country_floors else {
            continue;
        };
        let set = floors_by_unit
            .entry(unit.platform_unit_id.as_str())
            .or_default();
        for floor in country_floors {
            set.insert(normalize_country(&floor.country));
        }
    }

    let mut by_country: BTreeMap<String, CountryAggregate> = BTreeMap::new();
    for row in &snapshot.country_revenue {
        let aggregate = by_country
            .entry(normalize_country(&row.country))
            .or_default();
        aggregate.revenue += row.revenue_30d as i128;
        aggregate.impressions = aggregate.impressions.saturating_add(row.impressions_30d);
        if row.impressions_30d > 0 {
            aggregate.serving_units.insert(row.ad_unit_id.clone());
        }
    }

    let mut findings = Vec::new();
    for (country, aggregate) in &by_country {
        if aggregate.serving_units.is_empty() {
            continue;
        }
        let share_bp = aggregate.revenue * 10_000 / platform_revenue as i128;
        if share_bp < WARN_SHARE_BP {
            continue;
        }
        // One clamped value used in BOTH message and evidence: the parity
        // contract must hold even when i128 share overflows i64 (verifier A2).
        let share_bp_clamped = clamp_i64(share_bp);
        let severity = if share_bp >= CRITICAL_SHARE_BP {
            Severity::Critical
        } else {
            Severity::Warning
        };
        let floored_serving = aggregate
            .serving_units
            .iter()
            .filter(|unit_id| {
                enabled.get(unit_id.as_str()).copied().unwrap_or(false)
                    && floors_by_unit
                        .get(unit_id.as_str())
                        .is_some_and(|countries| countries.contains(country))
            })
            .count();
        if floored_serving > 0 {
            continue;
        }
        let ecpm = ecpm_i128(aggregate.revenue, aggregate.impressions).unwrap_or(0);
        let recovery = recovery_from_ecpm(ecpm, aggregate.impressions, UPLIFT_BP);
        let serving_units: Vec<&String> = aggregate.serving_units.iter().collect();
        let message = format!(
            "country `{country}` contributes {} micros ({} bp of platform revenue) over {} \
             impressions served by {} ad units, none of which has a country floor for it; \
             estimated recovery {recovery} micros/month (500 bp conservative fill-protection \
             uplift, model: country_ecpm_uplift)",
            clamp_i64(aggregate.revenue),
            share_bp_clamped,
            aggregate.impressions,
            serving_units.len(),
        );
        findings.push(finding(
            snapshot,
            RuleId::MissingCountryFloor,
            severity,
            format!("country:{country}"),
            serde_json::json!({
                "country": country,
                "country_revenue_30d_micros": clamp_i64(aggregate.revenue),
                "country_impressions_30d": aggregate.impressions,
                "platform_revenue_30d_micros": platform_revenue,
                "revenue_share_bp": share_bp_clamped,
                "country_ecpm_micros": ecpm,
                "uplift_bp": UPLIFT_BP,
                "serving_ad_units": serving_units,
                "serving_ad_units_count": serving_units.len(),
                "floored_serving_ad_units_count": floored_serving,
                "recovery_model": "country_ecpm_uplift",
                "estimated_monthly_recovery_micros": recovery,
            }),
            message,
            recovery,
        ));
    }
    findings
}

#[cfg(test)]
mod tests {
    use crate::testkit::{empty_snapshot, floor, row, totals, unit};

    use super::*;

    fn snapshot_with(rows: Vec<crate::model::CountryRevenueRow>) -> Snapshot {
        let mut snapshot = empty_snapshot();
        snapshot.ad_units = vec![unit("uA", true, Vec::new())];
        snapshot.platform_totals_30d = totals(10_000_000, 100_000);
        snapshot.country_revenue = rows;
        snapshot
    }

    fn only(snapshot: &Snapshot) -> Finding {
        let mut findings = check(snapshot);
        assert_eq!(findings.len(), 1, "expected exactly one finding");
        findings.remove(0)
    }

    #[test]
    fn warning_at_exact_5_percent_boundary() {
        // 500_000 / 10_000_000 = exactly 500 bp.
        let snapshot = snapshot_with(vec![row("EE", "uA", 500_000, 10_000)]);
        let finding = only(&snapshot);
        assert_eq!(finding.severity, Severity::Warning);
        assert_eq!(finding.evidence["revenue_share_bp"], 500);
        // recovery = (500_000 * 1000 / 10_000) * 10_000 / 1000 * 500 / 10_000
        //          = 50_000 * 10 * 500 / 10_000 = 25_000 micros
        assert_eq!(finding.estimated_monthly_recovery, 25_000);
    }

    #[test]
    fn does_not_fire_just_below_5_percent() {
        let snapshot = snapshot_with(vec![row("EE", "uA", 499_000, 10_000)]);
        assert!(check(&snapshot).is_empty());
    }

    #[test]
    fn critical_at_exact_15_percent_boundary() {
        let snapshot = snapshot_with(vec![row("EE", "uA", 1_500_000, 10_000)]);
        let finding = only(&snapshot);
        assert_eq!(finding.severity, Severity::Critical);
        assert_eq!(finding.evidence["revenue_share_bp"], 1_500);
    }

    #[test]
    fn warning_just_below_15_percent() {
        let snapshot = snapshot_with(vec![row("EE", "uA", 1_499_000, 10_000)]);
        assert_eq!(only(&snapshot).severity, Severity::Warning);
    }

    #[test]
    fn no_fire_when_serving_unit_has_a_floor() {
        let mut snapshot = snapshot_with(vec![row("EE", "uA", 1_500_000, 10_000)]);
        snapshot.ad_units = vec![unit("uA", true, vec![floor("EE", 1_000_000)])];
        assert!(check(&snapshot).is_empty());
    }

    #[test]
    fn zero_valued_floor_entry_counts_as_configured() {
        // Literal spec: coverage is "has a CountryFloor entry", regardless of
        // its value. Pinned so the interpretation is intentional.
        let mut snapshot = snapshot_with(vec![row("EE", "uA", 1_500_000, 10_000)]);
        snapshot.ad_units = vec![unit("uA", true, vec![floor("EE", 0)])];
        assert!(check(&snapshot).is_empty());
    }

    #[test]
    fn no_fire_without_serving_units() {
        // Revenue recorded but zero impressions: no eCPM, nothing to protect.
        let snapshot = snapshot_with(vec![row("EE", "uA", 1_500_000, 0)]);
        assert!(check(&snapshot).is_empty());
    }

    #[test]
    fn no_fire_when_platform_revenue_is_zero() {
        let mut snapshot = snapshot_with(vec![row("EE", "uA", 1_500_000, 10_000)]);
        snapshot.platform_totals_30d = totals(0, 0);
        assert!(check(&snapshot).is_empty());
    }

    #[test]
    fn no_fire_when_platform_revenue_is_negative() {
        let mut snapshot = snapshot_with(vec![row("EE", "uA", 1_500_000, 10_000)]);
        snapshot.platform_totals_30d = totals(-1, 0);
        assert!(check(&snapshot).is_empty());
    }

    #[test]
    fn floor_on_disabled_unit_does_not_cover() {
        let mut snapshot = snapshot_with(vec![row("EE", "uA", 1_500_000, 10_000)]);
        snapshot.ad_units = vec![unit("uA", false, vec![floor("EE", 1_000_000)])];
        assert_eq!(only(&snapshot).severity, Severity::Critical);
    }

    #[test]
    fn partial_coverage_does_not_fire_documented_scope() {
        // uA is floored, uB is not, both serve EE. Country-level scope: the
        // country has a floor on a serving unit, so it does not fire.
        let mut snapshot = snapshot_with(vec![
            row("EE", "uA", 1_000_000, 10_000),
            row("EE", "uB", 500_000, 5_000),
        ]);
        snapshot.ad_units = vec![
            unit("uA", true, vec![floor("EE", 1_000_000)]),
            unit("uB", true, Vec::new()),
        ];
        assert!(check(&snapshot).is_empty());
    }

    #[test]
    fn unit_absent_from_ad_units_is_uncovered() {
        let snapshot = snapshot_with(vec![row("EE", "ghost", 1_500_000, 10_000)]);
        assert_eq!(only(&snapshot).severity, Severity::Critical);
    }

    #[test]
    fn golden_recovery_worked_example() {
        // Hand computation:
        //   country DE: revenue 30_000_000 micros ($30), impressions 300_000
        //   country eCPM: 30_000_000 * 1000 / 300_000 = 100_000 micros
        //   share       : 30_000_000 / 100_000_000 = 3000 bp -> Critical
        //   recovery    : 100_000 * 300_000 / 1000 * 500 / 10_000
        //               = 30_000_000 * 5% = 1_500_000 micros ($1.50)
        let mut snapshot = empty_snapshot();
        snapshot.ad_units = vec![unit("uB", true, Vec::new())];
        snapshot.platform_totals_30d = totals(100_000_000, 1_000_000);
        snapshot.country_revenue = vec![row("DE", "uB", 30_000_000, 300_000)];
        let finding = only(&snapshot);
        assert_eq!(finding.severity, Severity::Critical);
        assert_eq!(finding.evidence["country_ecpm_micros"], 100_000);
        assert_eq!(finding.estimated_monthly_recovery, 1_500_000);
        assert!(finding.message.contains("1500000 micros/month"));
    }

    #[test]
    fn subject_evidence_and_serving_units_shape() {
        let mut snapshot = snapshot_with(vec![
            row("EE", "uA", 500_000, 10_000),
            row("EE", "uB", 100_000, 2_000),
        ]);
        snapshot.ad_units = vec![unit("uA", true, Vec::new()), unit("uB", true, Vec::new())];
        let finding = only(&snapshot);
        assert_eq!(finding.rule, RuleId::MissingCountryFloor);
        assert_eq!(finding.subject, "country:EE");
        assert_eq!(finding.platform, apex_core::PlatformId::Admob);
        assert_eq!(finding.evidence["serving_ad_units_count"], 2);
        assert_eq!(
            finding.evidence["serving_ad_units"],
            serde_json::json!(["uA", "uB"])
        );
    }

    #[test]
    fn country_codes_are_normalized_for_rows_and_floors() {
        let mut snapshot = snapshot_with(vec![row(" ee ", "uA", 1_500_000, 10_000)]);
        snapshot.ad_units = vec![unit("uA", true, vec![floor("ee", 1_000_000)])];
        assert!(check(&snapshot).is_empty());
        snapshot.ad_units = vec![unit("uA", true, Vec::new())];
        let finding = only(&snapshot);
        assert_eq!(finding.subject, "country:EE");
    }

    #[test]
    fn zero_country_ecpm_has_zero_recovery_but_keeps_severity() {
        // Share is 500 bp (fires), but the country eCPM truncates to 0
        // (500_000 * 1000 / 1_000_000_000 = 0), so recovery is 0 while the
        // severity stays Warning (this rule has no documented downgrade).
        let snapshot = snapshot_with(vec![row("EE", "uA", 500_000, 1_000_000_000)]);
        let finding = only(&snapshot);
        assert_eq!(finding.severity, Severity::Warning);
        assert_eq!(finding.evidence["country_ecpm_micros"], 0);
        assert_eq!(finding.estimated_monthly_recovery, 0);
    }
}
