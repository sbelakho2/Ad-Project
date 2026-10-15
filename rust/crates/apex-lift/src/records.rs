//! The six signed record types, in the order the spec lists them. Each record
//! serializes to canonical JSON and carries its SHA-256 so the ledger entry
//! can pin the exact bytes a publisher will re-compute from.

use sha2::{Digest, Sha256};

use crate::error::{LiftError, LiftResult};

/// One daily guardrail check that fired, with its inputs.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GuardrailRecord {
    pub day: String,
    pub guardrail: String,
    pub inputs: serde_json::Value,
    pub action: String,
}

/// One month's billing for one change.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BillingRecord {
    pub change_id: String,
    pub month: String,
    pub scope_revenue: i64,
    pub lambda: f64,
    pub factor: f64,
    pub incremental: i64,
    /// This change's log share of the combined factor: the fee re-derives as
    /// `rate * incremental * share` from the signed record alone.
    pub log_share: f64,
    pub rate_bp: u16,
    pub fee_micros: i64,
    /// Whole days the change was billable this month.
    pub billed_days: u32,
    /// Days in the month, for the pro-rata re-derivation.
    pub month_days: u32,
}

/// A billing change: override, re-check or term end.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BillingChangeRecord {
    pub change_id: String,
    pub day: String,
    pub reason: String,
    /// The billed lift after the change (0 when billing stopped).
    pub billed_lambda: f64,
}

/// The six record types in spec order.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum LiftRecord {
    /// 1. Written before the first platform call.
    PreRegistration {
        experiment_id: String,
        scope: String,
        before_value: serde_json::Value,
        after_value: serde_json::Value,
        split: f64,
        metric: String,
        burn_in_days: u32,
        window_days: u32,
        looks: u32,
        extension_allowed: bool,
        guardrail_thresholds: serde_json::Value,
        code_version_hash: String,
        rate_bp: u16,
    },
    /// 2. Daily data pull: raw platform rows and their SHA-256.
    DataPull {
        day: String,
        rows_sha256: String,
        rows: serde_json::Value,
    },
    /// 3. Guardrail event.
    Guardrail(GuardrailRecord),
    /// 4. Result: ratios, estimate, bounds, bootstrap and the decision.
    Result {
        experiment_id: String,
        daily_log_ratios: Vec<f64>,
        mean_log: f64,
        lower_90: f64,
        lower_99: f64,
        bootstrap_lower: f64,
        decision: String,
        lambda: f64,
    },
    /// 5. Monthly billing.
    MonthlyBilling(BillingRecord),
    /// 6. Override or re-check.
    BillingChange(BillingChangeRecord),
}

impl LiftRecord {
    /// Canonical JSON: the exact bytes that are hashed and signed. Fails
    /// closed on non-finite numbers (serde_json silently writes `null` for
    /// NaN, which would drop the value from the signed bytes) and on any
    /// round-trip loss.
    pub fn canonical_json(&self) -> LiftResult<String> {
        let json = serde_json::to_string(self).map_err(|error| {
            LiftError::Malformed(format!("record is not serializable: {error}"))
        })?;
        if serde_json::from_str::<LiftRecord>(&json).is_err() {
            return Err(LiftError::NonFinite {
                context: "lift record serialization".into(),
            });
        }
        Ok(json)
    }

    /// Lowercase hex SHA-256 of the canonical JSON; refuses non-finite
    /// inputs rather than hashing a collapsed record.
    pub fn sha256_hex(&self) -> LiftResult<String> {
        let canon = self.canonical_json()?;
        let mut hasher = Sha256::new();
        hasher.update(canon.as_bytes());
        let digest = hasher.finalize();
        Ok(digest.iter().map(|b| format!("{b:02x}")).collect())
    }
}

/// SHA-256 of arbitrary platform rows, for the data-pull record.
pub fn rows_sha256(rows: &serde_json::Value) -> String {
    let mut hasher = Sha256::new();
    hasher.update(
        serde_json::to_string(rows)
            .unwrap_or_else(|_| "{}".to_string())
            .as_bytes(),
    );
    let digest = hasher.finalize();
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    fn guardrail() -> LiftRecord {
        LiftRecord::Guardrail(GuardrailRecord {
            day: "2026-10-05".into(),
            guardrail: "fastHarm".into(),
            inputs: serde_json::json!({"pointLift": -0.061}),
            action: "rollback".into(),
        })
    }

    #[test]
    fn canonical_json_is_stable_and_hashes_are_pinned() {
        let record = guardrail();
        let json = record.canonical_json().expect("canonical");
        assert_eq!(
            json,
            r#"{"type":"guardrail","day":"2026-10-05","guardrail":"fastHarm","inputs":{"pointLift":-0.061},"action":"rollback"}"#
        );
        // Independently computed: sha256 of the exact canonical JSON above.
        assert_eq!(
            record.sha256_hex().expect("hash"),
            "27f3aaff5fd25adc1dd56e4b19f8ae638eddcd5c69a8e941c83b5c1c8bfd94d2"
        );
    }

    #[test]
    fn every_variant_round_trips_through_canonical_json() {
        let records = vec![
            LiftRecord::PreRegistration {
                experiment_id: "e".into(),
                scope: "u1".into(),
                before_value: serde_json::json!(1),
                after_value: serde_json::json!(2),
                split: 0.25,
                metric: "revenue_per_user".into(),
                burn_in_days: 3,
                window_days: 14,
                looks: 2,
                extension_allowed: true,
                guardrail_thresholds: serde_json::json!({}),
                code_version_hash: "h".into(),
                rate_bp: 2000,
            },
            LiftRecord::DataPull {
                day: "2026-10-01".into(),
                rows_sha256: "0".repeat(64),
                rows: serde_json::json!([]),
            },
            guardrail(),
            LiftRecord::Result {
                experiment_id: "e".into(),
                daily_log_ratios: vec![0.01, 0.02],
                mean_log: 0.015,
                lower_90: 0.01,
                lower_99: 0.005,
                bootstrap_lower: 0.009,
                decision: "keep".into(),
                lambda: 0.01,
            },
            LiftRecord::MonthlyBilling(BillingRecord {
                change_id: "c".into(),
                month: "2026-11".into(),
                scope_revenue: 1,
                lambda: 0.01,
                factor: 1.01,
                incremental: 1,
                log_share: 1.0,
                rate_bp: 2000,
                fee_micros: 1,
                billed_days: 0,
                month_days: 0,
            }),
            LiftRecord::BillingChange(BillingChangeRecord {
                change_id: "c".into(),
                day: "2026-12-01".into(),
                reason: "override".into(),
                billed_lambda: 0.0,
            }),
        ];
        for record in records {
            let json = record.canonical_json().expect("canonical");
            assert_eq!(
                serde_json::from_str::<LiftRecord>(&json).expect("round trip"),
                record
            );
            assert_eq!(record.sha256_hex().expect("hash").len(), 64);
        }
    }

    #[test]
    fn non_finite_numbers_are_refused_before_hashing() {
        let record = LiftRecord::Result {
            experiment_id: "exp".into(),
            daily_log_ratios: vec![0.01],
            mean_log: 0.01,
            lower_90: f64::NAN,
            lower_99: 0.0,
            bootstrap_lower: 0.0,
            decision: "keep".into(),
            lambda: 0.01,
        };
        assert!(matches!(
            record.canonical_json(),
            Err(crate::error::LiftError::NonFinite { .. })
        ));
        assert!(record.sha256_hex().is_err());
    }

    #[test]
    fn data_pull_rows_hash_is_content_addressed() {
        let rows = serde_json::json!([{"day":"2026-10-01","revenue":"12.5"}]);
        let a = rows_sha256(&rows);
        let b = rows_sha256(&rows);
        assert_eq!(a, b);
        assert_eq!(a.len(), 64);
        let other = serde_json::json!([{"day":"2026-10-01","revenue":"12.6"}]);
        assert_ne!(a, rows_sha256(&other));
    }
}
