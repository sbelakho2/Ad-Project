//! The daily guardrails. They can only stop a test, never keep one, which is
//! why checking them every day does not inflate billing errors. Thresholds
//! come from the pre-registration and can only be tightened from the spec
//! defaults.

use crate::decision::PreRegistration;
use crate::stats::estimate;
use crate::window::AnalysisDay;

/// A guardrail that fired, with the day and the inputs that fired it.
#[derive(Debug, Clone, PartialEq)]
pub enum GuardrailEvent {
    /// Point estimate below the registered harm threshold after at least two
    /// analysis days.
    FastHarm { day: String, point_lift: f64 },
    /// The 90% upper bound of lift is below zero.
    ConfirmedHarm { day: String, upper_bound: f64 },
    /// Rewarded or interstitial fill past the registered threshold below
    /// control, two consecutive days running.
    FillRewardedInterstitial { day: String, delta: f64 },
    /// Banner fill past the registered threshold below control, two
    /// consecutive days running.
    FillBanner { day: String, delta: f64 },
    /// The daily settings snapshot changed the ad unit by anyone but Apex.
    PublisherEdit { day: String },
    /// LevelPlay arm counts deviate from the planned split past the
    /// registered chi-square threshold.
    SplitMismatch { day: String, chi_square: f64 },
    /// Days still not final after 72 hours. One or two pause the window;
    /// three or more void it.
    ReportingGap { day: String, stale_days: u32 },
    /// Both arms' revenue fell more than 50% for two consecutive days.
    PlatformIncident { day: String },
}

impl GuardrailEvent {
    /// Whether the event voids the test (no decision, no fee) rather than
    /// rolling it back.
    pub fn voids(&self) -> bool {
        match self {
            GuardrailEvent::PublisherEdit { .. }
            | GuardrailEvent::SplitMismatch { .. }
            | GuardrailEvent::PlatformIncident { .. } => true,
            GuardrailEvent::ReportingGap { stale_days, .. } => *stale_days >= 3,
            _ => false,
        }
    }

    /// Whether the event is a hard rollback (harm found).
    pub fn rolls_back(&self) -> bool {
        matches!(
            self,
            GuardrailEvent::FastHarm { .. }
                | GuardrailEvent::ConfirmedHarm { .. }
                | GuardrailEvent::FillRewardedInterstitial { .. }
                | GuardrailEvent::FillBanner { .. }
        )
    }

    /// Whether the event pauses the window while it resolves (fewer than
    /// three stale days).
    pub fn pauses(&self) -> bool {
        match self {
            GuardrailEvent::ReportingGap { stale_days, .. } => *stale_days < 3,
            _ => false,
        }
    }
}

/// Fill formats the guardrails watch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FillFormat {
    Rewarded,
    Interstitial,
    Banner,
}

/// Evaluate every guardrail over the analyzed days that are final.
///
/// `arm_counts` holds `(test_users, control_users)` per analyzed day for the
/// split check (LevelPlay always; MAX when postbacks are on). `settings_changed`
/// flags days whose snapshot the publisher edited. `stale_days` counts days
/// still not final after 72 hours. `incident_runs` lists `(day, run_length)`
/// for incident days; only runs of two or more days void.
#[allow(clippy::too_many_arguments)]
pub fn evaluate(
    prereg: &PreRegistration,
    days: &[AnalysisDay],
    arm_counts: &[(u64, u64)],
    fill_format: FillFormat,
    settings_changed: &[String],
    stale_days: u32,
    incident_runs: &[(String, u32)],
) -> Vec<GuardrailEvent> {
    evaluate_with_unique_players(
        prereg,
        days,
        None,
        arm_counts,
        fill_format,
        settings_changed,
        stale_days,
        incident_runs,
    )
}

/// Evaluate every guardrail with the L3 unique-player split evidence when it
/// exists. `unique_players` is `(test, control)` over **unique** player ids;
/// `arm_counts` is only the fallback for adapters that report no player ids.
/// When unique counts are supplied they are the only split denominator, so
/// repeated player-days can never cancel or amplify a split mismatch.
#[allow(clippy::too_many_arguments)]
pub fn evaluate_with_unique_players(
    prereg: &PreRegistration,
    days: &[AnalysisDay],
    unique_players: Option<(u64, u64)>,
    arm_counts: &[(u64, u64)],
    fill_format: FillFormat,
    settings_changed: &[String],
    stale_days: u32,
    incident_runs: &[(String, u32)],
) -> Vec<GuardrailEvent> {
    let mut events = Vec::new();
    let thresholds = &prereg.thresholds;

    // Fast harm and confirmed harm run over the accumulated window as it
    // exists today; they never look forward.
    for cutoff in 2..=days.len() {
        let ratios: Vec<f64> = days[..cutoff].iter().map(|d| d.log_ratio).collect();
        let Some(current) = estimate(&ratios) else {
            continue;
        };
        let day = days[cutoff - 1].day.clone();
        if current.lift < thresholds.harm_point {
            events.push(GuardrailEvent::FastHarm {
                day: day.clone(),
                point_lift: current.lift,
            });
        }
        // A3 L4: confirmed harm only at the registered day-7 (99%) and
        // day-14 (90%) looks; the fast threshold every day remains.
        let confirmed =
            (cutoff == 7 && current.upper_99 < 0.0) || (cutoff == 14 && current.upper_90 < 0.0);
        if confirmed {
            events.push(GuardrailEvent::ConfirmedHarm {
                day: day.clone(),
                upper_bound: if cutoff == 7 {
                    current.upper_99
                } else {
                    current.upper_90
                },
            });
        }
    }

    // Fill guardrails: two consecutive calendar days running past the
    // threshold. A gap in the analyzed sequence resets the streak, so two
    // non-adjacent days can never fire the stop.
    let threshold = match fill_format {
        FillFormat::Rewarded | FillFormat::Interstitial => thresholds.fill_rewarded,
        FillFormat::Banner => thresholds.fill_banner,
    };
    let mut streak = 0usize;
    let mut previous_index: Option<usize> = None;
    for d in days {
        let consecutive = previous_index
            .map(|previous| d.calendar_index == previous + 1)
            .unwrap_or(false);
        previous_index = Some(d.calendar_index);
        match d.fill_delta {
            Some(delta) if delta < -threshold => {
                streak = if consecutive { streak + 1 } else { 1 };
                if streak >= 2 {
                    let event = match fill_format {
                        FillFormat::Rewarded | FillFormat::Interstitial => {
                            GuardrailEvent::FillRewardedInterstitial {
                                day: d.day.clone(),
                                delta,
                            }
                        }
                        FillFormat::Banner => GuardrailEvent::FillBanner {
                            day: d.day.clone(),
                            delta,
                        },
                    };
                    events.push(event);
                }
            }
            _ => streak = 0,
        }
    }

    for day in settings_changed {
        events.push(GuardrailEvent::PublisherEdit { day: day.clone() });
    }

    // Split mismatch. The caller passes unique-player counts when the
    // per-player feed is available (L3); the legacy per-day pair list is only
    // a fallback for adapters with no player labels. Summed player-days are
    // never the primary evidence because a player seen on several days would
    // otherwise count several times and distort the denominator.
    let split = match unique_players {
        Some((test, control)) => Some((test, control)),
        None => {
            let pooled = arm_counts
                .iter()
                .fold((0u64, 0u64), |(t, c), (dt, dc)| (t + dt, c + dc));
            (pooled.0 + pooled.1 > 0).then_some(pooled)
        }
    };
    if let Some((test_players, control_players)) = split {
        if let Some(chi_square) = split_chi_square(test_players, control_players, prereg.test_share)
        {
            if chi_square > thresholds.split_chi_square {
                let day = days.last().map(|d| d.day.clone()).unwrap_or_default();
                events.push(GuardrailEvent::SplitMismatch { day, chi_square });
            }
        }
    }

    if stale_days > 0 {
        let day = days.last().map(|d| d.day.clone()).unwrap_or_default();
        events.push(GuardrailEvent::ReportingGap { day, stale_days });
    }
    // Only runs of two or more consecutive incident days void; a single
    // incident day was already excluded from the window.
    for (day, run) in incident_runs {
        if *run >= 2 {
            events.push(GuardrailEvent::PlatformIncident { day: day.clone() });
        }
    }
    events
}

/// The split chi-square on unique players at df 1. `None` for a degenerate
/// planned share or an empty population. The threshold for p = 0.001 is
/// [`CHI_SQUARE_DF1_P001`]; callers compare against the registered threshold,
/// which may only be tighter.
#[must_use]
pub fn split_chi_square(
    test_players: u64,
    control_players: u64,
    planned_share: f64,
) -> Option<f64> {
    if !planned_share.is_finite() || !(0.0..1.0).contains(&planned_share) {
        return None;
    }
    let total = test_players as f64 + control_players as f64;
    if total <= 0.0 {
        return None;
    }
    let expected_test = total * planned_share;
    let expected_control = total * (1.0 - planned_share);
    if expected_test <= 0.0 || expected_control <= 0.0 {
        return None;
    }
    let chi_square = (test_players as f64 - expected_test).powi(2) / expected_test
        + (control_players as f64 - expected_control).powi(2) / expected_control;
    chi_square.is_finite().then_some(chi_square)
}

/// A3 L8: a MAX A/A label-free check voids the unit when its mean log
/// ratio is more than 3.3 of its own standard errors from zero.
pub const AA_SPLIT_Z: f64 = 3.3;

/// Whether an A/A run's label-free log ratios void the unit.
#[must_use]
pub fn aa_split_void(ratios: &[f64]) -> bool {
    let Some(estimate) = estimate(ratios) else {
        return true;
    };
    let se = estimate.sd / (estimate.days as f64).sqrt();
    !se.is_finite() || estimate.mean_log.abs() > AA_SPLIT_Z * se
}

/// Chi-square df 1 at p = 0.001: 10.8276, the default split threshold.
pub const CHI_SQUARE_DF1_P001: f64 = 10.8276;

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use crate::decision::{GuardrailThresholds, Metric, PreRegistration};

    fn analysis_day(day: &str, ratio: f64, fill_delta: Option<f64>, index: usize) -> AnalysisDay {
        AnalysisDay {
            day: day.to_string(),
            test_value: ratio.exp(),
            control_value: 1.0,
            log_ratio: ratio,
            fill_delta,
            calendar_index: index,
            test_users: None,
            control_users: None,
        }
    }

    fn prereg(share: f64) -> PreRegistration {
        PreRegistration {
            experiment_id: "exp".into(),
            instance_id: "exp@1".into(),
            scope: "u1".into(),
            change_description: "raise t1 floor".into(),
            before_value: "{\"floor\": 1000000}".into(),
            test_share: share,
            metric: Metric::RevenuePerUser,
            burn_in_days: 3,
            window_days: 14,
            interim_look: true,
            extension_allowed: true,
            code_version_hash: "test".into(),
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

    #[test]
    fn fast_harm_is_registered_threshold_and_confirmed_harm_bounds() {
        let days = vec![
            analysis_day("2026-10-01", -0.04, None, 0),
            analysis_day("2026-10-02", -0.07, None, 1),
        ];
        let events = evaluate(&prereg(0.5), &days, &[], FillFormat::Rewarded, &[], 0, &[]);
        assert!(events
            .iter()
            .any(|e| matches!(e, GuardrailEvent::FastHarm { .. })));
        assert!(!events
            .iter()
            .any(|e| matches!(e, GuardrailEvent::ConfirmedHarm { .. })));
        let fortnight: Vec<_> = (0..14)
            .map(|index| analysis_day("2026-10-01", -0.07, None, index))
            .collect();
        assert!(evaluate(
            &prereg(0.5),
            &fortnight,
            &[],
            FillFormat::Rewarded,
            &[],
            0,
            &[]
        )
        .iter()
        .any(|e| matches!(e, GuardrailEvent::ConfirmedHarm { .. })));

        // A tighter -2% threshold fires earlier; the default -5% does not.
        // Cumulative mean -2.5%: above the default -5% stop, past a tighter
        // -2% stop.
        let mild = vec![
            analysis_day("2026-10-01", -0.01, None, 0),
            analysis_day("2026-10-02", -0.04, None, 1),
        ];
        assert!(
            evaluate(&prereg(0.5), &mild, &[], FillFormat::Rewarded, &[], 0, &[])
                .iter()
                .all(|e| !matches!(e, GuardrailEvent::FastHarm { .. }))
        );
        let mut tighter = prereg(0.5);
        tighter.thresholds.harm_point = -0.02;
        assert!(
            evaluate(&tighter, &mild, &[], FillFormat::Rewarded, &[], 0, &[])
                .iter()
                .any(|e| matches!(e, GuardrailEvent::FastHarm { .. }))
        );
    }

    #[test]
    fn healthy_test_fires_nothing() {
        let days: Vec<AnalysisDay> = (0..14)
            .map(|i| analysis_day(&format!("2026-10-{i:02}"), 0.02, Some(0.0), i))
            .collect();
        let events = evaluate(&prereg(0.5), &days, &[], FillFormat::Rewarded, &[], 0, &[]);
        assert!(events.is_empty(), "{events:?}");
    }

    #[test]
    fn fill_guardrails_need_two_consecutive_days() {
        let two_days = vec![
            analysis_day("2026-10-01", 0.01, Some(-0.03), 0),
            analysis_day("2026-10-02", 0.01, Some(-0.025), 1),
        ];
        assert!(evaluate(
            &prereg(0.5),
            &two_days,
            &[],
            FillFormat::Rewarded,
            &[],
            0,
            &[]
        )
        .iter()
        .any(|e| matches!(e, GuardrailEvent::FillRewardedInterstitial { .. })));

        // A gap between the two bad days resets the streak.
        let gapped = vec![
            analysis_day("2026-10-01", 0.01, Some(-0.03), 0),
            analysis_day("2026-10-03", 0.01, Some(-0.03), 2),
        ];
        assert!(evaluate(
            &prereg(0.5),
            &gapped,
            &[],
            FillFormat::Rewarded,
            &[],
            0,
            &[]
        )
        .is_empty());

        // Banner is more tolerant by default but registered tighter.
        let banner_bad = vec![
            analysis_day("2026-10-01", 0.01, Some(-0.06), 0),
            analysis_day("2026-10-02", 0.01, Some(-0.06), 1),
        ];
        let mut banner = prereg(0.5);
        banner.thresholds.fill_banner = 0.03;
        assert!(
            evaluate(&banner, &banner_bad, &[], FillFormat::Banner, &[], 0, &[])
                .iter()
                .any(|e| matches!(e, GuardrailEvent::FillBanner { .. }))
        );
    }

    #[test]
    fn a_label_free_aa_drift_beyond_three_point_three_sigma_voids() {
        assert!(!aa_split_void(&[0.001, -0.001, 0.002, -0.002, 0.0, 0.001]));
        assert!(aa_split_void(&[0.2, 0.21, 0.19, 0.2, 0.205, 0.195]));
        assert!(aa_split_void(&[]));
    }

    #[test]
    fn split_mismatch_uses_the_registered_threshold() {
        let days = vec![analysis_day("2026-10-01", 0.01, None, 0)];
        let events = evaluate(
            &prereg(0.25),
            &days,
            &[(3000, 7000)],
            FillFormat::Rewarded,
            &[],
            0,
            &[],
        );
        assert!(events
            .iter()
            .any(|e| matches!(e, GuardrailEvent::SplitMismatch { .. })));
        let ok = evaluate(
            &prereg(0.25),
            &days,
            &[(2500, 7500)],
            FillFormat::Rewarded,
            &[],
            0,
            &[],
        );
        assert!(ok.is_empty());
        // A tighter chi-square threshold fires on a milder imbalance.
        let mut tighter = prereg(0.25);
        tighter.thresholds.split_chi_square = 8.0;
        let mild = evaluate(
            &tighter,
            &days,
            &[(2700, 7300)],
            FillFormat::Rewarded,
            &[],
            0,
            &[],
        );
        assert!(mild
            .iter()
            .any(|e| matches!(e, GuardrailEvent::SplitMismatch { .. })));
    }

    #[test]
    fn split_check_uses_unique_players_not_summed_player_days() {
        // Unique split: 60 test / 40 control at a planned 50% -> chi-square 4,
        // under the p=.001 threshold. But the test players were seen twice and
        // the control players once, so the summed player-day fallback sees
        // 120/40 and would void. Unique evidence must win.
        assert!((split_chi_square(60, 40, 0.5).expect("chi") - 4.0).abs() < 1e-12);
        let days = vec![analysis_day("2026-10-01", 0.01, None, 0)];
        let unique = evaluate_with_unique_players(
            &prereg(0.5),
            &days,
            Some((60, 40)),
            &[(120, 40)],
            FillFormat::Rewarded,
            &[],
            0,
            &[],
        );
        assert!(
            unique
                .iter()
                .all(|e| !matches!(e, GuardrailEvent::SplitMismatch { .. })),
            "unique players pass: {unique:?}"
        );
        let fallback = evaluate(
            &prereg(0.5),
            &days,
            &[(120, 40)],
            FillFormat::Rewarded,
            &[],
            0,
            &[],
        );
        assert!(
            fallback
                .iter()
                .any(|e| matches!(e, GuardrailEvent::SplitMismatch { .. })),
            "the summed fallback demonstrates the old distortion"
        );
    }

    #[test]
    fn split_p_value_is_point_zero_zero_one_on_unique_players() {
        // 30/70 at a planned 25%: chi-square 1.33, below 10.8276.
        assert!(split_chi_square(30, 70, 0.25).expect("chi") < CHI_SQUARE_DF1_P001);
        // 40/60 at a planned 25%: chi-square 12.0, above the p=.001 threshold.
        assert!(split_chi_square(40, 60, 0.25).expect("chi") > CHI_SQUARE_DF1_P001);
        assert!(split_chi_square(10, 10, 0.0).is_none());
        assert!(split_chi_square(0, 0, 0.5).is_none());
        assert!(split_chi_square(10, 10, f64::NAN).is_none());
    }

    #[test]
    fn reporting_gap_pauses_then_voids() {
        let days = vec![analysis_day("2026-10-01", 0.01, None, 0)];
        for (stale, expect_void, expect_pause) in
            [(1u32, false, true), (2, false, true), (3, true, false)]
        {
            let events = evaluate(
                &prereg(0.5),
                &days,
                &[],
                FillFormat::Rewarded,
                &[],
                stale,
                &[],
            );
            let gap = events
                .iter()
                .find(|e| matches!(e, GuardrailEvent::ReportingGap { .. }))
                .expect("gap event");
            assert_eq!(gap.voids(), expect_void, "stale {stale}");
            assert_eq!(gap.pauses(), expect_pause, "stale {stale}");
        }
    }

    #[test]
    fn incident_runs_helper_marks_consecutive_days() {
        let names = [
            "2026-10-01",
            "2026-10-02",
            "2026-10-03",
            "2026-10-04",
            "2026-10-05",
        ];
        let mut control: Vec<crate::window::ArmDay> = names
            .iter()
            .map(|n| crate::window::ArmDay {
                day: n.to_string(),
                revenue: 1_000_000,
                users: None,
                second_pull_revenue: Some(1_000_000),
                second_pull_users: None,
                fill: None,
                second_pull_fill: None,
                first_observed_at_unix_s: Some(1_700_000_000),
                second_observed_at_unix_s: Some(1_700_086_400),
                first_source_sha256: Some("a".repeat(64)),
                second_source_sha256: Some("b".repeat(64)),
            })
            .collect();
        let mut test = control.clone();
        control[3].revenue = 400_000;
        test[3].revenue = 400_000;
        control[4].revenue = 100_000;
        test[4].revenue = 100_000;
        let runs = crate::window::incident_runs(&control, &test);
        assert_eq!(
            runs,
            vec![("2026-10-04".to_string(), 1), ("2026-10-05".to_string(), 2)]
        );
    }

    #[test]
    fn single_incident_day_never_voids() {
        let days = vec![analysis_day("2026-10-01", 0.01, None, 0)];
        let single = evaluate(
            &prereg(0.5),
            &days,
            &[],
            FillFormat::Rewarded,
            &[],
            0,
            &[("2026-10-01".to_string(), 1)],
        );
        assert!(single
            .iter()
            .all(|e| !matches!(e, GuardrailEvent::PlatformIncident { .. })));
        let double = evaluate(
            &prereg(0.5),
            &days,
            &[],
            FillFormat::Rewarded,
            &[],
            0,
            &[("2026-10-02".to_string(), 2)],
        );
        assert!(double
            .iter()
            .any(|e| matches!(e, GuardrailEvent::PlatformIncident { .. })));
    }

    #[test]
    fn voids_and_rollbacks_classify_correctly() {
        assert!(GuardrailEvent::PublisherEdit { day: "d".into() }.voids());
        assert!(GuardrailEvent::SplitMismatch {
            day: "d".into(),
            chi_square: 11.0
        }
        .voids());
        assert!(GuardrailEvent::PlatformIncident { day: "d".into() }.voids());
        assert!(GuardrailEvent::FastHarm {
            day: "d".into(),
            point_lift: -0.06
        }
        .rolls_back());
        assert!(!GuardrailEvent::ReportingGap {
            day: "d".into(),
            stale_days: 1
        }
        .voids());
    }

    #[test]
    fn non_gap_events_never_pause_or_void() {
        assert!(!GuardrailEvent::FastHarm {
            day: "d".into(),
            point_lift: -0.2
        }
        .pauses());
        assert!(!GuardrailEvent::PublisherEdit { day: "d".into() }.pauses());
        assert!(!GuardrailEvent::PlatformIncident { day: "d".into() }.pauses());
        assert!(!GuardrailEvent::ReportingGap {
            day: "d".into(),
            stale_days: 3
        }
        .pauses());
        assert!(!GuardrailEvent::FastHarm {
            day: "d".into(),
            point_lift: -0.2
        }
        .voids());
    }

    #[test]
    fn settings_changes_are_reported_and_nan_days_are_skipped() {
        // A NaN ratio cannot produce an estimate; the day is skipped in the
        // harm loop while the publisher edit is still reported.
        let days = vec![
            analysis_day("2026-10-01", f64::NAN, None, 0),
            analysis_day("2026-10-02", 0.01, None, 1),
        ];
        let events = evaluate(
            &prereg(0.5),
            &days,
            &[],
            FillFormat::Rewarded,
            &["2026-10-02".to_string()],
            0,
            &[],
        );
        assert!(events
            .iter()
            .any(|e| matches!(e, GuardrailEvent::PublisherEdit { day } if day == "2026-10-02")));
        assert!(events
            .iter()
            .all(|e| !matches!(e, GuardrailEvent::FastHarm { .. })));
    }
}
