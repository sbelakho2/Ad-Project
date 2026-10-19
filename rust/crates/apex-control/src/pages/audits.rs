//! Audits `/control/audits` (page 60): eligibility before conclusions.

use apex_ui::components::{data_table, notice, Cell, EmptyState};
use apex_ui::escape::html_escape;
use apex_ui::status::StatusKind;

use crate::copy;
use crate::data::AuditPublisherView;
use crate::fixtures::AuditQuality;

use super::{document, header, kpis, PageMeta};

/// The audits page data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditsData {
    /// The illustrative audit-quality snapshot, or the reason it is
    /// unavailable.
    pub quality: Result<AuditQuality, String>,
    /// Live snapshot inputs and their eligibility, when the deployment has a
    /// runner snapshot root.
    pub live: Option<Result<Vec<AuditPublisherView>, String>>,
}

/// Render the audits page.
pub fn render(meta: &PageMeta, data: &AuditsData) -> String {
    let mut body = header(meta, "Audits");
    if let Some(live) = &data.live {
        body.push_str(&render_live(live));
        body.push_str(&format!(
            "<p class=\"table-note\">{}</p>",
            html_escape(
                "Eligibility is the denominator: a publisher whose snapshot inputs are missing or unreadable is shown with that reason, never as passing or failing an audit."
            )
        ));
        return document(meta, "Audits", &["Audits"], &body);
    }
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

fn render_live(live: &Result<Vec<AuditPublisherView>, String>) -> String {
    match live {
        Err(reason) => notice(
            StatusKind::Unavailable,
            copy::TELEMETRY_ABSENT,
            &format!("The snapshot read is not available in this deployment: {reason}"),
            None,
        ),
        Ok(publishers) => {
            let mut rows = Vec::new();
            for publisher in publishers {
                for platform in &publisher.platforms {
                    let age = platform
                        .age_ms
                        .map(|age_ms| super::overview::age_label((age_ms / 1_000).max(0) as u64))
                        .unwrap_or_else(|| apex_ui::format::or_not_available("age not recorded"));
                    let detail = match (&platform.error, platform.ad_units) {
                        (Some(error), _) => {
                            Cell::Status(format!("Unreadable: {error}"), StatusKind::Unavailable)
                        }
                        (None, Some(units)) => Cell::Text(format!("{units} ad unit(s)")),
                        (None, None) => Cell::Muted("Not parsed".to_string()),
                    };
                    rows.push(vec![
                        Cell::Mono(publisher.publisher.clone()),
                        Cell::Text(platform.platform.clone()),
                        Cell::Text(age),
                        detail,
                        Cell::Status(
                            if platform.error.is_some() {
                                "Not usable".to_string()
                            } else {
                                "Snapshot present".to_string()
                            },
                            if platform.error.is_some() {
                                StatusKind::Unavailable
                            } else {
                                StatusKind::Verified
                            },
                        ),
                    ]);
                }
                let reason = publisher
                    .eligibility_reason
                    .clone()
                    .unwrap_or_else(|| "-".to_string());
                rows.push(vec![
                    Cell::Mono(publisher.publisher.clone()),
                    Cell::Text(format!("Audit: {}", publisher.eligibility)),
                    Cell::Muted(reason),
                    Cell::Muted(String::new()),
                    Cell::Muted(String::new()),
                ]);
            }
            data_table(
                Some("Publisher snapshot inputs"),
                &[
                    "Publisher",
                    "Platform / eligibility",
                    "Snapshot age",
                    "Input",
                    "State",
                ],
                &rows,
                Some(EmptyState {
                    reason: "No runner snapshot is present under the configured root.".to_string(),
                    action: None,
                }),
                Some("Snapshot age is measured from the snapshot's own fetchedAtUnixMs against the console clock."),
            )
        }
    }
}
