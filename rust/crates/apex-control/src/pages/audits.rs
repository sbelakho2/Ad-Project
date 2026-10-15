//! Audits `/control/audits` (page 60): eligibility before conclusions.

use apex_ui::components::{data_table, notice, Cell, EmptyState};
use apex_ui::escape::html_escape;
use apex_ui::status::StatusKind;

use crate::copy;
use crate::fixtures::AuditQuality;

use super::{document, header, kpis, PageMeta};

/// The audits page data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditsData {
    /// The audit-quality snapshot, or the reason it is unavailable.
    pub quality: Result<AuditQuality, String>,
}

/// Render the audits page.
pub fn render(meta: &PageMeta, data: &AuditsData) -> String {
    let mut body = header(meta, "Audits");
    match &data.quality {
        Err(reason) => {
            body.push_str(&notice(
                StatusKind::Unavailable,
                copy::TELEMETRY_ABSENT,
                &format!("The audit-run read is not available in this deployment: {reason}"),
                Some((copy::TELEMETRY_ABSENT_ACTION, "/control/system")),
            ));
        }
        Ok(quality) => {
            body.push_str(&kpis(&[
                (
                    "Successful runs",
                    &quality.successful_runs,
                    "Completed without an internal failure",
                    None,
                ),
                (
                    "Eligible rule checks",
                    &quality.eligible_checks,
                    "A rule that could not run is not a clean check",
                    None,
                ),
                (
                    "Critical findings",
                    &quality.critical_findings.to_string(),
                    "Require review before publication",
                    None,
                ),
                (
                    "Affected customers",
                    &quality.critical_customers.to_string(),
                    "Customers with at least one critical finding",
                    Some("/control/customers"),
                ),
            ]));
            body.push_str(&format!(
                "<p class=\"table-note\">{}</p>",
                html_escape(&format!(
                    "Run eligibility: {}. {}",
                    quality.ineligible_note,
                    "Unreviewed findings carry no false-positive claim: model calibration is stated per rule and review state."
                ))
            ));
            let rule_rows: Vec<Vec<Cell>> = quality
                .rules
                .iter()
                .map(|rule| {
                    vec![
                        Cell::Text(rule.rule.clone()),
                        Cell::Text(rule.eligible_runs.clone()),
                        Cell::Num(rule.findings.to_string()),
                        Cell::Text(rule.review_signal.clone()),
                    ]
                })
                .collect();
            body.push_str(&data_table(
                Some("Per-rule eligibility and review signals"),
                &["Rule", "Eligible runs", "Findings", "Review signal"],
                &rule_rows,
                Some(EmptyState {
                    reason: copy::AUDIT_NO_INPUTS.to_string(),
                    action: Some((
                        copy::AUDIT_NO_INPUTS_ACTION.to_string(),
                        "/control/ingestion".to_string(),
                    )),
                }),
                Some("A missing input is listed by rule; the rule stayed ineligible."),
            ));
        }
    }
    body.push_str(&format!(
        "<p class=\"table-note\">{}</p>",
        html_escape(
            "Eligibility is the denominator: a rule whose inputs are missing is shown as not run, never as passing."
        )
    ));
    document(meta, "Audits", &["Audits"], &body)
}
