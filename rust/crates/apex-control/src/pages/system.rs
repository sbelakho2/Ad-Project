//! System `/control/system` and `/control/system/policies` (pages 70 and
//! 72): telemetry, release health, the verified-operation matrix, the policy
//! two-step and the fleet interventions.

use apex_analytics::projections::OperatorQueueRow;
use apex_ui::components::{data_table, notice, Cell, EmptyState};
use apex_ui::escape::html_escape;
use apex_ui::status::StatusKind;

use crate::copy;
use crate::data::ReadinessRow;
use crate::fixtures::{ReleaseHealthRow, SystemComponentRow};
use crate::governance::{InterventionRecord, PolicyRecord};

use super::{action_form, document, header, kpis, ActionFormSpec, PageMeta};

/// The system page data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemData {
    /// Overdue jobs from the operator queue.
    pub overdue_jobs: Result<Vec<OperatorQueueRow>, String>,
    /// Overdue job count derived from the same rows (`None` when the read
    /// failed: no fabricated zero).
    pub component_rows: Vec<SystemComponentRow>,
    /// The copied-ledger sync line: the recorded sync time and age, or the
    /// honest not-recorded state (see [`crate::data::LedgerCopy`]).
    pub ledger_copy_line: String,
    /// Scoped `job_retry` forms for the terminal durable jobs in the store.
    pub job_retry_forms: Vec<ActionFormSpec>,
}

/// Render the system page.
pub fn render(meta: &PageMeta, data: &SystemData) -> String {
    let mut body = header(meta, "System");
    let (jobs_value, jobs_note) = match &data.overdue_jobs {
        Ok(rows) => (
            rows.len().to_string(),
            format!(
                "Oldest {}",
                rows.iter()
                    .map(|row| super::overview::age_label(row.age_seconds))
                    .max()
                    .unwrap_or_else(|| "0m".to_string())
            ),
        ),
        Err(reason) => (
            apex_ui::format::or_not_available(reason),
            "The queue read is unavailable".to_string(),
        ),
    };
    body.push_str(&kpis(&[
        (
            "Availability",
            &apex_ui::format::or_not_available("no scrape is configured"),
            "This signal is not instrumented; no healthy claim is made from missing telemetry",
            None,
        ),
        (
            "HTML p95",
            &apex_ui::format::or_not_available("no scrape is configured"),
            "This signal is not instrumented; no latency claim is made from missing telemetry",
            None,
        ),
        (
            "5xx rate",
            &apex_ui::format::or_not_available("no scrape is configured"),
            "This signal is not instrumented; no zero-error claim is made from missing telemetry",
            None,
        ),
        ("Overdue jobs", &jobs_value, &jobs_note, None),
    ]));
    body.push_str(&notice(
        StatusKind::Attention,
        copy::TELEMETRY_ABSENT,
        "Availability, latency and error-rate SLIs are reported as not instrumented until a metrics source is wired; the bounded registry (`apex-metrics`) is not present in this workspace.",
        Some((copy::TELEMETRY_ABSENT_ACTION, "/control/system")),
    ));
    body.push_str(&format!(
        "<p class=\"panel-body\">{}</p>",
        html_escape(&data.ledger_copy_line)
    ));
    let component_cells: Vec<Vec<Cell>> = data
        .component_rows
        .iter()
        .map(|row| {
            vec![
                Cell::Text(row.component.clone()),
                Cell::Text(row.last_success.clone()),
                Cell::Status(row.signal.clone(), StatusKind::Neutral),
                Cell::Text(row.next_action.clone()),
            ]
        })
        .collect();
    body.push_str(&data_table(
        Some("Components"),
        &["Component", "Last success", "Signal", "Next action"],
        &component_cells,
        Some(EmptyState {
            reason: "No component rows are configured.".to_string(),
            action: None,
        }),
        Some("Uptime apart from report freshness, decision timeliness and restore success."),
    ));
    if !data.job_retry_forms.is_empty() {
        body.push_str("<h2 class=\"section-heading\">Retry a durable job</h2>");
        body.push_str(&format!(
            "<p class=\"panel-body\">{}</p>",
            html_escape(
                "A retry re-enqueues the referenced terminal job under its own owner; the previous state and outcome stay in the job file."
            )
        ));
        for form in &data.job_retry_forms {
            body.push_str(&format!(
                "<section class=\"panel\"><h3 class=\"panel-heading\">Retry {}</h3>{}</section>",
                html_escape(&form.target),
                action_form(form)
            ));
        }
    }
    body.push_str(&format!(
        "<p class=\"panel-body\">{} <a href=\"/control/system/policies\">Open policies and release health</a></p>",
        html_escape("Read-only diagnostics: restarts, secret rotation and fleet pauses are separate scoped actions.")
    ));
    document(meta, "System", &["System"], &body)
}

/// The policies page data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PoliciesData {
    /// The verified-operation matrix rows.
    pub readiness: Vec<ReadinessRow>,
    /// The reason the evidence file could not be read, when any.
    pub evidence_problem: Option<String>,
    /// Pending drafts.
    pub pending: Result<Vec<PolicyRecord>, String>,
    /// Active versions.
    pub active: Result<Vec<PolicyRecord>, String>,
    /// Recent interventions.
    pub interventions: Result<Vec<InterventionRecord>, String>,
    /// Whether scheduling is currently paused.
    pub fleet_paused: bool,
    /// Release-health rows.
    pub release_health: Vec<ReleaseHealthRow>,
    /// Scoped POST forms (policy draft, policy activate, fleet pause/resume).
    pub forms: Vec<ActionFormSpec>,
}

/// Render the policies page.
pub fn render_policies(meta: &PageMeta, data: &PoliciesData) -> String {
    let mut body = header(meta, "Policies, operations and release health");
    body.push_str("<h2 class=\"section-heading\">Verified operation matrix</h2>");
    if let Some(problem) = &data.evidence_problem {
        body.push_str(&notice(
            StatusKind::Unavailable,
            copy::TELEMETRY_ABSENT,
            &format!("The operation evidence file could not be read: {problem}"),
            None,
        ));
    }
    let cells: Vec<Vec<Cell>> = data
        .readiness
        .iter()
        .map(|row| {
            vec![
                Cell::Text(format!("{} / {}", row.platform, row.action)),
                Cell::Text(row.adapter_implemented.clone()),
                Cell::Status(
                    row.sandbox_evidence.clone(),
                    match row.sandbox_evidence.as_str() {
                        "Confirmed" => StatusKind::Verified,
                        "Not on file" => StatusKind::Unavailable,
                        _ => StatusKind::Attention,
                    },
                ),
                Cell::Text(row.allowed_deployment.clone()),
                Cell::Text(row.credential_scope.clone()),
                Cell::Text(row.approval_requirement.clone()),
                Cell::Text(row.release_version.clone()),
                Cell::Status(
                    if row.available {
                        "Available"
                    } else {
                        "Unavailable"
                    }
                    .to_string(),
                    if row.available {
                        StatusKind::Verified
                    } else {
                        StatusKind::Unavailable
                    },
                ),
            ]
        })
        .collect();
    body.push_str(&data_table(
        Some("Platform and action readiness from the connector layer and the evidence file"),
        &[
            "Platform / action",
            "Adapter implemented",
            "Sandbox evidence",
            "Allowed deployment",
            "Credential scope",
            "Approval requirement",
            "Release version",
            "Availability",
        ],
        &cells,
        Some(EmptyState {
            reason: "No connector operation metadata is registered in this build.".to_string(),
            action: None,
        }),
        Some("Missing evidence means the action remains unavailable."),
    ));
    body.push_str("<h2 class=\"section-heading\">Policy versioning</h2>");
    body.push_str(&format!(
        "<p class=\"table-note\">{}</p>",
        html_escape(
            "A draft is a versioned preview of caps, windows and thresholds. Activation names the author, the preview and the activation time; future proposals are affected and active experiments keep their registered policy."
        )
    ));
    let pending_rows: Vec<Vec<Cell>> = data
        .pending
        .as_ref()
        .map(|records| {
            records
                .iter()
                .map(|record| {
                    vec![
                        Cell::Mono(record.policy_id.clone()),
                        Cell::Num(record.version.to_string()),
                        Cell::Text(record.author.clone()),
                        Cell::Text(record.affected_proposals.clone()),
                        Cell::Status("Pending".to_string(), StatusKind::Queued),
                    ]
                })
                .collect()
        })
        .unwrap_or_default();
    body.push_str(&data_table(
        Some("Pending policy drafts"),
        &["Policy", "Version", "Author", "Affects", "State"],
        &pending_rows,
        Some(EmptyState {
            reason: "No policy draft is pending.".to_string(),
            action: None,
        }),
        None,
    ));
    match &data.active {
        Ok(active) if !active.is_empty() => {
            body.push_str(&format!(
                "<p class=\"panel-body\">Active version {} (activated by {} at {}).</p>",
                active
                    .last()
                    .map(|record| record.version.to_string())
                    .unwrap_or_default(),
                html_escape(
                    active
                        .last()
                        .and_then(|record| record.activated_by.as_deref())
                        .unwrap_or("not recorded")
                ),
                html_escape(
                    &active
                        .last()
                        .and_then(|record| record.activated_at_ms)
                        .map(apex_ui::format::utc_datetime)
                        .unwrap_or_else(|| "not recorded".to_string())
                )
            ));
        }
        Ok(_) => {
            body.push_str(&format!(
                "<p class=\"table-note\">{}</p>",
                html_escape("No policy version is active; the conservative defaults apply.")
            ));
        }
        Err(reason) => {
            body.push_str(&notice(
                StatusKind::Unavailable,
                "The policy register could not be read.",
                reason,
                None,
            ));
        }
    }
    for form in &data.forms {
        body.push_str(&format!(
            "<section class=\"panel\"><h3 class=\"panel-heading\">{}</h3>{}</section>",
            html_escape(&form.label),
            action_form(form)
        ));
    }
    body.push_str("<h2 class=\"section-heading\">Fleet interventions</h2>");
    body.push_str(&format!(
        "<p class=\"table-note\">Scheduling is currently {}. Reads continue independently of scheduling.</p>",
        html_escape(if data.fleet_paused { "paused" } else { "running" })
    ));
    let intervention_rows: Vec<Vec<Cell>> = data
        .interventions
        .as_ref()
        .map(|records| {
            records
                .iter()
                .rev()
                .map(|record| {
                    vec![
                        Cell::Text(apex_ui::format::utc_datetime(record.at_ms)),
                        Cell::Mono(record.intervention_id.clone()),
                        Cell::Text(record.kind.replace('_', " ")),
                        Cell::Text(
                            record
                                .platform
                                .clone()
                                .unwrap_or_else(|| "all platforms".to_string()),
                        ),
                        Cell::Text(record.customer_population.clone()),
                        Cell::Text(record.reason.clone()),
                        Cell::Text(record.expected_effect.clone()),
                    ]
                })
                .collect()
        })
        .unwrap_or_default();
    body.push_str(&data_table(
        Some("Recent interventions"),
        &[
            "Recorded",
            "Id",
            "Kind",
            "Platform",
            "Population",
            "Reason",
            "Expected effect",
        ],
        &intervention_rows,
        Some(EmptyState {
            reason: "No fleet intervention is recorded.".to_string(),
            action: None,
        }),
        None,
    ));
    body.push_str("<h2 class=\"section-heading\">Release health</h2>");
    let release_rows: Vec<Vec<Cell>> = data
        .release_health
        .iter()
        .map(|row| {
            vec![
                Cell::Text(row.service.clone()),
                Cell::Text(row.version.clone()),
                Cell::Text(row.migration_state.clone()),
                Cell::Text(row.report_schema.clone()),
                Cell::Text(row.metric_schema.clone()),
                Cell::Text(row.data_quality.clone()),
                Cell::Text(row.recent_errors.clone()),
            ]
        })
        .collect();
    body.push_str(&data_table(
        Some("Deployed versions, migrations and schema versions"),
        &[
            "Service",
            "Version",
            "Migration state",
            "Report schema",
            "Metric schema",
            "Data quality",
            "Recent errors",
        ],
        &release_rows,
        None,
        Some("Not instrumented stands where a version label is missing; nothing is inferred."),
    ));
    document(
        meta,
        "Policies and release health",
        &["System", "Policies"],
        &body,
    )
}
