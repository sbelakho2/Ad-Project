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
    /// The treated (exposed) revenue the fee was applied to. Legacy records
    /// written before A3 default to the scope revenue.
    #[serde(default)]
    pub treated_revenue: i64,
    /// The A3 holdout reading behind the line (measured lift, bounds,
    /// references). Absent on legacy lift-fee records.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub holdout: Option<crate::holdout::HoldoutInvoiceLine>,
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
    /// Signed ledger reference of the pre-registration record this fee was
    /// built from. Preserved on the billing record so the provenance of the
    /// fee can be re-verified from the signed month line alone.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub pre_registration_hash: String,
    /// Signed ledger reference of the result record that decided the billed
    /// lift, preserved beside the pre-registration reference.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub result_hash: String,
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
#[allow(clippy::large_enum_variant)] // signed records are boxed at the ledger boundary; keep one enum
pub enum LiftRecord {
    /// 1. Written before the first platform call.
    PreRegistration {
        experiment_id: String,
        /// The immutable experiment instance this registration pinned. A
        /// retry or restart reuses no instance, even when it reuses the
        /// platform experiment name.
        #[serde(default, skip_serializing_if = "String::is_empty")]
        instance_id: String,
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
        /// The pre-registered minimum point lift the final/interim keep
        /// condition uses. Records written before this field existed carry
        /// the historical 1% default.
        #[serde(default = "default_min_point_lift")]
        min_point_lift: f64,
        code_version_hash: String,
        rate_bp: u16,
        /// The unit randomization happens on. Apex randomizes users.
        #[serde(default = "default_randomization_unit")]
        randomization_unit: String,
        /// Who assigns a unit to an arm: the experiment provider, Apex, or
        /// the publisher.
        #[serde(default = "default_assignment_source")]
        assignment_source: String,
        /// The assignment epoch the split belongs to (a string-encoded
        /// u64), so a later re-run never silently joins an old cohort.
        #[serde(default = "default_assignment_epoch")]
        assignment_epoch: String,
        /// The estimator this registration pins; a change of estimator id is
        /// a different contract and changes the signed hash.
        #[serde(default = "default_estimator_id")]
        estimator_id: String,
        /// What the estimator measures outcomes from.
        #[serde(default = "default_outcome_source")]
        outcome_source: String,
        /// The measurement scope; on registration it equals the record's
        /// `scope`. Legacy records without the field rebuild with `scope`.
        #[serde(default, skip_serializing_if = "String::is_empty")]
        measurement_scope: String,
        /// Who counts as measurable: the registered eligible users.
        #[serde(default = "default_eligibility")]
        eligibility: String,
        /// The identity keys the measurement joins on.
        #[serde(default = "default_identity_contract")]
        identity_contract: String,
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
        /// The immutable experiment instance this result belongs to.
        #[serde(default, skip_serializing_if = "String::is_empty")]
        instance_id: String,
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
    /// The durable 90/10 holdout state of one kept change, keyed by the
    /// immutable experiment instance. Written once when the change is kept
    /// and updated only to record a rollback.
    HoldoutState(crate::holdout::HoldoutState),
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

/// The minimum point lift legacy pre-registration records pinned before the
/// field was configurable. New records always carry the policy's own value.
pub fn default_min_point_lift() -> f64 {
    0.01
}

/// The registered randomization unit: Apex randomizes users.
/// The estimator a registration pins when this runtime can execute it: the
/// planned-share arm-total estimator over labelled player rows.
pub const PLANNED_SHARE_ESTIMATOR_ID: &str = "planned_share_total_v2";

/// The outcome a planned-share registration must measure: the per-player
/// revenue rows, not the platform's daily ratios.
pub const PLAYER_REVENUE_OUTCOME_SOURCE: &str = "player_revenue";

/// The assignment sources this runtime recognises. A native experiment is
/// assigned by the provider; Apex and the publisher are the other legal
/// authorities the contract names.
pub const ASSIGNMENT_SOURCES: [&str; 3] = ["provider", "apex", "publisher"];

/// The executable estimator contract a pre-registration pins. The runtime
/// refuses to bill a reading whose signed contract is not exactly one of
/// these variants: the fields are not descriptive metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EstimatorContract {
    /// Planned-share arm totals over labelled player rows, with the
    /// registered 90/10 split supplying the denominators.
    PlannedSharePlayerTotalsV2 {
        /// Who assigned a unit to an arm.
        assignment_source: AssignmentSource,
    },
}

/// The legal assignment sources of [`EstimatorContract`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssignmentSource {
    /// The experiment provider assigns exposure.
    Provider,
    /// Apex assigns exposure.
    Apex,
    /// The publisher assigns exposure.
    Publisher,
}

impl AssignmentSource {
    /// Parse the signed assignment source, refusing an unknown authority.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim() {
            "provider" => Some(Self::Provider),
            "apex" => Some(Self::Apex),
            "publisher" => Some(Self::Publisher),
            _ => None,
        }
    }

    /// The signed spelling.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Provider => "provider",
            Self::Apex => "apex",
            Self::Publisher => "publisher",
        }
    }
}

impl EstimatorContract {
    /// Parse a registration's signed contract fields into the one executable
    /// variant. Any combination the runtime does not implement is a typed
    /// refusal, so settlement can never silently substitute another
    /// estimator for the one the publisher signed.
    pub fn parse(
        estimator_id: &str,
        randomization_unit: &str,
        assignment_source: &str,
        outcome_source: &str,
    ) -> Result<Self, String> {
        if estimator_id.trim() != PLANNED_SHARE_ESTIMATOR_ID {
            return Err(format!(
                "the signed estimator '{estimator_id}' is not executable by this runtime"
            ));
        }
        if randomization_unit.trim() != "user" {
            return Err(format!(
                "the planned-share estimator needs user-level randomization, not '{randomization_unit}'"
            ));
        }
        let Some(assignment_source) = AssignmentSource::parse(assignment_source) else {
            return Err(format!(
                "unknown assignment source '{assignment_source}'; expected one of {ASSIGNMENT_SOURCES:?}"
            ));
        };
        if outcome_source.trim() != PLAYER_REVENUE_OUTCOME_SOURCE {
            return Err(format!(
                "the planned-share estimator measures player revenue, not '{outcome_source}'"
            ));
        }
        Ok(Self::PlannedSharePlayerTotalsV2 { assignment_source })
    }
}

pub fn default_randomization_unit() -> String {
    "user".into()
}

/// The registered assignment source for records written before the
/// estimator contract existed: Apex assigned the split.
pub fn default_assignment_source() -> String {
    "apex".into()
}

/// The assignment epoch of legacy registrations.
pub fn default_assignment_epoch() -> String {
    "0".into()
}

/// The registered estimator: the planned-share total estimator, version 2.
pub fn default_estimator_id() -> String {
    "planned_share_total_v2".into()
}

/// The registered outcome source of legacy records: the daily ratios the
/// engine has always estimated from.
pub fn default_outcome_source() -> String {
    "daily_ratio".into()
}

/// The registered measurable population.
pub fn default_eligibility() -> String {
    "eligible_users".into()
}

/// The registered identity contract: IDFV/IDFA joined to the user id.
pub fn default_identity_contract() -> String {
    "idfv_idfa_user".into()
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

    #[test]
    fn the_signed_estimator_contract_is_executable_or_refused() {
        let ok = super::EstimatorContract::parse(
            "planned_share_total_v2",
            "user",
            "provider",
            "player_revenue",
        )
        .expect("the executable contract parses");
        assert_eq!(
            ok,
            super::EstimatorContract::PlannedSharePlayerTotalsV2 {
                assignment_source: super::AssignmentSource::Provider
            }
        );
        // The legacy mismatch (a planned-share id beside the daily-ratio
        // outcome) is refused: the runtime cannot execute that pairing.
        assert!(super::EstimatorContract::parse(
            "planned_share_total_v2",
            "user",
            "apex",
            "daily_ratio"
        )
        .is_err());
        assert!(super::EstimatorContract::parse(
            "some_other_estimator",
            "user",
            "provider",
            "player_revenue"
        )
        .is_err());
        assert!(super::EstimatorContract::parse(
            "planned_share_total_v2",
            "impression",
            "provider",
            "player_revenue"
        )
        .is_err());
        assert!(super::EstimatorContract::parse(
            "planned_share_total_v2",
            "user",
            "nobody",
            "player_revenue"
        )
        .is_err());
    }

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
                instance_id: "e@1".into(),
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
                min_point_lift: 0.01,
                code_version_hash: "h".into(),
                rate_bp: 2000,
                randomization_unit: default_randomization_unit(),
                assignment_source: default_assignment_source(),
                assignment_epoch: default_assignment_epoch(),
                estimator_id: default_estimator_id(),
                outcome_source: default_outcome_source(),
                measurement_scope: "u1".into(),
                eligibility: default_eligibility(),
                identity_contract: default_identity_contract(),
            },
            LiftRecord::DataPull {
                day: "2026-10-01".into(),
                rows_sha256: "0".repeat(64),
                rows: serde_json::json!([]),
            },
            guardrail(),
            LiftRecord::Result {
                experiment_id: "e".into(),
                instance_id: "e@1".into(),
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
                treated_revenue: 1,
                holdout: None,
                lambda: 0.01,
                factor: 1.01,
                incremental: 1,
                log_share: 1.0,
                rate_bp: 2000,
                fee_micros: 1,
                billed_days: 0,
                month_days: 0,
                pre_registration_hash: "pre".into(),
                result_hash: "res".into(),
            }),
            LiftRecord::BillingChange(BillingChangeRecord {
                change_id: "c".into(),
                day: "2026-12-01".into(),
                reason: "override".into(),
                billed_lambda: 0.0,
            }),
            LiftRecord::HoldoutState(crate::holdout::HoldoutState::open(
                "c@1",
                "c",
                "u1",
                "applovin_max",
                "2026-12-01",
                false,
                "pre",
                "result",
            )),
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
            instance_id: "exp@1".into(),
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

    /// A registration with an explicit (non-default) estimator contract.
    fn contract_record() -> LiftRecord {
        LiftRecord::PreRegistration {
            experiment_id: "e".into(),
            instance_id: "e@1".into(),
            scope: "u1".into(),
            before_value: serde_json::json!(1),
            after_value: serde_json::json!({"change": "raise the floor"}),
            split: 0.25,
            metric: "revenue_per_user".into(),
            burn_in_days: 3,
            window_days: 14,
            looks: 2,
            extension_allowed: true,
            guardrail_thresholds: serde_json::json!({
                "harmPoint": -0.05,
                "fillRewarded": 0.02,
                "fillBanner": 0.05,
                "splitChiSquare": 10.8276,
            }),
            min_point_lift: 0.01,
            code_version_hash: "h".into(),
            rate_bp: 2000,
            randomization_unit: "user".into(),
            assignment_source: "provider".into(),
            assignment_epoch: "1729".into(),
            estimator_id: "planned_share_total_v2".into(),
            outcome_source: "user_revenue".into(),
            measurement_scope: "u1/de".into(),
            eligibility: "eligible_users".into(),
            identity_contract: "idfv_idfa_user".into(),
        }
    }

    #[test]
    fn the_preregistration_estimator_contract_round_trips_and_is_signed() {
        let record = contract_record();
        let json = record.canonical_json().expect("canonical");
        // Every contract field is inside the signed bytes.
        for field in [
            "randomization_unit",
            "assignment_source",
            "assignment_epoch",
            "estimator_id",
            "outcome_source",
            "measurement_scope",
            "eligibility",
            "identity_contract",
        ] {
            assert!(
                json.contains(field),
                "{field} missing from the signed {json}"
            );
        }
        let parsed: LiftRecord = serde_json::from_str(&json).expect("round trip");
        assert_eq!(
            parsed, record,
            "the contract survives the signed round trip"
        );
        let rebuilt = crate::decision::PreRegistration::from_record(&parsed).expect("rebuilt");
        assert_eq!(rebuilt.randomization_unit, "user");
        assert_eq!(rebuilt.assignment_source, "provider");
        assert_eq!(rebuilt.assignment_epoch, "1729");
        assert_eq!(rebuilt.estimator_id, "planned_share_total_v2");
        assert_eq!(rebuilt.outcome_source, "user_revenue");
        assert_eq!(rebuilt.measurement_scope, "u1/de");
        assert_eq!(rebuilt.eligibility, "eligible_users");
        assert_eq!(rebuilt.identity_contract, "idfv_idfa_user");

        // A different estimator is a different signed contract.
        let mut changed = record.clone();
        let LiftRecord::PreRegistration { estimator_id, .. } = &mut changed else {
            unreachable!("the contract record is a pre-registration");
        };
        *estimator_id = "planned_share_total_v3".into();
        assert_ne!(
            record.sha256_hex().expect("hash"),
            changed.sha256_hex().expect("hash"),
            "changing the estimator id changes the signed hash"
        );
    }

    #[test]
    fn a_legacy_preregistration_defaults_the_estimator_contract() {
        // Records written before the contract existed deserialize with the
        // pinned defaults; the measurement scope rebuilds as the scope.
        let mut value = serde_json::to_value(contract_record()).expect("value");
        let object = value.as_object_mut().expect("object");
        for field in [
            "randomization_unit",
            "assignment_source",
            "assignment_epoch",
            "estimator_id",
            "outcome_source",
            "measurement_scope",
            "eligibility",
            "identity_contract",
        ] {
            object.remove(field);
        }
        let parsed: LiftRecord = serde_json::from_value(value).expect("legacy record");
        let rebuilt = crate::decision::PreRegistration::from_record(&parsed).expect("rebuilt");
        assert_eq!(rebuilt.randomization_unit, "user");
        assert_eq!(rebuilt.assignment_source, "apex");
        assert_eq!(rebuilt.assignment_epoch, "0");
        assert_eq!(rebuilt.estimator_id, "planned_share_total_v2");
        assert_eq!(rebuilt.outcome_source, "daily_ratio");
        assert_eq!(rebuilt.measurement_scope, rebuilt.scope, "default = scope");
        assert_eq!(rebuilt.eligibility, "eligible_users");
        assert_eq!(rebuilt.identity_contract, "idfv_idfa_user");
    }
}
