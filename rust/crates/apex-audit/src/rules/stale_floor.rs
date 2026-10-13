//! Rule `stale_floor` — a country floor is set on an ad unit, yet the pair
//! earned **zero** revenue while still serving meaningful impressions.
//!
//! Exact semantics (thresholds fire AT the boundary):
//!
//! - Consider enabled ad units only, and for each `(ad_unit, country)` floor.
//!   Stale floors on disabled units are not leaks: the unit cannot earn.
//! - Aggregate all `country_revenue` rows for the pair (duplicates are summed).
//! - Fire when pair revenue is exactly 0 AND pair impressions > 0 AND pair
//!   impressions `>= 10_000` (Warning) — `>= 100_000` (Critical).
//! - If the pair has no revenue row at all, impressions are 0 and the rule does
//!   not fire (nothing is being blocked).
//!
//! Recovery model: `expected_ecpm * impressions / 1000`, where `expected_ecpm`
//! is the **ad unit's overall eCPM** over all countries (`sum(revenue) *
//! 1000 / sum(impressions)` from `country_revenue` rows). Rationale: with the
//! floor lifted, the blocked impressions are expected to monetize at least at
//! the unit's average rate. No win-rate fudge factors are applied — the model is
//! exactly the unit eCPM applied to the blocked impressions, so the claim is
//! the minimum defensible one.
//!
//! Degenerate basis (documented rule): if the ad unit's overall eCPM is 0 or
//! unavailable (no impressions), recovery is 0 and the severity drops one level
//! (Critical -> Warning -> Info, floor at Info): we will not claim money the
//! snapshot cannot support.
//!
//! Country codes are compared normalized (trim + uppercase). Duplicate floor
//! entries for the same `(unit, country)` keep the **minimum** floor, so the
//! finding never overstates the configured barrier.

use std::collections::BTreeMap;

use apex_core::Micros;

use crate::model::{Finding, RuleId, Severity, Snapshot};
use crate::rules::{
    clamp_i64, downgrade, ecpm_i128, finding, normalize_country, recovery_from_ecpm,
};

const MIN_AFFECTED_IMPRESSIONS: u64 = 10_000;
const CRITICAL_AFFECTED_IMPRESSIONS: u64 = 100_000;

pub fn check(snapshot: &Snapshot) -> Vec<Finding> {
    // Ad-unit-wide aggregates over all countries (the eCPM basis).
    let mut unit_revenue: BTreeMap<&str, i128> = BTreeMap::new();
    let mut unit_impressions: BTreeMap<&str, u64> = BTreeMap::new();
    // (ad_unit, country) aggregates.
    let mut pair: BTreeMap<(&str, String), (i128, u64)> = BTreeMap::new();
    for row in &snapshot.country_revenue {
        let country = normalize_country(&row.country);
        *unit_revenue.entry(row.ad_unit_id.as_str()).or_insert(0) += row.revenue_30d as i128;
        let impressions = unit_impressions.entry(row.ad_unit_id.as_str()).or_insert(0);
        *impressions = impressions.saturating_add(row.impressions_30d);
        let aggregate = pair
            .entry((row.ad_unit_id.as_str(), country))
            .or_insert((0, 0));
        aggregate.0 += row.revenue_30d as i128;
        aggregate.1 = aggregate.1.saturating_add(row.impressions_30d);
    }

    // Configured floors on enabled units; keep the minimum on duplicates.
    let mut floors: BTreeMap<(&str, String), Micros> = BTreeMap::new();
    for unit in &snapshot.ad_units {
        if !unit.enabled {
            continue;
        }
        let Some(country_floors) = &unit.country_floors else {
            continue;
        };
        for floor in country_floors {
            // A zero-valued floor blocks nothing, so it can never be the cause of
            // a revenue leak; it is "no floor" written explicitly (consistent with
            // missing_floor counting a zero floor as configured). Verifier A8.
            if floor.floor_ecpm <= 0 {
                continue;
            }
            let country = normalize_country(&floor.country);
            let entry = floors
                .entry((unit.platform_unit_id.as_str(), country))
                .or_insert(floor.floor_ecpm);
            *entry = (*entry).min(floor.floor_ecpm);
        }
    }

    let mut findings = Vec::new();
    for ((unit_id, country), floor_ecpm) in floors {
        let (revenue, impressions) = pair
            .get(&(unit_id, country.clone()))
            .copied()
            .unwrap_or((0, 0));
        if revenue != 0 || impressions == 0 || impressions < MIN_AFFECTED_IMPRESSIONS {
            continue;
        }
        let mut severity = if impressions >= CRITICAL_AFFECTED_IMPRESSIONS {
            Severity::Critical
        } else {
            Severity::Warning
        };
        let unit_total_revenue = unit_revenue.get(unit_id).copied().unwrap_or(0);
        let unit_total_impressions = unit_impressions.get(unit_id).copied().unwrap_or(0);
        let unit_ecpm = ecpm_i128(unit_total_revenue, unit_total_impressions);
        // Distinguish WHY the eCPM is unavailable (verifier A9): a true zero, no
        // impressions, and an i128->i64 overflow are different operator stories.
        let (expected_ecpm, unavailable_reason) = match unit_ecpm {
            Some(value) if value > 0 => (value, None),
            Some(_) => (0, Some("zero_ecpm")),
            None if unit_total_impressions == 0 => (0, Some("no_impressions")),
            None => (0, Some("ecpm_overflow")),
        };
        if unavailable_reason.is_some() {
            severity = downgrade(severity);
        }
        let recovery = recovery_from_ecpm(expected_ecpm, impressions, 10_000);
        let recovery_model = if expected_ecpm > 0 {
            "floor_blocks_fill"
        } else {
            "unavailable_ecpm"
        };
        let message = if expected_ecpm > 0 {
            format!(
                "stale floor on ad unit {unit_id} in country {country}: floor {floor_ecpm} micros \
                 set, pair revenue 0 micros over {impressions} impressions; ad unit eCPM \
                 {expected_ecpm} micros implies {recovery} micros/month recoverable \
                 (model: {recovery_model})"
            )
        } else {
            let why = match unavailable_reason {
                Some("zero_ecpm") => "ad unit eCPM is 0 micros",
                Some("no_impressions") => "ad unit has no impressions",
                _ => "ad unit eCPM overflows the representable micros range",
            };
            format!(
                "stale floor on ad unit {unit_id} in country {country}: floor {floor_ecpm} micros \
                 set, pair revenue 0 micros over {impressions} impressions; {why}, recovery capped \
                 at 0 micros/month and severity downgraded one level (model: {recovery_model})"
            )
        };
        findings.push(finding(
            snapshot,
            RuleId::StaleFloor,
            severity,
            format!("ad_unit:{unit_id}:country:{country}"),
            serde_json::json!({
                "ad_unit_id": unit_id,
                "country": country,
                "floor_ecpm_micros": floor_ecpm,
                "revenue_30d_micros": clamp_i64(revenue),
                "impressions_30d": impressions,
                "ad_unit_total_revenue_30d_micros": clamp_i64(unit_total_revenue),
                "ad_unit_total_impressions_30d": unit_total_impressions,
                "ad_unit_ecpm_micros": expected_ecpm,
                "ecpm_unavailable_reason": unavailable_reason,
                "recovery_model": recovery_model,
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
        snapshot.ad_units = vec![unit("uA", true, vec![floor("EE", 2_500_000)])];
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
    fn fires_warning_at_exact_10k_boundary() {
        let snapshot = snapshot_with(vec![
            row("EE", "uA", 0, 10_000),
            row("US", "uA", 10_000_000, 100_000),
        ]);
        assert_eq!(only(&snapshot).severity, Severity::Warning);
    }

    #[test]
    fn does_not_fire_just_below_10k() {
        let snapshot = snapshot_with(vec![
            row("EE", "uA", 0, 9_999),
            row("US", "uA", 10_000_000, 100_000),
        ]);
        assert!(check(&snapshot).is_empty());
    }

    #[test]
    fn critical_at_exact_100k_boundary() {
        let snapshot = snapshot_with(vec![
            row("EE", "uA", 0, 100_000),
            row("US", "uA", 10_000_000, 100_000),
        ]);
        assert_eq!(only(&snapshot).severity, Severity::Critical);
    }

    #[test]
    fn warning_just_below_100k() {
        let snapshot = snapshot_with(vec![
            row("EE", "uA", 0, 99_999),
            row("US", "uA", 10_000_000, 100_000),
        ]);
        assert_eq!(only(&snapshot).severity, Severity::Warning);
    }

    #[test]
    fn no_fire_when_revenue_nonzero() {
        let snapshot = snapshot_with(vec![row("EE", "uA", 1, 100_000)]);
        assert!(check(&snapshot).is_empty());
    }

    #[test]
    fn no_fire_when_impressions_zero() {
        // Zero impressions with zero revenue: division by zero must not panic
        // and must not produce a claim.
        let snapshot = snapshot_with(vec![row("EE", "uA", 0, 0)]);
        assert!(check(&snapshot).is_empty());
    }

    #[test]
    fn no_fire_when_pair_has_no_revenue_row() {
        let snapshot = snapshot_with(vec![row("US", "uA", 10_000_000, 100_000)]);
        assert!(check(&snapshot).is_empty());
    }

    #[test]
    fn no_fire_for_disabled_unit() {
        let mut snapshot = snapshot_with(vec![row("EE", "uA", 0, 100_000)]);
        snapshot.ad_units[0].enabled = false;
        assert!(check(&snapshot).is_empty());
    }

    #[test]
    fn zero_ecpm_downgrades_critical_to_warning_with_zero_recovery() {
        // The only revenue row is the stale pair itself, so the unit eCPM is 0.
        let snapshot = snapshot_with(vec![row("EE", "uA", 0, 100_000)]);
        let finding = only(&snapshot);
        assert_eq!(finding.severity, Severity::Warning);
        assert_eq!(finding.estimated_monthly_recovery, 0);
        assert_eq!(finding.evidence["recovery_model"], "unavailable_ecpm");
        assert!(finding
            .message
            .contains("recovery capped at 0 micros/month"));
    }

    #[test]
    fn zero_ecpm_downgrades_warning_to_info() {
        let snapshot = snapshot_with(vec![row("EE", "uA", 0, 10_000)]);
        let finding = only(&snapshot);
        assert_eq!(finding.severity, Severity::Info);
        assert_eq!(finding.estimated_monthly_recovery, 0);
    }

    #[test]
    fn golden_recovery_worked_example() {
        // Hand computation:
        //   unit totals: revenue 10_000_000 micros ($10), impressions 200_000
        //   unit eCPM  : 10_000_000 * 1000 / 200_000 = 50_000 micros ($0.05)
        //   blocked    : 100_000 impressions (EE earned 0)
        //   recovery   : 50_000 * 100_000 / 1000 = 5_000_000 micros ($5.00)
        let snapshot = snapshot_with(vec![
            row("EE", "uA", 0, 100_000),
            row("US", "uA", 10_000_000, 100_000),
        ]);
        let finding = only(&snapshot);
        assert_eq!(finding.severity, Severity::Critical);
        assert_eq!(finding.estimated_monthly_recovery, 5_000_000);
        assert_eq!(finding.evidence["ad_unit_ecpm_micros"], 50_000);
        assert_eq!(
            finding.evidence["estimated_monthly_recovery_micros"],
            5_000_000
        );
        assert!(finding.message.contains("5000000 micros/month"));
    }

    #[test]
    fn golden_truncating_ecpm_is_pinned() {
        // Hand computation:
        //   unit totals: revenue 40 micros, impressions 10_003
        //   unit eCPM  : 40 * 1000 / 10_003 = 40000 / 10003 = 3.998 -> 3 micros
        //   recovery   : 3 * 10_000 / 1000 = 30 micros
        let snapshot = snapshot_with(vec![row("EE", "uA", 0, 10_000), row("US", "uA", 40, 3)]);
        let finding = only(&snapshot);
        assert_eq!(finding.evidence["ad_unit_ecpm_micros"], 3);
        assert_eq!(finding.estimated_monthly_recovery, 30);
    }

    #[test]
    fn duplicate_revenue_rows_are_summed_per_pair() {
        let snapshot = snapshot_with(vec![row("EE", "uA", 0, 4_000), row("EE", "uA", 0, 6_000)]);
        let finding = only(&snapshot);
        assert_eq!(finding.evidence["impressions_30d"], 10_000);
    }

    #[test]
    fn duplicate_floors_keep_the_minimum() {
        let mut snapshot = empty_snapshot();
        snapshot.ad_units = vec![unit(
            "uA",
            true,
            vec![floor("EE", 2_500_000), floor("EE", 777_000)],
        )];
        snapshot.country_revenue = vec![row("EE", "uA", 0, 10_000)];
        let finding = only(&snapshot);
        assert_eq!(finding.evidence["floor_ecpm_micros"], 777_000);
    }

    #[test]
    fn country_codes_are_normalized() {
        let mut snapshot = empty_snapshot();
        snapshot.ad_units = vec![unit("uA", true, vec![floor(" ee ", 2_500_000)])];
        snapshot.country_revenue = vec![row("Ee", "uA", 0, 10_000)];
        let finding = only(&snapshot);
        assert_eq!(finding.subject, "ad_unit:uA:country:EE");
        assert_eq!(finding.evidence["country"], "EE");
    }

    #[test]
    fn subject_evidence_and_platform_are_stable() {
        let snapshot = snapshot_with(vec![row("EE", "uA", 0, 100_000)]);
        let finding = only(&snapshot);
        assert_eq!(finding.rule, RuleId::StaleFloor);
        assert_eq!(finding.subject, "ad_unit:uA:country:EE");
        assert_eq!(finding.platform, apex_core::PlatformId::Admob);
        assert_eq!(finding.evidence["ad_unit_id"], "uA");
    }
}
