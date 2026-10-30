//! The record pipeline: turn engine inputs and outcomes into the six signed
//! record types, and feed finished results back into the priors. This is the
//! only place that maps live structures to ledger-ready payloads.

use crate::decision::{Decision, PreRegistration};
use crate::error::{LiftError, LiftResult};
use crate::guardrails::GuardrailEvent;
use crate::priors::{Bucket, LiftPriors};
use crate::records::{
    rows_sha256, BillingChangeRecord, BillingRecord, GuardrailRecord, LiftRecord,
};
use crate::stats::Estimate;
use crate::window::ArmDay;

/// The signed record with its canonical hash, ready for the ledger.
#[derive(Debug, Clone, PartialEq)]
pub struct EmittedRecord {
    pub record: LiftRecord,
    pub sha256: String,
}

fn emit(record: LiftRecord) -> LiftResult<EmittedRecord> {
    let sha256 = record.sha256_hex()?;
    Ok(EmittedRecord { record, sha256 })
}

/// 1. The pre-registration, written before the first platform call.
pub fn pre_registration(prereg: &PreRegistration) -> LiftResult<EmittedRecord> {
    prereg.validate().map_err(LiftError::Malformed)?;
    if prereg.instance_id.trim().is_empty() {
        return Err(LiftError::Malformed(
            "pre-registration must carry an immutable experiment instance id".into(),
        ));
    }
    emit(LiftRecord::PreRegistration {
        experiment_id: prereg.experiment_id.clone(),
        instance_id: prereg.instance_id.clone(),
        scope: prereg.scope.clone(),
        // Stored verbatim as a JSON string so the record round-trips byte for
        // byte: the prior platform value is pinned exactly as it was read.
        before_value: serde_json::Value::String(prereg.before_value.clone()),
        after_value: serde_json::json!({"change": prereg.change_description}),
        split: prereg.test_share,
        metric: "revenue_per_user".into(),
        burn_in_days: prereg.burn_in_days,
        window_days: prereg.window_days as u32,
        looks: if prereg.interim_look { 2 } else { 1 },
        extension_allowed: prereg.extension_allowed,
        guardrail_thresholds: serde_json::json!({
            "harmPoint": prereg.thresholds.harm_point,
            "fillRewarded": prereg.thresholds.fill_rewarded,
            "fillBanner": prereg.thresholds.fill_banner,
            "splitChiSquare": prereg.thresholds.split_chi_square,
        }),
        min_point_lift: prereg.min_point_lift,
        code_version_hash: prereg.code_version_hash.clone(),
        rate_bp: prereg.rate_bp,
        // The estimator contract: pinned from the values the registration
        // actually carries, so the signed bytes name the estimator, the
        // assignment and the measurement identity.
        randomization_unit: prereg.randomization_unit.clone(),
        assignment_source: prereg.assignment_source.clone(),
        assignment_epoch: prereg.assignment_epoch.clone(),
        estimator_id: prereg.estimator_id.clone(),
        outcome_source: prereg.outcome_source.clone(),
        measurement_scope: prereg.measurement_scope.clone(),
        eligibility: prereg.eligibility.clone(),
        identity_contract: prereg.identity_contract.clone(),
    })
}

/// 2. The daily data pull: the raw platform rows and their SHA-256.
pub fn data_pull(day: &str, rows: &serde_json::Value) -> LiftResult<EmittedRecord> {
    emit(LiftRecord::DataPull {
        day: day.to_string(),
        rows_sha256: rows_sha256(rows),
        rows: rows.clone(),
    })
}

/// 3. Guardrail events, each with its inputs.
pub fn guardrail_event(event: &GuardrailEvent) -> LiftResult<EmittedRecord> {
    let (guardrail, inputs, action) = match event {
        GuardrailEvent::FastHarm { point_lift, .. } => (
            "fastHarm",
            serde_json::json!({"pointLift": point_lift}),
            "rollback",
        ),
        GuardrailEvent::ConfirmedHarm { upper_bound, .. } => (
            "confirmedHarm",
            serde_json::json!({"upperBound": upper_bound}),
            "rollback",
        ),
        GuardrailEvent::FillRewardedInterstitial { delta, .. } => (
            "fillRewardedInterstitial",
            serde_json::json!({"delta": delta}),
            "rollback",
        ),
        GuardrailEvent::FillBanner { delta, .. } => (
            "fillBanner",
            serde_json::json!({"delta": delta}),
            "rollback",
        ),
        GuardrailEvent::PublisherEdit { .. } => ("publisherEdit", serde_json::json!({}), "void"),
        GuardrailEvent::SplitMismatch { chi_square, .. } => (
            "splitMismatch",
            serde_json::json!({"chiSquare": chi_square}),
            "void",
        ),
        GuardrailEvent::ReportingGap { stale_days, .. } => (
            "reportingGap",
            serde_json::json!({"staleDays": stale_days}),
            if *stale_days >= 3 { "void" } else { "pause" },
        ),
        GuardrailEvent::PlatformIncident { .. } => {
            ("platformIncident", serde_json::json!({}), "void")
        }
    };
    let day = match event {
        GuardrailEvent::FastHarm { day, .. }
        | GuardrailEvent::ConfirmedHarm { day, .. }
        | GuardrailEvent::FillRewardedInterstitial { day, .. }
        | GuardrailEvent::FillBanner { day, .. }
        | GuardrailEvent::PublisherEdit { day }
        | GuardrailEvent::SplitMismatch { day, .. }
        | GuardrailEvent::ReportingGap { day, .. }
        | GuardrailEvent::PlatformIncident { day } => day.clone(),
    };
    emit(LiftRecord::Guardrail(GuardrailRecord {
        day,
        guardrail: guardrail.to_string(),
        inputs,
        action: action.to_string(),
    }))
}

/// The result record: daily ratios, estimate, bounds, bootstrap and the
/// decision.
pub fn result(
    prereg: &PreRegistration,
    decision: &Decision,
    estimate: &Estimate,
    daily_log_ratios: Vec<f64>,
    bootstrap_lower: f64,
) -> LiftResult<EmittedRecord> {
    let (decision_label, lambda) = match decision {
        Decision::Keep { lambda, .. } => ("keep", *lambda),
        Decision::Rollback { .. } => ("rollback", 0.0),
        Decision::Void { .. } => ("void", 0.0),
        Decision::Pending { .. } => ("pending", 0.0),
        Decision::Extend { .. } => ("extend", 0.0),
    };
    emit(LiftRecord::Result {
        experiment_id: prereg.experiment_id.clone(),
        instance_id: prereg.instance_id.clone(),
        daily_log_ratios,
        mean_log: estimate.mean_log,
        lower_90: estimate.lower_90,
        lower_99: estimate.lower_99,
        bootstrap_lower,
        decision: decision_label.to_string(),
        lambda,
    })
}

/// 5. One monthly billing line.
pub fn monthly_billing(fee: &crate::billing::FeeLine, month: &str) -> LiftResult<EmittedRecord> {
    emit(LiftRecord::MonthlyBilling(BillingRecord {
        change_id: fee.change_id.clone(),
        month: month.to_string(),
        scope_revenue: fee.scope_revenue,
        treated_revenue: fee.treated_revenue,
        holdout: fee.holdout.clone(),
        lambda: fee.lambda,
        factor: fee.factor,
        incremental: fee.incremental,
        log_share: fee.log_share,
        rate_bp: fee.rate_bp,
        fee_micros: fee.fee_micros,
        billed_days: fee.billed_days,
        month_days: fee.month_days,
        pre_registration_hash: fee.pre_registration_hash.clone(),
        result_hash: fee.result_hash.clone(),
    }))
}

/// The durable 90/10 holdout state for one kept change instance.
pub fn holdout_state(state: &crate::holdout::HoldoutState) -> LiftResult<EmittedRecord> {
    emit(LiftRecord::HoldoutState(state.clone()))
}

/// 6. A billing change: override, re-check or term end.
pub fn billing_change(
    change_id: &str,
    day: &str,
    reason: &str,
    billed_lambda: f64,
) -> LiftResult<EmittedRecord> {
    emit(LiftRecord::BillingChange(BillingChangeRecord {
        change_id: change_id.to_string(),
        day: day.to_string(),
        reason: reason.to_string(),
        billed_lambda,
    }))
}

/// The prior pipeline: every finished test feeds the bucket model, keep or
/// rollback alike. Feeding only keeps would condition the model on the
/// decision and bias every later Thompson draw upward.
pub fn apply_decision_to_priors(
    priors: &mut LiftPriors,
    bucket: &Bucket,
    decision: &str,
    daily_log_ratios: &[f64],
) {
    if matches!(decision, "kept" | "rolled_back") {
        priors.observe_ratios(bucket, daily_log_ratios);
    }
}

/// One arm-day's signed evidence: the first observation and, once accepted,
/// the independent later observation with its own source hash.
fn day_evidence(day: &ArmDay) -> serde_json::Value {
    let first = match day.first_observation() {
        Some(observation) => serde_json::to_value(observation).unwrap_or(serde_json::Value::Null),
        None => serde_json::json!({
            "revenue": day.revenue,
            "users": day.users,
            "fill": day.fill,
            "observedAtUnixS": day.first_observed_at_unix_s,
            "sourceSha256": day.first_source_sha256,
        }),
    };
    let second = day
        .second_observation()
        .and_then(|observation| serde_json::to_value(observation).ok())
        .unwrap_or(serde_json::Value::Null);
    serde_json::json!({
        "day": day.day,
        "first": first,
        "second": second,
    })
}

/// The raw day rows for a data-pull record, from the window pulls. Every
/// arm-day carries both pull observations (revenue, users, fill, observed-at
/// and the raw source hash) so a third party can reproduce the finality
/// decision from the signed record alone.
pub fn day_rows(control: &[ArmDay], test: &[ArmDay]) -> serde_json::Value {
    serde_json::json!({
        "control": control.iter().map(day_evidence).collect::<Vec<_>>(),
        "test": test.iter().map(day_evidence).collect::<Vec<_>>(),
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use crate::decision::{GuardrailThresholds, Metric};

    fn prereg() -> PreRegistration {
        PreRegistration {
            experiment_id: "exp-1".into(),
            instance_id: "exp-1@1".into(),
            scope: "u1".into(),
            change_description: "raise t1 floor by 10%".into(),
            before_value: "{\"floor\": 1000000}".into(),
            test_share: 0.25,
            metric: Metric::RevenuePerUser,
            burn_in_days: 3,
            window_days: 14,
            interim_look: true,
            extension_allowed: true,
            code_version_hash: "abc123".into(),
            rate_bp: 2000,
            thresholds: GuardrailThresholds::default(),
            min_point_lift: 0.01,
            randomization_unit: crate::records::default_randomization_unit(),
            assignment_source: crate::records::default_assignment_source(),
            assignment_epoch: crate::records::default_assignment_epoch(),
            estimator_id: crate::records::default_estimator_id(),
            outcome_source: crate::records::default_outcome_source(),
            measurement_scope: "u1".into(),
            eligibility: crate::records::default_eligibility(),
            identity_contract: crate::records::default_identity_contract(),
        }
    }

    fn estimate() -> Estimate {
        Estimate {
            days: 14,
            mean_log: 0.02,
            lift: 0.0202,
            sd: 0.01,
            lower_90: 0.015,
            lower_99: 0.010,
            upper_90: 0.026,
            upper_99: 0.030,
            lower_alpha: 0.012,
            lower_interim: 0.010,
        }
    }

    #[test]
    fn the_six_record_types_emit_with_hashes() {
        let pre = pre_registration(&prereg()).expect("pre");
        assert_eq!(pre.sha256.len(), 64);
        let pull = data_pull("2026-10-04", &serde_json::json!([{"revenue": "1"}])).expect("pull");
        assert_eq!(pull.sha256.len(), 64);
        let guard =
            guardrail_event(&GuardrailEvent::PublisherEdit { day: "d".into() }).expect("guard");
        assert_eq!(guard.sha256.len(), 64);
        let decision = Decision::Keep {
            lambda: 0.015,
            estimate: estimate(),
            used_interim_99: false,
            bootstrap_lower: 0.014,
        };
        let result_record =
            result(&prereg(), &decision, &estimate(), vec![0.01, 0.03], 0.014).expect("result");
        assert_eq!(result_record.sha256.len(), 64);
        let fee = crate::billing::FeeLine {
            change_id: "c1".into(),
            scope_revenue: 1_000_000,
            treated_revenue: 1_000_000,
            holdout: None,
            lambda: 0.015,
            factor: 1.015,
            incremental: 15_000,
            log_share: 1.0,
            rate_bp: 2000,
            fee_micros: 3_000,
            billed_days: 0,
            month_days: 0,
            pre_registration_hash: "pre".into(),
            result_hash: "res".into(),
        };
        assert_eq!(
            monthly_billing(&fee, "2026-11")
                .expect("billing")
                .sha256
                .len(),
            64
        );
        // The signed monthly line preserves the exact signed record refs it
        // was built from, so the invoice line never loses its provenance.
        let billing = monthly_billing(&fee, "2026-11").expect("billing");
        let LiftRecord::MonthlyBilling(billing) = billing.record else {
            panic!("expected a monthly billing record");
        };
        assert_eq!(billing.pre_registration_hash, "pre");
        assert_eq!(billing.result_hash, "res");
        assert_eq!(
            billing_change("c1", "2026-12-01", "override", 0.0)
                .expect("change")
                .sha256
                .len(),
            64
        );
    }

    #[test]
    fn every_guardrail_event_emits_its_inputs() {
        let events = [
            GuardrailEvent::FastHarm {
                day: "d".into(),
                point_lift: -0.06,
            },
            GuardrailEvent::ConfirmedHarm {
                day: "d".into(),
                upper_bound: -0.01,
            },
            GuardrailEvent::FillRewardedInterstitial {
                day: "d".into(),
                delta: -0.03,
            },
            GuardrailEvent::FillBanner {
                day: "d".into(),
                delta: -0.06,
            },
            GuardrailEvent::PublisherEdit { day: "d".into() },
            GuardrailEvent::SplitMismatch {
                day: "d".into(),
                chi_square: 12.0,
            },
            GuardrailEvent::ReportingGap {
                day: "d".into(),
                stale_days: 2,
            },
            GuardrailEvent::ReportingGap {
                day: "d".into(),
                stale_days: 3,
            },
            GuardrailEvent::PlatformIncident { day: "d".into() },
        ];
        for event in events {
            let record = guardrail_event(&event).expect("record");
            assert_eq!(record.sha256.len(), 64, "{event:?}");
        }
    }

    #[test]
    fn pending_and_void_results_emit_zero_lambda() {
        let estimate = estimate();
        for decision in [
            Decision::Pending {
                reason: "waiting".into(),
            },
            Decision::Void {
                reason: "gap".into(),
            },
            Decision::Extend {
                reason: "week".into(),
            },
        ] {
            let record = result(&prereg(), &decision, &estimate, vec![0.01], 0.0).expect("record");
            assert_eq!(record.sha256.len(), 64);
        }
    }

    #[test]
    fn rollback_results_emit_the_rollback_label_with_zero_lambda() {
        let decision = Decision::Rollback {
            reason: "flat test".into(),
        };
        let record =
            result(&prereg(), &decision, &estimate(), vec![0.0, 0.001], 0.0).expect("record");
        match record.record {
            LiftRecord::Result {
                decision, lambda, ..
            } => {
                assert_eq!(decision, "rollback");
                assert_eq!(lambda, 0.0);
            }
            other => panic!("expected a result record, got {other:?}"),
        }
    }

    #[test]
    fn pre_registration_record_round_trips() {
        let original = prereg();
        let emitted = pre_registration(&original).expect("emit");
        let back = PreRegistration::from_record(&emitted.record).expect("from record");
        assert_eq!(back, original);
    }

    #[test]
    fn emission_pins_the_estimator_contract_from_the_registration() {
        let mut original = prereg();
        original.assignment_source = "publisher".into();
        original.assignment_epoch = "2026-10-29".into();
        original.estimator_id = "planned_share_total_v3".into();
        original.outcome_source = "user_revenue".into();
        original.measurement_scope = "u1/de".into();
        let emitted = pre_registration(&original).expect("emit");
        let LiftRecord::PreRegistration {
            randomization_unit,
            assignment_source,
            assignment_epoch,
            estimator_id,
            outcome_source,
            measurement_scope,
            eligibility,
            identity_contract,
            ..
        } = &emitted.record
        else {
            panic!("expected a pre-registration record");
        };
        assert_eq!(randomization_unit, "user");
        assert_eq!(assignment_source, "publisher");
        assert_eq!(assignment_epoch, "2026-10-29");
        assert_eq!(estimator_id, "planned_share_total_v3");
        assert_eq!(outcome_source, "user_revenue");
        assert_eq!(measurement_scope, "u1/de");
        assert_eq!(eligibility, "eligible_users");
        assert_eq!(identity_contract, "idfv_idfa_user");
        // The pinned values also rebuild into the registration the settle
        // path uses.
        let back = PreRegistration::from_record(&emitted.record).expect("from record");
        assert_eq!(back, original);
    }

    #[test]
    fn from_record_refuses_non_registration_records() {
        let pull = data_pull("2026-10-04", &serde_json::json!([])).expect("pull");
        assert!(PreRegistration::from_record(&pull.record).is_err());
    }

    #[test]
    fn invalid_registrations_never_emit() {
        let mut bad = prereg();
        bad.window_days = 10;
        assert!(pre_registration(&bad).is_err());
    }

    #[test]
    fn the_prior_pipeline_learns_from_keeps_only() {
        let bucket = Bucket {
            lever: "floor".into(),
            ad_format: "rewarded".into(),
            country_tier: "t1".into(),
            platform: "applovin_max".into(),
        };
        let mut priors = LiftPriors::default();
        let ratios = [0.03, 0.012, -0.004, 0.026, 0.041, 0.019, 0.008];
        let _rollback = Decision::Rollback {
            reason: "flat".into(),
        };
        apply_decision_to_priors(&mut priors, &bucket, "rolled_back", &ratios);
        assert_eq!(
            priors.total_observations(),
            1,
            "a finished rollback is a usable observation"
        );
        let _keep = Decision::Keep {
            lambda: 0.01,
            estimate: estimate(),
            used_interim_99: false,
            bootstrap_lower: 0.009,
        };
        apply_decision_to_priors(&mut priors, &bucket, "kept", &ratios);
        assert_eq!(priors.total_observations(), 2);
        let _pending = Decision::Pending {
            reason: "waiting".into(),
        };
        apply_decision_to_priors(&mut priors, &bucket, "pending", &ratios);
        assert_eq!(priors.total_observations(), 2, "pending teaches nothing");
    }

    fn evidence_day() -> ArmDay {
        ArmDay {
            day: "2026-10-01".into(),
            revenue: 1_000_000,
            users: Some(1_000),
            second_pull_revenue: Some(1_000_000),
            second_pull_users: Some(1_000),
            fill: Some(0.9),
            second_pull_fill: Some(0.9),
            first_observed_at_unix_s: Some(1_700_000_000),
            second_observed_at_unix_s: Some(1_700_086_400),
            first_source_sha256: Some(crate::window::arm_day_source_sha256(
                "2026-10-01",
                "control",
                1_000_000,
                Some(1_000),
                Some(0.9),
            )),
            second_source_sha256: Some(crate::window::arm_day_source_sha256(
                "2026-10-01",
                "control",
                1_000_000,
                Some(1_000),
                Some(0.9),
            )),
        }
    }

    #[test]
    fn day_rows_carry_both_pull_observations_and_their_source_hashes() {
        let day = evidence_day();
        let rows = day_rows(std::slice::from_ref(&day), std::slice::from_ref(&day));
        assert_eq!(rows["control"][0]["day"], "2026-10-01");
        assert_eq!(rows["control"][0]["first"]["revenue"], 1_000_000);
        assert_eq!(rows["control"][0]["first"]["users"], 1_000);
        assert_eq!(rows["control"][0]["first"]["fill"], 0.9);
        assert_eq!(
            rows["control"][0]["first"]["observedAtUnixS"],
            1_700_000_000_i64
        );
        assert_eq!(
            rows["control"][0]["first"]["sourceSha256"],
            day.first_source_sha256.clone().expect("first hash")
        );
        assert_eq!(rows["test"][0]["second"]["revenue"], 1_000_000);
        assert_eq!(rows["test"][0]["second"]["users"], 1_000);
        assert_eq!(rows["test"][0]["second"]["fill"], 0.9);
        assert_eq!(
            rows["test"][0]["second"]["observedAtUnixS"],
            1_700_086_400_i64
        );
        assert_eq!(
            rows["test"][0]["second"]["sourceSha256"],
            day.second_source_sha256.clone().expect("second hash")
        );
    }

    #[test]
    fn signed_data_pull_evidence_round_trips() {
        let day = evidence_day();
        let rows = day_rows(std::slice::from_ref(&day), std::slice::from_ref(&day));
        let emitted = data_pull("2026-10-01", &rows).expect("data pull");
        let json = emitted.record.canonical_json().expect("canonical");
        let back: LiftRecord = serde_json::from_str(&json).expect("round trip");
        let back_hash = back.sha256_hex().expect("hash");
        let LiftRecord::DataPull {
            day: signed_day,
            rows_sha256: signed_hash,
            rows: signed_rows,
        } = back
        else {
            panic!("expected a data-pull record");
        };
        assert_eq!(signed_day, "2026-10-01");
        assert_eq!(emitted.sha256, back_hash);
        assert_eq!(signed_hash, rows_sha256(&signed_rows));
        for arm in ["control", "test"] {
            let row = &signed_rows[arm][0];
            assert_eq!(
                row["first"]["sourceSha256"],
                day.first_source_sha256.clone().unwrap()
            );
            assert_eq!(
                row["second"]["sourceSha256"],
                day.second_source_sha256.clone().unwrap()
            );
            assert!(row["first"]["observedAtUnixS"].as_i64().is_some());
            assert!(row["second"]["observedAtUnixS"].as_i64().is_some());
        }
    }
}

#[cfg(test)]
mod from_record_tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use crate::decision::{GuardrailThresholds, Metric, PreRegistration};
    use crate::records::LiftRecord;

    fn prereg() -> PreRegistration {
        PreRegistration {
            experiment_id: "exp-1".into(),
            instance_id: "exp-1@1".into(),
            scope: "u1".into(),
            change_description: "raise EE".into(),
            before_value: "{\"floor\":1000000}".into(),
            test_share: 0.25,
            metric: Metric::RevenuePerUser,
            burn_in_days: 3,
            window_days: 7,
            interim_look: true,
            extension_allowed: true,
            code_version_hash: "sha256:x".into(),
            rate_bp: 2_000,
            thresholds: GuardrailThresholds::default(),
            min_point_lift: 0.01,
            randomization_unit: crate::records::default_randomization_unit(),
            assignment_source: crate::records::default_assignment_source(),
            assignment_epoch: crate::records::default_assignment_epoch(),
            estimator_id: crate::records::default_estimator_id(),
            outcome_source: crate::records::default_outcome_source(),
            measurement_scope: "u1".into(),
            eligibility: crate::records::default_eligibility(),
            identity_contract: crate::records::default_identity_contract(),
        }
    }

    #[test]
    fn from_record_reads_back_every_field() {
        let emitted = pre_registration(&prereg()).expect("emit");
        let back = PreRegistration::from_record(&emitted.record).expect("from record");
        assert_eq!(back, prereg());
    }

    #[test]
    fn from_record_refuses_every_malformed_registration() {
        let mut record = pre_registration(&prereg()).expect("emit").record;

        // Not a pre-registration at all.
        assert!(PreRegistration::from_record(&LiftRecord::Result {
            experiment_id: "e".into(),
            instance_id: "e@1".into(),
            daily_log_ratios: vec![0.01, 0.02],
            mean_log: 0.015,
            lower_90: 0.01,
            lower_99: 0.005,
            bootstrap_lower: 0.01,
            decision: "keep".into(),
            lambda: 0.1,
        })
        .is_err());

        // Unknown metric.
        if let LiftRecord::PreRegistration { metric, .. } = &mut record {
            *metric = "clicks".into();
        }
        assert!(PreRegistration::from_record(&record).is_err());

        // A registration that lost its change description.
        let mut missing_change = pre_registration(&prereg()).expect("emit").record;
        if let LiftRecord::PreRegistration { after_value, .. } = &mut missing_change {
            *after_value = serde_json::json!({});
        }
        assert!(PreRegistration::from_record(&missing_change).is_err());

        // A registration that lost a guardrail threshold.
        let mut missing_threshold = pre_registration(&prereg()).expect("emit").record;
        if let LiftRecord::PreRegistration {
            guardrail_thresholds,
            ..
        } = &mut missing_threshold
        {
            guardrail_thresholds
                .as_object_mut()
                .unwrap()
                .remove("fillBanner");
        }
        assert!(PreRegistration::from_record(&missing_threshold).is_err());
    }
}
