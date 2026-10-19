//! Experiments `/control/experiments`, `/control/experiments/{id}` and the
//! effectiveness view (pages 61, 62 and 67).
//!
//! The decision review renders the engine's own outputs: the observed lift
//! and its interval and bootstrap fields come from the recorded decision
//! records, never recomputed here, and the page states plainly that an
//! operator cannot type a better result.

use apex_analytics::queries::{ExperimentEffectiveness, ExperimentLandscapeRow, LiftRow, Money};
use apex_ui::components::{data_table, notice, Cell, EmptyState};
use apex_ui::escape::html_escape;
use apex_ui::status::StatusKind;

use crate::copy;
use crate::data::{DecisionCheck, DurableExperiment};

use super::{document, header, kpis, PageMeta};

/// The fleet page data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExperimentsData {
    /// The fleet landscape.
    pub landscape: Result<Vec<ExperimentLandscapeRow>, String>,
    /// The effectiveness cohort, when `?view=effectiveness`.
    pub effectiveness: Option<Result<ExperimentEffectiveness, String>>,
    /// The cohort selector months from the dataset.
    pub cohorts: Vec<String>,
    /// The selected cohort month.
    pub selected_cohort: Option<String>,
    /// At least one decision is due.
    pub decisions_due: bool,
}

/// One fleet KPI computed from the landscape rows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FleetKpis {
    /// Running count and oldest age.
    pub running: (u64, Option<i64>),
    /// Awaiting-data count, reason and age.
    pub awaiting_data: (u64, Option<String>, Option<i64>),
    /// Decisions-due count, reason and age.
    pub decisions_due: (u64, Option<String>, Option<i64>),
    /// Restore-failed count, reason and age.
    pub restore_failed: (u64, Option<String>, Option<i64>),
}

/// Compute the fleet KPIs from the landscape rows (the row population of
/// each KPI is exactly the rows counted).
pub fn fleet_kpis(rows: &[ExperimentLandscapeRow]) -> FleetKpis {
    let group = |state: &str| -> (u64, Option<String>, Option<i64>) {
        let matching: Vec<&ExperimentLandscapeRow> =
            rows.iter().filter(|row| row.state == state).collect();
        (
            matching.iter().map(|row| row.count).sum(),
            matching
                .iter()
                .find_map(|row| row.oldest_waiting_reason.clone()),
            matching.iter().filter_map(|row| row.oldest_age_days).max(),
        )
    };
    let running = group("running");
    FleetKpis {
        running: (running.0, running.2),
        awaiting_data: group("awaiting_data"),
        decisions_due: group("decisions_due"),
        restore_failed: group("restore_failed"),
    }
}

/// Render the fleet page (and the effectiveness view).
pub fn render_fleet(meta: &PageMeta, data: &ExperimentsData) -> String {
    let mut body = header(meta, "Experiment health");
    match &data.landscape {
        Err(reason) => {
            body.push_str(&notice(
                StatusKind::Unavailable,
                copy::TELEMETRY_ABSENT,
                &format!("The experiment landscape is not readable: {reason}"),
                Some((copy::TELEMETRY_ABSENT_ACTION, "/control/system")),
            ));
        }
        Ok(rows) => {
            let kpis = fleet_kpis(rows);
            body.push_str(&kpis_html(&kpis));
            if let Some(effectiveness) = &data.effectiveness {
                body.push_str(&render_effectiveness(
                    meta,
                    effectiveness,
                    &data.cohorts,
                    data.selected_cohort.as_deref(),
                ));
            } else {
                body.push_str(&render_landscape(rows, data.decisions_due));
            }
        }
    }
    document(meta, "Experiments", &["Experiments"], &body)
}

fn kpis_html(kpis: &FleetKpis) -> String {
    super::kpis(&[
        (
            "Running",
            &kpis.running.0.to_string(),
            &format!(
                "Oldest {}",
                kpis.running
                    .1
                    .map(|days| format!("{days}d"))
                    .unwrap_or_else(|| "0d".to_string())
            ),
            None,
        ),
        (
            "Awaiting data",
            &kpis.awaiting_data.0.to_string(),
            &kpis
                .awaiting_data
                .1
                .clone()
                .map(|reason| format!("Waiting because {reason}"))
                .unwrap_or_else(|| "Fresh state".to_string()),
            None,
        ),
        (
            "Decisions due",
            &kpis.decisions_due.0.to_string(),
            &kpis
                .decisions_due
                .1
                .clone()
                .unwrap_or_else(|| "No decisions waiting".to_string()),
            Some("/control/experiments"),
        ),
        (
            "Restore failed",
            &kpis.restore_failed.0.to_string(),
            &kpis
                .restore_failed
                .1
                .clone()
                .unwrap_or_else(|| "No failed restores".to_string()),
            Some("/control/incidents"),
        ),
    ])
}

fn render_landscape(rows: &[ExperimentLandscapeRow], decisions_due: bool) -> String {
    let cells: Vec<Vec<Cell>> = rows
        .iter()
        .map(|row| {
            vec![
                Cell::Status(row.state.replace('_', " "), state_kind(&row.state)),
                Cell::Num(row.count.to_string()),
                Cell::Text(
                    row.oldest_waiting_reason
                        .clone()
                        .unwrap_or_else(|| "None waiting".to_string()),
                ),
                Cell::Num(
                    row.oldest_age_days
                        .map(|days| format!("{days}d"))
                        .unwrap_or_else(|| "-".to_string()),
                ),
            ]
        })
        .collect();
    let mut out = data_table(
        Some("Fleet by state"),
        &["State", "Count", "Waiting reason / blocker", "Oldest age"],
        &cells,
        Some(EmptyState {
            reason: copy::AWAITING_DATA.to_string(),
            action: Some((copy::AWAITING_DATA_ACTION.to_string(), "/control/ingestion".to_string())),
        }),
        Some("Waiting reasons are shown per state; the elapsed window alone does not establish a result."),
    );
    if decisions_due {
        out.push_str(&notice(
            StatusKind::Attention,
            "A decision window is complete.",
            copy::AWAITING_DATA,
            Some((copy::AWAITING_DATA_ACTION, "/control/experiments")),
        ));
    }
    out.push_str(
        "<p class=\"panel-body\"><a href=\"/control/experiments?view=effectiveness\">Open effectiveness (read the denominator)</a></p>",
    );
    out.push_str(&format!(
        "<p class=\"table-note\">{}</p>",
        html_escape(copy::NO_MANUAL_KEEP)
    ));
    out
}

fn state_kind(state: &str) -> StatusKind {
    match state {
        "running" => StatusKind::InProgress,
        "awaiting_data" => StatusKind::Attention,
        "decisions_due" => StatusKind::Attention,
        "restore_failed" => StatusKind::Failure,
        _ => StatusKind::Neutral,
    }
}

fn render_effectiveness(
    meta: &PageMeta,
    effectiveness: &Result<ExperimentEffectiveness, String>,
    cohorts: &[String],
    selected: Option<&str>,
) -> String {
    let mut out = String::new();
    out.push_str(&cohort_filter(cohorts, selected));
    match effectiveness {
        Err(reason) => {
            out.push_str(&notice(
                StatusKind::Unavailable,
                "The effectiveness cohort is not available.",
                reason,
                None,
            ));
        }
        Ok(cohort) if !cohort.mature => {
            out.push_str(&notice(
                StatusKind::Attention,
                copy::AWAITING_DATA,
                "The cohort has not cleared the 30-day follow-up horizon, so its rates are not compared.",
                None,
            ));
        }
        Ok(cohort) => {
            out.push_str("<section class=\"panel\">");
            out.push_str(&format!(
                "<h2 class=\"panel-heading\">{}</h2>",
                html_escape(copy::READ_THE_DENOMINATOR)
            ));
            out.push_str(&bars(cohort));
            out.push_str(&format!(
                "<p class=\"panel-body\">{} of {} valid settled tests kept: {} / {} of {} started tests kept: {}</p>",
                cohort.kept,
                cohort.valid_settled,
                apex_ui::format::percent(keep_rate(cohort.kept, cohort.valid_settled)),
                cohort.kept,
                cohort.started,
                apex_ui::format::percent(keep_rate(cohort.kept, cohort.started))
            ));
            out.push_str(&format!(
                "<p class=\"panel-body\">{} voided are shown separately; {} pending are never counted as successes; {} unknown and {} still in flight stay outside the denominators.</p>",
                cohort.voided, cohort.pending, cohort.unknown, cohort.in_flight
            ));
            if !cohort.excluded_from_denominators.is_empty() {
                out.push_str(&format!(
                    "<p class=\"panel-body\">{}</p>",
                    html_escape(&cohort.excluded_from_denominators.join("; "))
                ));
            }
            out.push_str("</section>");
            out.push_str(&data_table(
                Some("Cohort outcomes"),
                &["Outcome", "Count", "Share of valid settled"],
                &[
                    vec![
                        Cell::Status("Kept".to_string(), StatusKind::Verified),
                        Cell::Num(cohort.kept.to_string()),
                        Cell::Num(apex_ui::format::percent(share(
                            cohort.kept,
                            cohort.valid_settled,
                        ))),
                    ],
                    vec![
                        Cell::Status("Rolled back".to_string(), StatusKind::Neutral),
                        Cell::Num(cohort.rolled_back.to_string()),
                        Cell::Num(apex_ui::format::percent(share(
                            cohort.rolled_back,
                            cohort.valid_settled,
                        ))),
                    ],
                    vec![
                        Cell::Status("Voided".to_string(), StatusKind::Unavailable),
                        Cell::Num(cohort.voided.to_string()),
                        Cell::Muted("separate".to_string()),
                    ],
                    vec![
                        Cell::Status("Still pending".to_string(), StatusKind::Queued),
                        Cell::Num(cohort.pending.to_string()),
                        Cell::Muted("never a success".to_string()),
                    ],
                ],
                None,
                Some("Kept share within 30-day maturity; completion p50/p95 shown when recorded."),
            ));
            if let (Some(p50), Some(p95)) = (cohort.completion_p50_days, cohort.completion_p95_days)
            {
                out.push_str(&format!(
                    "<p class=\"table-note\">Completion: p50 {p50}d / p95 {p95}d. Cohort {} / {}.</p>",
                    html_escape(&cohort.cohort_label),
                    meta.as_of_display()
                ));
            }
        }
    }
    out
}

/// The kept share as a plain ratio (`numerator / denominator`); the caller
/// renders it through [`apex_ui::format::percent`], which scales once.
fn keep_rate(numerator: u64, denominator: u64) -> f64 {
    if denominator == 0 {
        return 0.0;
    }
    numerator as f64 / denominator as f64
}

fn share(numerator: u64, denominator: u64) -> f64 {
    keep_rate(numerator, denominator)
}

fn cohort_filter(cohorts: &[String], selected: Option<&str>) -> String {
    let selected = selected.unwrap_or("latest mature");
    let mut out = String::from(
        "<nav class=\"tabs\" aria-label=\"Start cohorts\"><span class=\"status-pair-label\">Cohort:</span>",
    );
    for cohort in cohorts {
        let current = if cohort == selected {
            " aria-current=\"page\""
        } else {
            ""
        };
        out.push_str(&format!(
            "<a class=\"tab\" href=\"/control/experiments?view=effectiveness&amp;cohort={}\"{}>{}</a>",
            apex_ui::escape::attr(cohort),
            current,
            html_escape(cohort)
        ));
    }
    out.push_str("</nav>");
    out
}

fn bars(cohort: &ExperimentEffectiveness) -> String {
    let spec = apex_ui::charts::ChartSpec {
        title: format!("Start cohort {} outcomes", cohort.cohort_label),
        desc: format!(
            "Kept {}, rolled back {}, voided {}, still pending {}.",
            cohort.kept, cohort.rolled_back, cohort.voided, cohort.pending
        ),
        width: 480,
        height: 200,
        y_label: "Tests".to_string(),
        x_labels: vec![
            "Kept".to_string(),
            "Rolled back".to_string(),
            "Voided".to_string(),
            "Still pending".to_string(),
        ],
        zero_line: true,
    };
    let values = [
        Some(cohort.kept as f64),
        Some(cohort.rolled_back as f64),
        Some(cohort.voided as f64),
        Some(cohort.pending as f64),
    ];
    apex_ui::charts::bar_chart(&spec, &values, apex_ui::charts::ChartColor::Test)
}

/// The experiment detail page data (page 62).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExperimentDetail {
    /// The durable runner record.
    pub experiment: Result<DurableExperiment, String>,
    /// The engine's recorded lift row, when a decision exists.
    pub lift: Option<LiftRow>,
    /// The registered decision checks from the dataset.
    pub checks: Result<Vec<DecisionCheck>, String>,
    /// Scoped POST forms (restore reconciliation).
    pub forms: Vec<super::ActionFormSpec>,
}

/// Render the experiment detail.
pub fn render_detail(meta: &PageMeta, experiment_id: &str, detail: &ExperimentDetail) -> String {
    let mut body = header(meta, &format!("Experiment {experiment_id}"));
    match &detail.experiment {
        Err(reason) => {
            body.push_str(&notice(
                StatusKind::Unavailable,
                copy::TELEMETRY_ABSENT,
                &format!("The durable experiment record is not readable: {reason}"),
                None,
            ));
        }
        Ok(experiment) => {
            body.push_str(&format!(
                "<p class=\"panel-body\">Publisher <code>{}</code> / Registered change: {} / Window: 14 analysis days (registered)</p>",
                html_escape(&experiment.publisher),
                html_escape(
                    experiment
                        .change_description
                        .as_deref()
                        .unwrap_or("not recorded")
                )
            ));
            body.push_str(&kpis(&[
                (
                    "Observed lift",
                    &detail
                        .lift
                        .as_ref()
                        .and_then(|lift| lift.observed_lift_bp)
                        .map(|bp| apex_ui::format::percent(bp as f64 / 10_000.0))
                        .unwrap_or_else(|| {
                            apex_ui::format::or_not_available("no recorded decision")
                        }),
                    "Point estimate from the engine's recorded output",
                    None,
                ),
                (
                    "Billable bound",
                    &detail
                        .lift
                        .as_ref()
                        .and_then(|lift| lift.billable_bound_bp)
                        .map(|bp| apex_ui::format::percent(bp as f64 / 10_000.0))
                        .unwrap_or_else(|| apex_ui::format::or_not_available("no recorded bound")),
                    "From the signed decision record",
                    None,
                ),
                (
                    "Platform state",
                    &experiment
                        .state
                        .clone()
                        .unwrap_or_else(|| "State unknown".to_string()),
                    &experiment
                        .observed_at_ms
                        .map(|at| format!("Observed {}", apex_ui::format::utc_datetime(at)))
                        .unwrap_or_else(|| "No observation recorded".to_string()),
                    None,
                ),
            ]));
            body.push_str(&render_checks(&detail.checks));
            body.push_str(&render_provenance(experiment, detail.lift.as_ref()));
        }
    }
    for form in &detail.forms {
        body.push_str(&format!(
            "<section class=\"panel\"><h3 class=\"panel-heading\">{}</h3>{}</section>",
            html_escape(&form.label),
            super::action_form(form)
        ));
    }
    body.push_str(&format!(
        "<p class=\"table-note\">{}</p>",
        html_escape(copy::NO_MANUAL_KEEP)
    ));
    document(
        meta,
        &format!("Experiment {experiment_id}"),
        &["Experiments", experiment_id],
        &body,
    )
}

fn render_checks(checks: &Result<Vec<DecisionCheck>, String>) -> String {
    match checks {
        Err(reason) => notice(
            StatusKind::Unavailable,
            "The registered decision checks are not available.",
            reason,
            None,
        ),
        Ok(checks) => {
            let cells: Vec<Vec<Cell>> = checks
                .iter()
                .map(|check| {
                    vec![
                        Cell::Text(check.check.clone()),
                        Cell::Mono(check.registered.clone()),
                        Cell::Mono(check.observed.clone()),
                        Cell::Status(check.verdict.clone(), check_kind(&check.verdict)),
                    ]
                })
                .collect();
            data_table(
                Some("Registered decision checks"),
                &["Check", "Registered", "Observed", "Verdict"],
                &cells,
                Some(EmptyState {
                    reason: copy::AWAITING_DATA.to_string(),
                    action: Some((
                        copy::AWAITING_DATA_ACTION.to_string(),
                        "/control/ingestion".to_string(),
                    )),
                }),
                Some("Observed values come from the engine records; this page recomputes nothing."),
            )
        }
    }
}

fn check_kind(verdict: &str) -> StatusKind {
    match verdict {
        "Pass" => StatusKind::Verified,
        "Fail" => StatusKind::Failure,
        "Pending" => StatusKind::Queued,
        _ => StatusKind::Neutral,
    }
}

fn render_provenance(experiment: &DurableExperiment, lift: Option<&LiftRow>) -> String {
    let mut out =
        String::from("<section class=\"panel\"><h2 class=\"panel-heading\">Provenance</h2>");
    out.push_str(&format!(
        "<p class=\"panel-body\">Registered metric: {}. </p>",
        html_escape(experiment.metric.as_deref().unwrap_or("not recorded"))
    ));
    match lift {
        Some(lift) => {
            out.push_str(&format!(
                "<p class=\"panel-body\">Engine measurement source: <code>{}</code><br>Source digest: <code>{}</code><br>Decided at: {}</p>",
                html_escape(lift.measurement_source.as_deref().unwrap_or("not recorded")),
                html_escape(lift.measurement_version.as_deref().unwrap_or("not recorded")),
                html_escape(lift.decided_at.as_deref().unwrap_or("not recorded"))
            ));
        }
        None => {
            out.push_str(&format!(
                "<p class=\"panel-body\">{}</p>",
                html_escape(&apex_ui::format::or_not_available(
                    "no engine decision is recorded for this experiment"
                ))
            ));
        }
    }
    out.push_str("</section>");
    out
}

/// The demo decision checks for `exp_0246` (fixture dataset).
pub fn demo_checks() -> Vec<DecisionCheck> {
    vec![
        DecisionCheck {
            check: "Final days".to_string(),
            registered: "14".to_string(),
            observed: "14 final days".to_string(),
            verdict: "Pass".to_string(),
        },
        DecisionCheck {
            check: "Observed lift".to_string(),
            registered: ">= 0".to_string(),
            observed: "+540 bp".to_string(),
            verdict: "Pass".to_string(),
        },
        DecisionCheck {
            check: "Billable bound".to_string(),
            registered: "300 bp".to_string(),
            observed: "+300 bp".to_string(),
            verdict: "Pass".to_string(),
        },
        DecisionCheck {
            check: "Bootstrap check".to_string(),
            registered: "upper 95% bound".to_string(),
            observed: "upper bound +620 bp".to_string(),
            verdict: "Pass".to_string(),
        },
        DecisionCheck {
            check: "Guardrail status".to_string(),
            registered: "no open guardrail".to_string(),
            observed: "clear".to_string(),
            verdict: "Pass".to_string(),
        },
    ]
}

/// Unused money helper kept for the finance handoff: format a money value.
pub fn money(value: &Money) -> String {
    apex_ui::format::money(value.micros, &value.currency)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::DemoData;

    fn meta() -> PageMeta {
        PageMeta {
            environment: "TEST".to_string(),
            as_of_ms: crate::fixtures::DEMO_NOW_MS,
            operator: Some("Maya".to_string()),
            role: Some(crate::capabilities::Role::Operator),
            fixtures: true,
            csrf: String::new(),
        }
    }

    #[test]
    fn keep_rates_are_plain_ratios_scaled_once_by_the_formatter() {
        // 18 of 30 kept is a ratio of 0.6, rendered as 60.0%.
        assert_eq!(keep_rate(18, 30), 0.6);
        assert_eq!(share(18, 30), 0.6);
        assert_eq!(apex_ui::format::percent(keep_rate(18, 30)), "60.0%");
        // 18 of 40 started is 45.0%.
        assert_eq!(apex_ui::format::percent(keep_rate(18, 40)), "45.0%");
        // A zero denominator has no share and renders as 0.0%, never a NaN.
        assert_eq!(keep_rate(0, 0), 0.0);
        assert_eq!(apex_ui::format::percent(keep_rate(1, 0)), "0.0%");
    }

    #[test]
    fn the_effectiveness_view_renders_corrected_keep_shares() {
        let demo = DemoData::new();
        let data = ExperimentsData {
            landscape: Ok(demo.experiment_landscape()),
            effectiveness: Some(
                demo.experiment_effectiveness("2026-08")
                    .map_err(|e| e.reason),
            ),
            cohorts: vec!["2026-08".to_string(), "2026-09".to_string()],
            selected_cohort: Some("2026-08".to_string()),
            decisions_due: false,
        };
        let html = render_fleet(&meta(), &data);
        assert!(html.contains("60.0%"), "18 of 30 valid settled is 60.0%");
        assert!(html.contains("45.0%"), "18 of 40 started is 45.0%");
        assert!(!html.contains("6000.0%"), "the share is never scaled twice");
        assert!(!html.contains("4500.0%"), "the share is never scaled twice");
    }

    #[test]
    fn recorded_lift_basis_points_render_as_percent_exactly_once() {
        // The recorded basis points are divided by 10_000, matching the
        // apex-api contract: 540 bp is 5.4%, 300 bp is 3.0%.
        let demo = DemoData::new();
        let detail = ExperimentDetail {
            experiment: demo
                .experiment_detail("exp_0246")
                .ok_or_else(|| "fixture experiment missing".to_string()),
            lift: demo
                .lift_rows()
                .into_iter()
                .find(|row| row.experiment_id == "exp_0246"),
            checks: Ok(demo_checks()),
            forms: Vec::new(),
        };
        let html = render_detail(&meta(), "exp_0246", &detail);
        assert!(html.contains("5.4%"), "540 bp renders 5.4%");
        assert!(html.contains("3.0%"), "300 bp renders 3.0%");
        assert!(!html.contains("540.0%"), "lift is never scaled twice");
        assert!(!html.contains("54.0%"), "lift is never scaled twice");
    }
}
