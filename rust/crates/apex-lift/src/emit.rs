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
    emit(LiftRecord::PreRegistration {
        experiment_id: prereg.experiment_id.clone(),
        scope: prereg.scope.clone(),
        before_value: serde_json::from_str(&prereg.before_value)
            .unwrap_or(serde_json::Value::String(prereg.before_value.clone())),
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
        code_version_hash: prereg.code_version_hash.clone(),
        rate_bp: prereg.rate_bp,
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
        lambda: fee.lambda,
        factor: fee.factor,
        incremental: fee.incremental,
        rate_bp: fee.rate_bp,
        fee_micros: fee.fee_micros,
    }))
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
    decision: &Decision,
    daily_log_ratios: &[f64],
) {
    if matches!(decision, Decision::Keep { .. } | Decision::Rollback { .. }) {
        priors.observe_ratios(bucket, daily_log_ratios);
    }
}

/// The raw day rows for a data-pull record, from the window pulls.
pub fn day_rows(control: &[ArmDay], test: &[ArmDay]) -> serde_json::Value {
    serde_json::json!({
        "control": control.iter().map(|d| serde_json::json!({
            "day": d.day, "revenue": d.revenue, "users": d.users, "fill": d.fill
        })).collect::<Vec<_>>(),
        "test": test.iter().map(|d| serde_json::json!({
            "day": d.day, "revenue": d.revenue, "users": d.users, "fill": d.fill
        })).collect::<Vec<_>>(),
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
            lambda: 0.015,
            factor: 1.015,
            incremental: 15_000,
            log_share: 1.0,
            rate_bp: 2000,
            fee_micros: 3_000,
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
        let rollback = Decision::Rollback {
            reason: "flat".into(),
        };
        apply_decision_to_priors(&mut priors, &bucket, &rollback, &ratios);
        assert_eq!(
            priors.total_observations(),
            1,
            "a finished rollback is a usable observation"
        );
        let keep = Decision::Keep {
            lambda: 0.01,
            estimate: estimate(),
            used_interim_99: false,
            bootstrap_lower: 0.009,
        };
        apply_decision_to_priors(&mut priors, &bucket, &keep, &ratios);
        assert_eq!(priors.total_observations(), 2);
        let pending = Decision::Pending {
            reason: "waiting".into(),
        };
        apply_decision_to_priors(&mut priors, &bucket, &pending, &ratios);
        assert_eq!(priors.total_observations(), 2, "pending teaches nothing");
    }

    #[test]
    fn day_rows_carry_both_arms() {
        let control = vec![ArmDay {
            day: "2026-10-01".into(),
            revenue: 1,
            users: None,
            second_pull_revenue: None,
            second_pull_users: None,
            fill: Some(0.9),
        }];
        let rows = day_rows(&control, &control);
        assert_eq!(rows["control"][0]["day"], "2026-10-01");
        assert_eq!(rows["test"][0]["fill"], 0.9);
    }
}
