//! Growth `/control/growth` (page 69): funnel with cohort maturity, no fake
//! attribution.

use apex_analytics::queries::{ActivationFunnel, GrowthCohortRow};
use apex_ui::components::{data_table, notice, Cell, EmptyState};
use apex_ui::escape::html_escape;
use apex_ui::status::StatusKind;

use crate::copy;

use super::{document, header, kpis, PageMeta};

/// The growth page data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrowthData {
    /// Cohorts by request month.
    pub cohorts: Result<Vec<GrowthCohortRow>, String>,
    /// The selected cohort (URL parameter).
    pub selected_cohort: Option<String>,
    /// The funnel of the selected cohort.
    pub funnel: Result<ActivationFunnel, String>,
    /// Site requests over the last 30 days from the edge access log, when the
    /// analytics reader answered. `None` renders nothing: an unavailable read
    /// is not a zero.
    pub site_requests: Option<u64>,
}

/// Render the growth page.
pub fn render(meta: &PageMeta, data: &GrowthData) -> String {
    let mut body = header(meta, "Growth");
    match &data.cohorts {
        Err(reason) => {
            body.push_str(&notice(
                StatusKind::Unavailable,
                copy::INCOMPLETE_SOURCE,
                &format!("The cohort read is not available: {reason}"),
                Some((copy::INCOMPLETE_SOURCE_ACTION, "/control/ingestion")),
            ));
        }
        Ok(cohorts) if cohorts.is_empty() => {
            body.push_str(&notice(
                StatusKind::Unavailable,
                copy::INCOMPLETE_SOURCE,
                "No request cohort is recorded yet.",
                Some((copy::INCOMPLETE_SOURCE_ACTION, "/control/ingestion")),
            ));
        }
        Ok(cohorts) => {
            let selected = data
                .selected_cohort
                .clone()
                .or_else(|| {
                    cohorts
                        .iter()
                        .rfind(|cohort| cohort.mature)
                        .map(|cohort| cohort.cohort_month.clone())
                })
                .or_else(|| cohorts.last().map(|cohort| cohort.cohort_month.clone()))
                .unwrap_or_default();
            body.push_str(&cohort_tabs(cohorts, &selected));
            match &data.funnel {
                Err(reason) => {
                    body.push_str(&notice(
                        StatusKind::Unavailable,
                        copy::INCOMPLETE_SOURCE,
                        &format!("The funnel for {selected} is not available: {reason}"),
                        Some((copy::INCOMPLETE_SOURCE_ACTION, "/control/ingestion")),
                    ));
                }
                Ok(funnel) => {
                    body.push_str(&render_funnel(funnel));
                }
            }
            if let Some(count) = data.site_requests {
                body.push_str(&format!(
                    "<p class=\"table-note\">Site requests, last 30 days: {count} (edge access log)</p>"
                ));
            }
            body.push_str(&render_cohort_table(cohorts, &selected));
        }
    }
    body.push_str(&format!(
        "<p class=\"table-note\">{}</p>",
        html_escape(
            "The measurable sources are the request table and the typed lifecycle events. Access-log boundaries limit what can be attributed; no source is inferred."
        )
    ));
    document(meta, "Growth", &["Growth"], &body)
}

fn cohort_tabs(cohorts: &[GrowthCohortRow], selected: &str) -> String {
    let mut out = String::from("<nav class=\"tabs\" aria-label=\"Request cohorts\">");
    for cohort in cohorts.iter().rev() {
        let current = if cohort.cohort_month == selected {
            " aria-current=\"page\""
        } else {
            ""
        };
        out.push_str(&format!(
            "<a class=\"tab\" href=\"/control/growth?cohort={}\"{}>{}{}</a>",
            apex_ui::escape::attr(&cohort.cohort_month),
            current,
            html_escape(&cohort.cohort_month),
            if cohort.mature { "" } else { " (maturing)" }
        ));
    }
    out.push_str("</nav>");
    out
}

fn render_funnel(funnel: &ActivationFunnel) -> String {
    let mut body = String::new();
    if !funnel.mature {
        body.push_str(&notice(
            StatusKind::Attention,
            copy::INCOMPLETE_SOURCE,
            "The cohort has not cleared the 30-day follow-up horizon; its rates are shown for coverage, not for comparison.",
            Some((copy::INCOMPLETE_SOURCE_ACTION, "/control/ingestion")),
        ));
    }
    let requested = funnel
        .stages
        .iter()
        .find(|stage| stage.stage == "requested")
        .map(|stage| stage.accounts);
    let rows: Vec<Vec<Cell>> = funnel
        .stages
        .iter()
        .map(|stage| {
            let share = requested
                .filter(|requested| *requested > 0)
                .map(|requested| apex_ui::format::percent(stage.accounts as f64 / requested as f64))
                .unwrap_or_else(|| apex_ui::format::or_not_available("no requested count"));
            vec![
                Cell::Text(stage.stage.replace('_', " ")),
                Cell::Num(stage.accounts.to_string()),
                Cell::Num(share),
                Cell::Muted(next_unblock(&stage.stage)),
            ]
        })
        .collect();
    body.push_str(&kpis(&[
        (
            "Requested",
            &requested
                .map(|count| count.to_string())
                .unwrap_or_else(|| apex_ui::format::or_not_available("no request count")),
            "Cohort requests in the month",
            None,
        ),
        (
            "Paid service",
            &funnel
                .stages
                .iter()
                .find(|stage| stage.stage == "paid")
                .map(|stage| stage.accounts.to_string())
                .unwrap_or_else(|| apex_ui::format::or_not_available("no paid count")),
            "Reached the paid stage",
            Some("/control/finance"),
        ),
        (
            "Maturity",
            if funnel.mature { "30-day cleared" } else { "Maturing" },
            "The 30-day conversion rate uses accounts from the same request cohort with enough follow-up time",
            None,
        ),
    ]));
    body.push_str(&data_table(
        Some(&format!("Funnel: cohort {}", funnel.cohort_month)),
        &["Stage", "Accounts", "Share of requested", "Work to unblock"],
        &rows,
        Some(EmptyState {
            reason: copy::INCOMPLETE_SOURCE.to_string(),
            action: Some((
                copy::INCOMPLETE_SOURCE_ACTION.to_string(),
                "/control/ingestion".to_string(),
            )),
        }),
        Some("A reached stage counts for every later stage."),
    ));
    body
}

fn next_unblock(stage: &str) -> String {
    match stage {
        "qualified" => "Check account readiness inputs".to_string(),
        "connected" => "Complete credential connection".to_string(),
        "audit_delivered" => "Run and deliver the audit".to_string(),
        "paid" => "Resolve fee proof and collection".to_string(),
        _ => "No action".to_string(),
    }
}

fn render_cohort_table(cohorts: &[GrowthCohortRow], selected: &str) -> String {
    let rows: Vec<Vec<Cell>> = cohorts
        .iter()
        .map(|cohort| {
            vec![
                Cell::Link(
                    cohort.cohort_month.clone(),
                    format!(
                        "/control/growth?cohort={}",
                        apex_ui::escape::attr(&cohort.cohort_month)
                    ),
                ),
                Cell::Status(
                    if cohort.mature { "Mature" } else { "Maturing" }.to_string(),
                    if cohort.mature {
                        StatusKind::Verified
                    } else {
                        StatusKind::Queued
                    },
                ),
                Cell::Num(cohort.accepted.to_string()),
                Cell::Num(cohort.qualified.to_string()),
                Cell::Num(cohort.connected.to_string()),
                Cell::Num(cohort.audit_delivered.to_string()),
                Cell::Num(cohort.paid.to_string()),
            ]
        })
        .collect();
    format!(
        "<section class=\"panel\"><h2 class=\"panel-heading\">Cohorts (selected: {})</h2>{}</section>",
        html_escape(selected),
        data_table(
            None,
            &["Cohort", "Maturity", "Requested", "Qualified", "Connected", "Audit delivered", "Paid"],
            &rows,
            None,
            Some("Cohort maturity framing: 30 days of follow-up before rate comparison."),
        )
    )
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
        }
    }

    fn render_cohort(cohort_month: &str) -> String {
        let demo = DemoData::new();
        render(
            &meta(),
            &GrowthData {
                cohorts: Ok(demo.growth_cohorts()),
                selected_cohort: Some(cohort_month.to_string()),
                funnel: demo.activation_funnel(cohort_month).map_err(|e| e.reason),
                site_requests: Some(demo.site_requests_last_30_days()),
            },
        )
    }

    #[test]
    fn site_requests_render_as_one_factual_context_line() {
        let demo = DemoData::new();
        let base = GrowthData {
            cohorts: Ok(demo.growth_cohorts()),
            selected_cohort: Some("2026-08".to_string()),
            funnel: demo.activation_funnel("2026-08").map_err(|e| e.reason),
            site_requests: Some(18_240),
        };
        let html = render(&meta(), &base);
        assert!(
            html.contains("Site requests, last 30 days: 18240 (edge access log)"),
            "{html}"
        );
        assert_eq!(
            html.matches("Site requests, last 30 days:").count(),
            1,
            "exactly one context line"
        );
        // An unavailable read renders nothing, never a fabricated zero.
        let unavailable = GrowthData {
            site_requests: None,
            ..base
        };
        let html = render(&meta(), &unavailable);
        assert!(!html.contains("Site requests, last 30 days:"), "{html}");
        assert!(!html.contains(": 0 ("), "{html}");
    }

    #[test]
    fn funnel_shares_are_ratios_of_requested_rendered_through_percent() {
        // August requested 52: 35 qualified (67.3%), 8 paid (15.4%), the
        // requested stage itself 100.0%.
        let html = render_cohort("2026-08");
        assert!(html.contains("67.3%"), "35 of 52 is 67.3%");
        assert!(html.contains("15.4%"), "8 of 52 is 15.4%");
        assert!(html.contains("100.0%"), "the requested stage is 100.0%");
        assert!(!html.contains("6730.8%"), "the share is never scaled twice");
        assert!(!html.contains("1538.5%"), "the share is never scaled twice");
        // September requested 30: 18 qualified is 60.0%.
        let incomplete = render_cohort("2026-09");
        assert!(incomplete.contains("60.0%"), "18 of 30 is 60.0%");
    }
}
