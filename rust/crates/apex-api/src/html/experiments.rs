//! `/console/experiments` and `/console/experiments/{id}` (plus the incident
//! view): the runner-backed experiment surfaces.
//!
//! The list uses the frozen state names, states and next steps. The detail
//! page keeps uncertainty visible: the SVG chart carries an adjacent data
//! table, the interval crossing zero is stated in words, and the pending
//! state repeats the frozen "a required input is incomplete or reporting is
//! paused" copy. The restore-failed incident page carries the frozen
//! intervention copy, the incident reference and the next check.

use apex_ui::charts::{line_chart, ChartColor, ChartSeries, ChartSpec};
use apex_ui::components::{action, data_table, notice, panel, tabs, Cell, EmptyState};
use apex_ui::format::{or_not_available, percent, utc_datetime};
use apex_ui::status::StatusKind;

use crate::experiments::{
    detail_chip_label, platform_label, ExperimentStateView, ExperimentView, INCIDENT_NEXT_STEP,
    PREVIOUS_STATE_NEXT_STEP, RESTORE_FAILED_HEADLINE, RESTORE_FAILED_MESSAGE,
};

use super::{console_document, html_escape, ConsoleCtx};

/// The list filter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExperimentFilter {
    /// All states.
    All,
    /// Finalized data only.
    Finalized,
    /// Last 30 days.
    Last30Days,
}

impl ExperimentFilter {
    /// The query value.
    pub fn as_query(self) -> Option<&'static str> {
        match self {
            ExperimentFilter::All => None,
            ExperimentFilter::Finalized => Some("final"),
            ExperimentFilter::Last30Days => Some("30d"),
        }
    }

    /// Parse the `filter` query parameter; an unknown value is `None`.
    pub fn parse(value: Option<&str>) -> Option<Self> {
        match value {
            None | Some("") | Some("all") => Some(ExperimentFilter::All),
            Some("final") => Some(ExperimentFilter::Finalized),
            Some("30d") => Some(ExperimentFilter::Last30Days),
            Some(_) => None,
        }
    }
}

/// The detail tabs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExperimentTab {
    /// Summary (chart and decision).
    Summary,
    /// Inputs (metric, allocation, versions).
    Inputs,
    /// Guardrails.
    Guardrails,
    /// History.
    History,
}

impl ExperimentTab {
    /// The query value.
    pub fn as_query(self) -> Option<&'static str> {
        match self {
            ExperimentTab::Summary => None,
            ExperimentTab::Inputs => Some("inputs"),
            ExperimentTab::Guardrails => Some("guardrails"),
            ExperimentTab::History => Some("history"),
        }
    }

    /// Parse the `tab` query parameter; an unknown value is `None`.
    pub fn parse(value: Option<&str>) -> Option<Self> {
        match value {
            None | Some("") | Some("summary") => Some(ExperimentTab::Summary),
            Some("inputs") => Some(ExperimentTab::Inputs),
            Some("guardrails") => Some(ExperimentTab::Guardrails),
            Some("history") => Some(ExperimentTab::History),
            Some(_) => None,
        }
    }
}

/// The list page data.
#[derive(Debug, Clone, PartialEq)]
pub struct ExperimentsListData<'a> {
    /// Workspace publisher.
    pub publisher: &'a str,
    /// Read time of the page.
    pub as_of_unix_ms: i64,
    /// The active filter.
    pub filter: ExperimentFilter,
    /// The projected experiments.
    pub views: Vec<ExperimentView>,
}

/// The detail page data.
#[derive(Debug, Clone, PartialEq)]
pub struct ExperimentDetailData<'a> {
    /// Workspace publisher.
    pub publisher: &'a str,
    /// The experiment.
    pub view: &'a ExperimentView,
    /// The active tab.
    pub tab: ExperimentTab,
    /// The signed ledger history of the experiment.
    pub history: Vec<crate::experiments::ExperimentHistoryEntry>,
}

/// Render the experiments list.
pub fn render_list(ctx: &ConsoleCtx, data: &ExperimentsListData<'_>) -> String {
    let mut body = String::from("<h1>Experiments</h1>");
    body.push_str(&format!(
        "<p class=\"page-subtitle\">{}</p>",
        html_escape(&format!(
            "{} / as of {}",
            data.publisher,
            utc_datetime(data.as_of_unix_ms)
        ))
    ));
    let tabs_html = tabs(&[
        (
            "All states".to_string(),
            "/console/experiments".to_string(),
            data.filter == ExperimentFilter::All,
        ),
        (
            "Finalized data only".to_string(),
            "/console/experiments?filter=final".to_string(),
            data.filter == ExperimentFilter::Finalized,
        ),
        (
            "Last 30 days".to_string(),
            "/console/experiments?filter=30d".to_string(),
            data.filter == ExperimentFilter::Last30Days,
        ),
    ]);
    body.push_str(&tabs_html);

    let cutoff = data.as_of_unix_ms - 30 * 86_400_000;
    let rows: Vec<Vec<Cell>> =
        data.views
            .iter()
            .filter(|view| match data.filter {
                ExperimentFilter::All => true,
                ExperimentFilter::Finalized => view.is_finalized(),
                ExperimentFilter::Last30Days => {
                    view.started_at_unix_ms.is_none_or(|at| at >= cutoff) || view.result.is_some()
                }
            })
            .map(|view| {
                vec![
                    Cell::Link(
                        view.experiment_id.clone(),
                        format!("/console/experiments/{}", view.experiment_id),
                    ),
                    Cell::Text(view.scope.clone().unwrap_or_else(|| {
                        or_not_available("no pre-registration record is stored")
                    })),
                    Cell::Status(view.state.label().to_string(), state_kind(&view.state)),
                    Cell::Text(view.next_step()),
                ]
            })
            .collect();
    body.push_str(&data_table(
        Some("Registered experiments"),
        &["Experiment", "Scope", "State", "Next step"],
        &rows,
        Some(EmptyState {
            reason: match data.filter {
                ExperimentFilter::All => {
                    "No experiment is recorded in this workspace's verified history.".to_string()
                }
                ExperimentFilter::Finalized => {
                    "No finalized experiment matches this filter.".to_string()
                }
                ExperimentFilter::Last30Days => {
                    "No experiment started in the last 30 days.".to_string()
                }
            },
            action: None,
        }),
        None,
    ));
    body.push_str(&notice(
        StatusKind::Neutral,
        "Pending data means waiting.",
        "A positive interim estimate does not turn a row green or create a fee; only a recorded \
         decision does.",
        None,
    ));
    console_document(ctx, "Experiments", "Experiments", &body)
}

fn state_kind(state: &ExperimentStateView) -> StatusKind {
    match state {
        ExperimentStateView::Kept => StatusKind::Verified,
        ExperimentStateView::RestoreFailed => StatusKind::Failure,
        ExperimentStateView::Stopped | ExperimentStateView::Voided => StatusKind::Attention,
        _ => StatusKind::Neutral,
    }
}

/// Render the experiment detail page.
pub fn render_detail(ctx: &ConsoleCtx, data: &ExperimentDetailData<'_>) -> String {
    let view = data.view;
    let title = format!(
        "{} / {}",
        view.experiment_id,
        view.scope
            .clone()
            .unwrap_or_else(|| or_not_available("scope"))
    );
    let mut body = format!("<h1>{}</h1>", html_escape(&title));
    body.push_str(&notice(
        state_kind(&view.state),
        detail_chip_label(&view.state),
        view.state.customer_sees(),
        None,
    ));
    if let Some(pending) = &view.promotion_pending {
        body.push_str(&notice(
            StatusKind::Attention,
            "Promotion pending",
            &format!(
                "{} The deadline is {} (UTC).",
                pending.instructions,
                date_time(pending.deadline_unix_ms)
            ),
            None,
        ));
    }
    let base = format!("/console/experiments/{}", view.experiment_id);
    body.push_str(&tabs(&[
        (
            "Summary".to_string(),
            base.clone(),
            data.tab == ExperimentTab::Summary,
        ),
        (
            "Inputs".to_string(),
            format!("{base}?tab=inputs"),
            data.tab == ExperimentTab::Inputs,
        ),
        (
            "Guardrails".to_string(),
            format!("{base}?tab=guardrails"),
            data.tab == ExperimentTab::Guardrails,
        ),
        (
            "History".to_string(),
            format!("{base}?tab=history"),
            data.tab == ExperimentTab::History,
        ),
    ]));
    match data.tab {
        ExperimentTab::Summary => body.push_str(&summary_tab(view)),
        ExperimentTab::Inputs => body.push_str(&inputs_tab(view)),
        ExperimentTab::Guardrails => body.push_str(&guardrails_tab(view)),
        ExperimentTab::History => body.push_str(&history_tab(data)),
    }
    if let Some(incident) = &view.incident {
        body.push_str(&format!(
            "<p>{}</p>",
            action("View incident details", &format!("{base}/incident"), false)
        ));
        let _ = incident;
    }
    console_document(
        ctx,
        &format!("Experiment {}", view.experiment_id),
        "Experiments",
        &body,
    )
}

fn summary_tab(view: &ExperimentView) -> String {
    let mut out = String::new();
    if let Some(result) = &view.result {
        out.push_str(&chart_panel(view, result));
        if result.decision == "keep" && result.lambda > 0.0 {
            out.push_str(&notice(
                StatusKind::Verified,
                "The decision rule passed.",
                "The signed result carries a positive lower bound; billing uses that bound, not the \
                 point estimate.",
                None,
            ));
        } else if result.lower_90 <= 0.0 {
            out.push_str(&notice(
                StatusKind::Neutral,
                "The interval crosses zero. No lift fee has been established.",
                "A positive estimate by itself does not establish a fee.",
                None,
            ));
        }
    } else if view.state == ExperimentStateView::RestoreFailed {
        out.push_str(&recovery_panel(view));
    } else {
        out.push_str(&panel(
            "Observed lift and uncertainty",
            "<p>No published estimate exists yet. The chart and interval appear when the signed \
             analysis record is written; a percent is never invented from partial days.</p>",
            Some(&view.next_step()),
        ));
        out.push_str(&facts_panel(view));
    }
    if let Some(result) = &view.result {
        let rows = vec![
            vec![
                Cell::Text("Observed lift".to_string()),
                Cell::Num(
                    result
                        .observed_lift_bp
                        .map(|bp| percent(bp as f64 / 10_000.0))
                        .unwrap_or_else(|| or_not_available("the signed mean is not finite")),
                ),
            ],
            vec![
                Cell::Text("Billable bound (90% lower)".to_string()),
                Cell::Num(percent(result.lower_90)),
            ],
            vec![
                Cell::Text("Bootstrap lower bound".to_string()),
                Cell::Num(percent(result.bootstrap_lower)),
            ],
            vec![
                Cell::Text("Decision".to_string()),
                Cell::Text(format!(
                    "{} / lambda {}",
                    result.decision,
                    percent(result.lambda)
                )),
            ],
            vec![
                Cell::Text("Signed result".to_string()),
                Cell::Mono(format!(
                    "seq {} / sha {}",
                    result.seq,
                    short_hex(&result.payload_sha256)
                )),
            ],
        ];
        out.push_str(&data_table(
            Some("Signed result"),
            &["Output", "Value"],
            &rows,
            None,
            None,
        ));
        if view.state == ExperimentStateView::Kept {
            out.push_str(&format!(
                "<p>{}</p>",
                action("View fee basis", "/console/billing", false)
            ));
        }
    }
    out.push_str(&facts_panel(view));
    out
}

fn chart_panel(view: &ExperimentView, result: &crate::experiments::ExperimentResultView) -> String {
    let window = view.window_days.unwrap_or(0) as usize;
    let points: Vec<Option<f64>> = result
        .daily_log_ratios
        .iter()
        .map(|ratio| {
            if ratio.is_finite() {
                Some(ratio * 100.0)
            } else {
                None
            }
        })
        .collect();
    let labels: Vec<String> = view
        .data_pull_days
        .iter()
        .take(points.len().max(1))
        .enumerate()
        .map(|(index, day)| {
            if index < view.data_pull_days.len() {
                day.clone()
            } else {
                format!("analysis day {}", index + 1)
            }
        })
        .collect();
    let labels = if labels.len() >= points.len().max(1) {
        labels
    } else {
        (1..=points.len().max(1))
            .map(|index| format!("analysis day {index}"))
            .collect()
    };
    let (mut y_min, mut y_max) = (0.0f64, 0.0f64);
    for point in points.iter().flatten() {
        y_min = y_min.min(*point);
        y_max = y_max.max(*point);
    }
    let pad = ((y_max - y_min) * 0.1).max(0.5);
    let spec = ChartSpec {
        title: "Observed lift and uncertainty".to_string(),
        desc: format!(
            "Daily log ratios in percent for the signed result of {}; {} analysis days of the \
             {}-day window are final. Values are rounded in the table, not the SVG.",
            view.experiment_id,
            view.data_pull_days.len().min(window.max(1)),
            if window == 0 { 14 } else { window }
        ),
        width: 640,
        height: 220,
        y_label: "Lift (daily log ratio, %)".to_string(),
        x_labels: labels,
        zero_line: true,
    };
    let series = vec![ChartSeries {
        label: "Daily log ratio".to_string(),
        color: ChartColor::Test,
        points,
    }];
    line_chart(&spec, &series, y_min - pad, y_max + pad)
}

fn facts_panel(view: &ExperimentView) -> String {
    let window = view.window_days.unwrap_or(0);
    let pulled = view
        .data_pull_days
        .len()
        .min(usize::try_from(window).unwrap_or(usize::MAX));
    let billable = match &view.result {
        Some(result) if result.decision == "keep" => "decided: kept",
        Some(_) => "decided: no fee",
        None => "not decided",
    };
    let rows = vec![
        vec![
            Cell::Text("Source".to_string()),
            Cell::Text(format!(
                "{} platform reporting",
                platform_label(view.platform)
            )),
        ],
        vec![
            Cell::Text("Metric".to_string()),
            Cell::Text("Revenue / allocated share".to_string()),
        ],
        vec![
            Cell::Text("Analysis days final".to_string()),
            Cell::Text(if window == 0 {
                or_not_available("no window is registered")
            } else {
                format!("{pulled} of {window} analysis days final")
            }),
        ],
        vec![
            Cell::Text("Billable result".to_string()),
            Cell::Text(billable.to_string()),
        ],
    ];
    data_table(
        Some("What this chart uses"),
        &["Fact", "Value"],
        &rows,
        None,
        None,
    )
}

fn inputs_tab(view: &ExperimentView) -> String {
    let rows = vec![
        vec![
            Cell::Text("Scope".to_string()),
            Cell::Mono(
                view.scope
                    .clone()
                    .unwrap_or_else(|| or_not_available("no pre-registration record is stored")),
            ),
        ],
        vec![
            Cell::Text("Allocation".to_string()),
            Cell::Text(view.allocation_label()),
        ],
        vec![
            Cell::Text("Burn-in".to_string()),
            Cell::Text(match view.burn_in_days {
                Some(days) => format!("{days} days"),
                None => or_not_available("no pre-registration record is stored"),
            }),
        ],
        vec![
            Cell::Text("Analysis window".to_string()),
            Cell::Text(match view.window_days {
                Some(days) => format!("{days} days"),
                None => or_not_available("no pre-registration record is stored"),
            }),
        ],
        vec![
            Cell::Text("Engine / rule version".to_string()),
            Cell::Mono(
                view.rule_version
                    .clone()
                    .unwrap_or_else(|| or_not_available("no pre-registration record is stored")),
            ),
        ],
        vec![
            Cell::Text("Fee rate".to_string()),
            Cell::Text(match view.rate_bp {
                Some(bp) => percent(bp as f64 / 10_000.0),
                None => or_not_available("no pre-registration record is stored"),
            }),
        ],
        vec![
            Cell::Text("Metric exclusions".to_string()),
            Cell::Text("None recorded".to_string()),
        ],
        vec![
            Cell::Text("Finalized pulls".to_string()),
            Cell::Text(format!(
                "{} recorded analysis days",
                view.data_pull_days.len()
            )),
        ],
    ];
    format!(
        "<p>{}</p>{}",
        html_escape(&view.metric_definition()),
        data_table(
            Some("Registered inputs"),
            &["Input", "Value"],
            &rows,
            None,
            None,
        )
    )
}

fn guardrails_tab(view: &ExperimentView) -> String {
    let rows: Vec<Vec<Cell>> = view
        .guardrails
        .iter()
        .map(|guardrail| {
            vec![
                Cell::Text(guardrail.day.clone()),
                Cell::Text(guardrail.guardrail.clone()),
                Cell::Text(guardrail.action.clone()),
            ]
        })
        .collect();
    let mut out = data_table(
        Some("Guardrail checks that fired"),
        &["Day", "Guardrail", "Action"],
        &rows,
        Some(EmptyState {
            reason: "No guardrail fired on the recorded days.".to_string(),
            action: None,
        }),
        None,
    );
    if let Some(thresholds) = &view.guardrail_thresholds {
        out.push_str(&format!(
            "<p class=\"table-note\">Registered thresholds: <code>{}</code></p>",
            html_escape(&thresholds.to_string())
        ));
    }
    out
}

fn history_tab(data: &ExperimentDetailData<'_>) -> String {
    let rows: Vec<Vec<Cell>> = data
        .history
        .iter()
        .map(|entry| {
            vec![
                Cell::Mono(entry.seq.to_string()),
                Cell::Text(utc_datetime(entry.at_unix_ms)),
                Cell::Text(entry.label.to_string()),
                Cell::Link(
                    "Open record".to_string(),
                    format!("/console/ledger/records/{}", entry.seq),
                ),
            ]
        })
        .collect();
    data_table(
        Some("Signed history"),
        &["Seq", "Recorded at", "Record", "Action"],
        &rows,
        Some(EmptyState {
            reason: "No signed record is stored for this experiment.".to_string(),
            action: None,
        }),
        None,
    )
}

/// The recovery panel body (used by the incident page and the detail).
pub fn recovery_panel(view: &ExperimentView) -> String {
    let Some(incident) = &view.incident else {
        return panel(
            RESTORE_FAILED_HEADLINE,
            "<p>No unresolved incident is recorded for this experiment.</p>",
            None,
        );
    };
    let rows = vec![
        vec![
            Cell::Text(incident.previous_state.clone()),
            Cell::Text(date_time(incident.recorded_at_unix_ms)),
            Cell::Text(INCIDENT_NEXT_STEP.to_string()),
        ],
        vec![
            Cell::Text("Previous settings are recorded".to_string()),
            Cell::Text(match incident.previous_recorded_at_unix_ms {
                Some(at) => date_time(at),
                None => or_not_available("no preceding change is recorded"),
            }),
            Cell::Text(PREVIOUS_STATE_NEXT_STEP.to_string()),
        ],
    ];
    let mut out = notice(
        StatusKind::Failure,
        &format!("Restore not confirmed / incident {}", incident.reference),
        RESTORE_FAILED_MESSAGE,
        None,
    );
    out.push_str(&data_table(
        Some("Known state"),
        &["Known state", "Recorded at", "Next step"],
        &rows,
        None,
        None,
    ));
    if !view.previous_settings_recorded {
        out.push_str(&format!(
            "<p class=\"table-note\">The preceding change record is {}; manual review still \
             needs the platform's own settings screen.</p>",
            or_not_available("not stored")
        ));
    }
    panel(RESTORE_FAILED_HEADLINE, &out, None)
}

/// Render the standalone incident page.
pub fn render_incident(ctx: &ConsoleCtx, publisher: &str, view: &ExperimentView) -> String {
    let mut body = format!("<h1>{}</h1>", html_escape(RESTORE_FAILED_HEADLINE));
    body.push_str(&format!(
        "<p class=\"page-subtitle\">{}</p>",
        html_escape(&format!(
            "{} / experiment {} / {}",
            publisher,
            view.experiment_id,
            platform_label(view.platform)
        ))
    ));
    body.push_str(&recovery_panel(view));
    body.push_str(&state_table(view));
    body.push_str(&format!(
        "<p>{}</p>",
        action(
            "Back to the experiment",
            &format!("/console/experiments/{}", view.experiment_id),
            false
        )
    ));
    console_document(
        ctx,
        &format!("Incident {}", view.experiment_id),
        "Experiments",
        &body,
    )
}

fn state_table(view: &ExperimentView) -> String {
    let rows = vec![vec![
        Cell::Text(view.state.decision_state().to_string()),
        Cell::Text(view.state.customer_sees().to_string()),
        Cell::Text(view.state.billing_behavior().to_string()),
    ]];
    data_table(
        Some("Decision state"),
        &["State", "What the customer sees", "What becomes billable"],
        &rows,
        None,
        None,
    )
}

fn date_time(unix_ms: i64) -> String {
    utc_datetime(unix_ms)
}

fn short_hex(hash: &str) -> String {
    hash.chars().take(16).collect()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use apex_core::PlatformId;
    use apex_lift::records::GuardrailRecord;

    use crate::experiments::{ExperimentResultView, IncidentView};

    use super::*;

    fn ctx() -> ConsoleCtx {
        ConsoleCtx::new("Northstar Studio", "USD", true)
    }

    fn base_view(state: ExperimentStateView) -> ExperimentView {
        ExperimentView {
            experiment_id: "exp_0247".to_string(),
            platform: Some(PlatformId::AppLovinMax),
            state,
            scope: Some("rw_de_01".to_string()),
            metric: Some("RevenuePerUser".to_string()),
            test_share: Some(0.25),
            burn_in_days: Some(3),
            window_days: Some(14),
            rule_version: Some("apex-lift-test".to_string()),
            rate_bp: Some(2_000),
            started_at_unix_ms: Some(1_791_900_000_000),
            data_pull_days: vec!["2026-10-01".to_string(), "2026-10-02".to_string()],
            promotion_pending: None,
            result: None,
            guardrails: Vec::new(),
            guardrail_thresholds: None,
            started_seq: Some(1),
            incident: None,
            previous_settings_recorded: false,
        }
    }

    fn waiting_view() -> ExperimentView {
        base_view(ExperimentStateView::WaitingForData {
            missing_days: Some(2),
        })
    }

    fn kept_view() -> ExperimentView {
        let mut view = base_view(ExperimentStateView::Kept);
        view.data_pull_days = vec![
            "2026-10-01".to_string(),
            "2026-10-02".to_string(),
            "2026-10-03".to_string(),
        ];
        view.result = Some(ExperimentResultView {
            observed_lift_bp: Some(540),
            lower_90: 0.03,
            lower_99: 0.02,
            bootstrap_lower: 0.028,
            decision: "keep".to_string(),
            lambda: 0.03,
            daily_log_ratios: vec![0.05, 0.055, 0.052],
            seq: 9,
            payload_sha256: "ab".repeat(32),
            at_unix_ms: 1_792_055_400_000,
        });
        view
    }

    fn incident_view() -> ExperimentView {
        let mut view = base_view(ExperimentStateView::RestoreFailed);
        view.incident = Some(IncidentView {
            reference: "inc_0123456789".to_string(),
            recorded_at_unix_ms: 1_792_050_000_000,
            previous_state: "Test still reported active".to_string(),
            previous_recorded_at_unix_ms: Some(1_791_900_000_000),
        });
        view.previous_settings_recorded = true;
        view
    }

    #[test]
    fn list_renders_states_and_next_steps() {
        let data = ExperimentsListData {
            publisher: "northstar",
            as_of_unix_ms: 1_792_055_400_000,
            filter: ExperimentFilter::All,
            views: vec![waiting_view(), kept_view(), incident_view()],
        };
        let html = render_list(&ctx(), &data);
        assert!(html.contains("<h1>Experiments</h1>"));
        assert!(html.contains("Waiting for data"));
        assert!(html.contains("2 final days missing"));
        assert!(html.contains(">Kept<"));
        assert!(html.contains("View fee basis"));
        assert!(html.contains("Restore failed"));
        assert!(html.contains("Open incident"));
        assert!(html.contains("Pending data means waiting."));
        assert!(html.contains("All states"));
        assert!(!html.contains("<script"));
    }

    #[test]
    fn filters_are_stable_get_links_and_unknown_values_are_refused() {
        assert_eq!(ExperimentFilter::parse(None), Some(ExperimentFilter::All));
        assert_eq!(
            ExperimentFilter::parse(Some("final")),
            Some(ExperimentFilter::Finalized)
        );
        assert_eq!(
            ExperimentFilter::parse(Some("30d")),
            Some(ExperimentFilter::Last30Days)
        );
        assert_eq!(ExperimentFilter::parse(Some("everything")), None);
        assert_eq!(
            ExperimentTab::parse(Some("inputs")),
            Some(ExperimentTab::Inputs)
        );
        assert_eq!(ExperimentTab::parse(Some("nope")), None);
        let filtered = ExperimentsListData {
            publisher: "northstar",
            as_of_unix_ms: 1_792_055_400_000,
            filter: ExperimentFilter::Finalized,
            views: vec![waiting_view(), kept_view()],
        };
        let html = render_list(&ctx(), &filtered);
        assert!(!html.contains(">Waiting for data<"), "not finalized");
        assert!(html.contains(">Kept<"));
        assert!(html.contains("aria-current=\"page\" href=\"/console/experiments?filter=final\""));
    }

    #[test]
    fn detail_shows_chart_with_adjacent_table_and_interval_copy() {
        let view = kept_view();
        let history = vec![crate::experiments::ExperimentHistoryEntry {
            seq: 9,
            at_unix_ms: 1_792_055_400_000,
            label: "result",
        }];
        let html = render_detail(
            &ctx(),
            &ExperimentDetailData {
                publisher: "northstar",
                view: &view,
                tab: ExperimentTab::Summary,
                history,
            },
        );
        assert!(html.contains("<h1>exp_0247 / rw_de_01</h1>"));
        assert!(html.contains("Observed lift and uncertainty"));
        assert!(html.contains("What this chart uses"));
        assert!(html.contains("AppLovin MAX platform reporting"));
        assert!(html.contains("Revenue / allocated share"));
        assert!(html.contains("3 of 14 analysis days final"));
        assert!(html.contains("The decision rule passed."));
        assert!(html.contains("View fee basis"));
        assert!(html.contains("<table class=\"data-table\">"));
        // The chart table also carries the values, so no meaning is hover-only.
        assert!(html.contains("chart-table"));
        assert!(html.contains("5.4%"));
        assert!(!html.contains("<script"));
    }

    #[test]
    fn a_pending_promotion_renders_its_instructions_and_deadline() {
        let mut view = base_view(ExperimentStateView::Kept);
        view.promotion_pending = Some(crate::experiments::PromotionPendingView {
            deadline_unix_ms: 1_792_000_000_000,
            instructions: "Promote group B in the dashboard.".to_string(),
        });
        let html = render_detail(
            &ctx(),
            &ExperimentDetailData {
                publisher: "northstar",
                view: &view,
                tab: ExperimentTab::Summary,
                history: Vec::new(),
            },
        );
        assert!(html.contains("Promotion pending"), "{html}");
        assert!(html.contains("Promote group B in the dashboard."), "{html}");
        assert!(html.contains("2026"), "{html}");
    }

    #[test]
    fn pending_promotion_instructions_are_escaped() {
        let mut view = base_view(ExperimentStateView::Kept);
        view.promotion_pending = Some(crate::experiments::PromotionPendingView {
            deadline_unix_ms: 1_792_000_000_000,
            instructions: "<script>alert(1)</script>".to_string(),
        });
        let html = render_detail(
            &ctx(),
            &ExperimentDetailData {
                publisher: "northstar",
                view: &view,
                tab: ExperimentTab::Summary,
                history: Vec::new(),
            },
        );
        assert!(!html.contains("<script>alert(1)</script>"), "{html}");
        assert!(html.contains("&lt;script&gt;"), "{html}");
    }

    #[test]
    fn waiting_detail_says_pending_data_means_waiting() {
        let view = waiting_view();
        let html = render_detail(
            &ctx(),
            &ExperimentDetailData {
                publisher: "northstar",
                view: &view,
                tab: ExperimentTab::Summary,
                history: Vec::new(),
            },
        );
        assert!(html.contains("Awaiting final data"));
        assert!(html.contains("A required input is incomplete or reporting is paused."));
        assert!(html.contains("No published estimate exists yet."));
        assert!(
            !html.contains("<figure class=\"chart\">"),
            "no chart without a signed result"
        );
    }

    #[test]
    fn crossing_zero_states_no_fee() {
        let mut view = kept_view();
        if let Some(result) = &mut view.result {
            result.decision = "rollback".to_string();
            result.lower_90 = -0.01;
            result.lambda = 0.0;
        }
        view.state = ExperimentStateView::Stopped;
        let html = render_detail(
            &ctx(),
            &ExperimentDetailData {
                publisher: "northstar",
                view: &view,
                tab: ExperimentTab::Summary,
                history: Vec::new(),
            },
        );
        assert!(html.contains("The interval crosses zero. No lift fee has been established."));
        assert!(!html.contains("View fee basis"));
    }

    #[test]
    fn inputs_guardrails_and_history_tabs_render_truthfully() {
        let mut view = waiting_view();
        view.guardrails = vec![GuardrailRecord {
            day: "2026-10-10".to_string(),
            guardrail: "fastHarm".to_string(),
            inputs: serde_json::json!({"pointLift": -0.06}),
            action: "stopped".to_string(),
        }];
        view.guardrail_thresholds = Some(serde_json::json!({"harmPoint": -0.05}));
        let inputs = render_detail(
            &ctx(),
            &ExperimentDetailData {
                publisher: "northstar",
                view: &view,
                tab: ExperimentTab::Inputs,
                history: Vec::new(),
            },
        );
        assert!(inputs.contains("allocation-normalized revenue metric"));
        assert!(inputs.contains("None recorded"));
        assert!(inputs.contains("apex-lift-test"));
        let guardrails = render_detail(
            &ctx(),
            &ExperimentDetailData {
                publisher: "northstar",
                view: &view,
                tab: ExperimentTab::Guardrails,
                history: Vec::new(),
            },
        );
        assert!(guardrails.contains("fastHarm"));
        assert!(guardrails.contains(">stopped<"));
        let empty_guardrails = render_detail(
            &ctx(),
            &ExperimentDetailData {
                publisher: "northstar",
                view: &base_view(ExperimentStateView::BurnIn),
                tab: ExperimentTab::Guardrails,
                history: Vec::new(),
            },
        );
        assert!(empty_guardrails.contains("No guardrail fired on the recorded days."));
    }

    #[test]
    fn the_incident_page_carries_the_frozen_recovery_copy() {
        let view = incident_view();
        let html = render_incident(&ctx(), "northstar", &view);
        assert!(html.contains("<h1>This test needs intervention</h1>"));
        assert!(html.contains("Restore not confirmed / incident inc_0123456789"));
        assert!(html.contains(
            "The platform refused the stop request. Apex has not marked the test restored."
        ));
        assert!(html.contains("Test still reported active"));
        assert!(html.contains("Operator investigating"));
        assert!(html.contains("Previous settings are recorded"));
        assert!(html.contains("Review before manual action"));
        assert!(!html.contains("<script"));
    }

    #[test]
    fn hostile_scope_values_are_escaped() {
        let mut view = waiting_view();
        view.scope = Some("<script>alert(1)</script>".to_string());
        let html = render_detail(
            &ctx(),
            &ExperimentDetailData {
                publisher: "northstar",
                view: &view,
                tab: ExperimentTab::Summary,
                history: Vec::new(),
            },
        );
        assert!(!html.contains("<script"));
        assert!(html.contains("&lt;script&gt;alert(1)&lt;/script&gt;"));
    }
}
