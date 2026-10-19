//! Canonical snapshot digest (SHA-256 over the compact JSON of the snapshot).
//!
//! Canonical here means: typed `serde_json` serialization, struct fields in
//! declaration order, no maps, no floats — so the bytes depend only on the
//! snapshot's field values and vector order. Two snapshots that are `==` always
//! produce the same digest; changing any single micro changes it.

use sha2::{Digest, Sha256};

use crate::model::Snapshot;

/// Lowercase hex SHA-256 of the canonical JSON of `snapshot`.
pub fn snapshot_digest(snapshot: &Snapshot) -> String {
    let canonical = match serde_json::to_vec(snapshot) {
        Ok(bytes) => bytes,
        // Unreachable for the current `Snapshot` (integers, strings, bools and
        // sequences only). Kept as a total function so `engine::run` stays
        // infallible; the derived `Debug` output still distinguishes values.
        Err(_) => format!("{snapshot:?}").into_bytes(),
    };
    let mut hasher = Sha256::new();
    hasher.update(&canonical);
    hex_lower(&hasher.finalize())
}

fn hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use crate::testkit;

    use super::*;

    #[test]
    fn digest_is_64_lowercase_hex_and_stable() {
        let snapshot = testkit::empty_snapshot();
        let first = snapshot_digest(&snapshot);
        let second = snapshot_digest(&snapshot.clone());
        assert_eq!(first, second);
        assert_eq!(first.len(), 64);
        assert!(first.bytes().all(|b| b.is_ascii_hexdigit()));
        assert!(first.bytes().all(|b| !b.is_ascii_uppercase()));
    }

    #[test]
    fn digest_changes_when_one_micro_changes() {
        let mut snapshot = testkit::empty_snapshot();
        snapshot.platform_totals_30d.revenue_30d = 1_000_000;
        let before = snapshot_digest(&snapshot);
        snapshot.platform_totals_30d.revenue_30d = 1_000_001;
        let after = snapshot_digest(&snapshot);
        assert_ne!(before, after);
    }

    #[test]
    fn digest_changes_for_any_single_field() {
        let base = testkit::empty_snapshot();
        let changes: Vec<fn(&mut Snapshot)> = vec![
            |s: &mut Snapshot| s.fetched_at_unix_ms += 1,
            |s: &mut Snapshot| s.platform = apex_core::PlatformId::LevelPlay,
            |s: &mut Snapshot| s.app_ads_txt.expected_networks.push("admob.com".into()),
            |s: &mut Snapshot| s.app_ads_txt.present = false,
        ];
        for change in changes {
            let mut mutated = base.clone();
            change(&mut mutated);
            assert_ne!(snapshot_digest(&base), snapshot_digest(&mutated));
        }
    }

    #[test]
    fn digest_is_pinned_for_a_worked_snapshot() {
        // Pins the canonical-JSON contract: field order, camelCase rename,
        // skip_serializing_if on absent floors, integer encoding. If this test
        // starts failing, the digest format changed and must be versioned.
        let mut snapshot = testkit::empty_snapshot();
        snapshot.fetched_at_unix_ms = 1_700_000_000_000;
        snapshot.ad_units = vec![testkit::unit(
            "u1",
            true,
            vec![testkit::floor("EE", 2_500_000)],
        )];
        snapshot.country_revenue = vec![testkit::row("EE", "u1", 0, 50_000)];
        snapshot.platform_totals_30d = testkit::totals(10_000_000, 100_000);
        snapshot.mediation_totals_30d = testkit::totals(10_000_000, 100_000);
        snapshot.mediation_totals_reported = true;
        snapshot.app_ads_txt =
            testkit::ads_txt(true, &["admob.com, pub-1, DIRECT"], &["admob.com"]);
        assert_eq!(
            snapshot_digest(&snapshot),
            "08f654ab75306fb5f1eae7bcdb641d6d80e51fb9c74b2c39d3d95d86a766519b"
        );
    }
}

#[cfg(test)]
mod extra_sensitivity_tests {
    use super::*;
    use crate::model::AuditReport;
    use crate::testkit::{empty_snapshot, mediation_group, row, totals, unit, waterfall_entry};

    fn digest_of(snapshot: &crate::model::Snapshot) -> String {
        snapshot_digest(snapshot)
    }

    #[test]
    fn digest_changes_for_unit_waterfall_and_mediation_fields() {
        let build = || {
            let mut s = empty_snapshot();
            s.platform_totals_30d = totals(1_000_000, 1_000);
            s.mediation_totals_30d = totals(1_000_000, 1_000);
            s.mediation_totals_reported = true;
            s.ad_units = vec![unit("u1", true, vec![])];
            s.waterfall = vec![waterfall_entry("e1", "u1", "admob")];
            s.mediation_groups = vec![mediation_group("g1", &["admob"], "u1")];
            s.country_revenue = vec![row("EE", "u1", 1, 1)];
            s
        };
        let base = digest_of(&build());

        let mut units = build();
        units.ad_units[0].name = "renamed-ad-unit".into();
        assert_ne!(
            digest_of(&units),
            base,
            "ad unit content must move the digest"
        );

        let mut waterfall = build();
        waterfall.waterfall[0].price_ecpm += 1;
        assert_ne!(
            digest_of(&waterfall),
            base,
            "waterfall content must move the digest"
        );

        let mut mediation = build();
        mediation.mediation_groups[0].mappings[0].ecpm += 1;
        assert_ne!(
            digest_of(&mediation),
            base,
            "mediation content must move the digest"
        );

        let mut revenue = build();
        revenue.country_revenue[0].revenue_30d += 1;
        assert_ne!(
            digest_of(&revenue),
            base,
            "revenue rows must move the digest"
        );
    }

    #[test]
    fn report_digest_matches_snapshot_digest() {
        let mut s = empty_snapshot();
        s.platform_totals_30d = totals(1, 1);
        s.mediation_totals_30d = totals(1, 1);
        s.mediation_totals_reported = true;
        let report: AuditReport = crate::engine::run(&s);
        assert_eq!(report.snapshot_digest, digest_of(&s));
    }
}
