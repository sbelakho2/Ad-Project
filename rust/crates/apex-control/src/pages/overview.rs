//! Operations overview `/control/` (page 56): impact before uptime.
//!
//! KPIs are computed from the queue rows themselves, and the priority table
//! is exactly that row population sorted by harm and age, so every KPI
//! number equals the count of its rows (the summary-to-row parity rule).

use apex_analytics::projections::OperatorQueueRow;
use apex_analytics::queries::{FeeRow, FeeState, OperatorQueues};
use apex_ui::components::action;
use apex_ui::components::{data_table, notice, Cell, EmptyState};
use apex_ui::escape::html_escape;
use apex_ui::format::money;
use apex_ui::status::StatusKind;

use crate::copy;

use super::{document, header, kpis, PageMeta};

/// One priority queue row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PriorityRow {
    /// Severity (`critical`/`warning`).
    pub severity: String,
    /// Age in seconds.
    pub age_seconds: u64,
    /// Customer (publisher identity).
    pub publisher_id: String,
    /// The issue line.
    pub issue: String,
    /// The owner and next step line.
    pub owner_next_step: String,
    /// The object link, when the object has a page.
    pub href: Option<String>,
}

/// The overview page data: the queues and fee rows, or the reason each read
/// is unavailable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OverviewData {
    /// The operator queues.
    pub queues: Result<OperatorQueues, String>,
    /// Fee rows (for the blocked-fees KPI).
    pub fee_rows: Result<Vec<FeeRow>, String>,
}

/// The priority rows of the overview, sorted by harm and age.
pub fn priority_rows(queues: &OperatorQueues) -> Vec<PriorityRow> {
    let mut rows: Vec<(u8, u64, PriorityRow)> = Vec::new();
    let mut push = |rank: u8, age: u64, row: PriorityRow| rows.push((rank, age, row));
    for row in &queues.incidents {
        push(
            severity_rank(&row.severity),
            row.age_seconds,
            PriorityRow {
                severity: row.severity.clone(),
                age_seconds: row.age_seconds,
                publisher_id: row.publisher_id.clone(),
                issue: format!("{}: {}", row.object_id, humanize(&row.next_action)),
                owner_next_step: owner_next_step(row),
                href: Some(format!("/control/incidents/{}", row.object_id)),
            },
        );
    }
    for row in &queues.due_decisions {
        push(
            2,
            row.age_seconds,
            PriorityRow {
                severity: "warning".to_string(),
                age_seconds: row.age_seconds,
                publisher_id: row.publisher_id.clone(),
                issue: format!("{}: decision window complete", row.object_id),
                owner_next_step: owner_next_step(row),
                href: Some(format!("/control/experiments/{}", row.object_id)),
            },
        );
    }
    for row in &queues.stale_accounts {
        push(
            3,
            row.age_seconds,
            PriorityRow {
                severity: "warning".to_string(),
                age_seconds: row.age_seconds,
                publisher_id: row.publisher_id.clone(),
                issue: format!(
                    "Reporting key or connection needs attention ({})",
                    row.object_id
                ),
                owner_next_step: owner_next_step(row),
                href: Some("/control/ingestion".to_string()),
            },
        );
    }
    for row in &queues.held_fees {
        push(
            3,
            row.age_seconds,
            PriorityRow {
                severity: "warning".to_string(),
                age_seconds: row.age_seconds,
                publisher_id: row.publisher_id.clone(),
                issue: format!("{}: fee on proof hold", row.object_id),
                owner_next_step: owner_next_step(row),
                href: Some("/control/finance".to_string()),
            },
        );
    }
    for row in &queues.overdue_jobs {
        push(
            4,
            row.age_seconds,
            PriorityRow {
                severity: "warning".to_string(),
                age_seconds: row.age_seconds,
                publisher_id: row.publisher_id.clone(),
                issue: format!("{}: job overdue", row.object_id),
                owner_next_step: owner_next_step(row),
                href: Some("/control/system".to_string()),
            },
        );
    }
    rows.sort_by(|left, right| {
        left.0
            .cmp(&right.0)
            .then(right.1.cmp(&left.1))
            .then(left.2.publisher_id.cmp(&right.2.publisher_id))
            .then(left.2.issue.cmp(&right.2.issue))
    });
    rows.into_iter().map(|(_, _, row)| row).collect()
}

fn severity_rank(severity: &str) -> u8 {
    match severity {
        "critical" => 0,
        "warning" => 1,
        _ => 2,
    }
}

fn owner_next_step(row: &OperatorQueueRow) -> String {
    let owner = row
        .owner
        .clone()
        .unwrap_or_else(|| "Unassigned".to_string());
    format!("{} / {}", owner, humanize(&row.next_action))
}

/// `verify_proof` to `Verify proof`.
pub fn humanize(action: &str) -> String {
    let spaced = action.replace('_', " ");
    let mut chars = spaced.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// `480` to `8m`, `7200` to `2h`, `259200` to `3d`.
pub fn age_label(seconds: u64) -> String {
    if seconds >= 86_400 {
        format!("{}d", seconds / 86_400)
    } else if seconds >= 3_600 {
        format!("{}h", seconds / 3_600)
    } else if seconds >= 60 {
        format!("{}m", seconds / 60)
    } else {
        format!("{seconds}s")
    }
}

/// The blocked-fee totals per currency: `(currency, micros, row count)`.
pub fn held_totals(fee_rows: &[FeeRow]) -> Vec<(String, i64, usize)> {
    let mut totals: Vec<(String, i64, usize)> = Vec::new();
    for row in fee_rows.iter().filter(|row| row.state == FeeState::Held) {
        let Some(amount) = &row.amount else {
            continue;
        };
        match totals
            .iter_mut()
            .find(|(currency, _, _)| *currency == amount.currency)
        {
            Some((_, micros, count)) => {
                *micros = micros.saturating_add(amount.micros);
                *count += 1;
            }
            None => totals.push((amount.currency.clone(), amount.micros, 1)),
        }
    }
    totals.sort_by(|left, right| left.0.cmp(&right.0));
    totals
}

/// Render the overview.
pub fn render(meta: &PageMeta, data: &OverviewData) -> String {
    let mut body = header(meta, "What needs attention");
    match &data.queues {
        Err(reason) => {
            body.push_str(&notice(
                StatusKind::Unavailable,
                copy::TELEMETRY_ABSENT,
                &format!("The operator queues are not readable: {reason}"),
                Some((copy::TELEMETRY_ABSENT_ACTION, "/control/system")),
            ));
        }
        Ok(queues) => {
            body.push_str(&render_queues(meta, queues, &data.fee_rows));
        }
    }
    document(meta, "Operations", &["Overview"], &body)
}

fn render_queues(
    meta: &PageMeta,
    queues: &OperatorQueues,
    fee_rows: &Result<Vec<FeeRow>, String>,
) -> String {
    let mut out = String::new();
    let affected: Vec<String> = queues
        .incidents
        .iter()
        .map(|row| row.publisher_id.clone())
        .filter(|publisher| !publisher.is_empty())
        .collect();
    let mut distinct: Vec<String> = affected.clone();
    distinct.sort();
    distinct.dedup();
    let critical = queues
        .incidents
        .iter()
        .filter(|row| row.severity == "critical")
        .count();
    let warning = queues
        .incidents
        .iter()
        .filter(|row| row.severity == "warning")
        .count();
    let stale = queues.stale_accounts.len();
    let overdue = queues.overdue_jobs.len();
    let oldest_overdue = queues
        .overdue_jobs
        .iter()
        .map(|row| row.age_seconds)
        .max()
        .map(age_label)
        .unwrap_or_else(|| "0m".to_string());

    let held = fee_rows
        .as_ref()
        .map(|rows| held_totals(rows))
        .unwrap_or_default();
    let (held_value, held_note) = match (&fee_rows, held.as_slice()) {
        (Err(reason), _) => (
            apex_ui::format::or_not_available(reason),
            "The fee read is unavailable".to_string(),
        ),
        (Ok(_), []) => (
            "USD 0.00".to_string(),
            "No held fee lines (a known denominator)".to_string(),
        ),
        (Ok(_), [(currency, micros, count)]) if held.len() == 1 => (
            money(*micros, currency),
            format!(
                "{}, proof verification hold",
                super::count(*count, "held fee line", "held fee lines")
            ),
        ),
        (Ok(_), totals) => (
            format!("{} currencies", totals.len()),
            "Held fee totals per currency are split; inspect the finance page".to_string(),
        ),
    };

    out.push_str(&kpis(&[
        (
            "Affected customers",
            &distinct.len().to_string(),
            &format!("{critical} critical / {warning} warning"),
            Some("/control/customers"),
        ),
        (
            "Stale reporting",
            &stale.to_string(),
            "Last final day > expected",
            Some("/control/ingestion"),
        ),
        (
            "Blocked fees",
            &held_value,
            &held_note,
            Some("/control/finance"),
        ),
        (
            "Jobs overdue",
            &overdue.to_string(),
            &format!("Oldest delayed {oldest_overdue}"),
            Some("/control/system"),
        ),
    ]));

    out.push_str("<h2 class=\"section-heading\">Priority queue</h2>");
    let rows = priority_rows(queues);
    if rows.is_empty() {
        out.push_str(&notice(
            StatusKind::Verified,
            copy::EMPTY_OPERATIONS_HEADLINE,
            &copy::EMPTY_OPERATIONS_BODY.replace("{as_of}", &meta.as_of_display()),
            Some((copy::EMPTY_OPERATIONS_ACTION, "/control/ingestion")),
        ));
    } else {
        let cells: Vec<Vec<Cell>> = rows
            .iter()
            .map(|row| {
                vec![
                    Cell::Status(
                        format!(
                            "{} / {}",
                            capitalize(&row.severity),
                            age_label(row.age_seconds)
                        ),
                        apex_ui::status::severity(&row.severity),
                    ),
                    Cell::Text(row.publisher_id.clone()),
                    Cell::Text(row.issue.clone()),
                    Cell::Text(row.owner_next_step.clone()),
                ]
            })
            .collect();
        let count_note = format!(
            "{} / as of {}",
            super::count(rows.len(), "unresolved item", "unresolved items"),
            meta.as_of_display()
        );
        out.push_str(&data_table(
            Some("Unresolved work, sorted by harm and age"),
            &["Priority / age", "Customer", "Issue", "Owner / next step"],
            &cells,
            Some(EmptyState {
                reason: copy::EMPTY_OPERATIONS_HEADLINE.to_string(),
                action: Some((
                    copy::EMPTY_OPERATIONS_ACTION.to_string(),
                    "/control/ingestion".to_string(),
                )),
            }),
            Some(&count_note),
        ));
    }
    out.push_str(&format!(
        "<p class=\"table-note\">{}</p>",
        html_escape("Acknowledged is not resolved. Each item has an owner and a next check.")
    ));
    out.push_str(&format!(
        "<p class=\"table-note\">{}</p>",
        html_escape("Blocked fees are a held amount, not lost revenue. Count affected customers once, with the incident population stated.")
    ));
    out.push_str(&format!(
        "<p class=\"panel-body\">{}</p>",
        action("Open incidents", "/control/incidents", false)
    ));
    out
}

fn capitalize(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::DemoData;

    fn meta() -> PageMeta {
        PageMeta {
            environment: "PRODUCTION".to_string(),
            as_of_ms: crate::fixtures::DEMO_NOW_MS,
            operator: Some("Maya".to_string()),
            role: Some(crate::capabilities::Role::Operator),
            fixtures: true,
            csrf: String::new(),
        }
    }

    #[test]
    fn every_kpi_equals_its_row_population() {
        let demo = DemoData::new();
        let queues = demo.queues();
        let fee_rows = demo.fee_rows();
        let data = OverviewData {
            queues: Ok(queues.clone()),
            fee_rows: Ok(fee_rows.clone()),
        };
        let html = render(&meta(), &data);

        // Affected customers: distinct publishers among incident rows.
        let mut publishers: Vec<String> = queues
            .incidents
            .iter()
            .map(|row| row.publisher_id.clone())
            .collect();
        publishers.sort();
        publishers.dedup();
        assert_eq!(publishers.len(), 3);
        assert!(
            html.contains(">3</a>") || html.contains(">3</p>"),
            "affected customer KPI"
        );
        assert!(html.contains("2 critical / 1 warning"));

        // Stale reporting: the stale_accounts rows.
        assert_eq!(queues.stale_accounts.len(), 2);
        assert!(
            html.contains(">2</a>") || html.contains(">2</p>"),
            "stale KPI"
        );

        // Blocked fees: the sum of held rows (300 + 200 USD).
        let totals = held_totals(&fee_rows);
        assert_eq!(totals, vec![("USD".to_string(), 500_000_000, 2)]);
        assert!(html.contains("USD 500.00"));
        assert!(html.contains("2 held fee lines"));

        // Jobs overdue: the overdue_jobs rows.
        assert_eq!(queues.overdue_jobs.len(), 1);
        assert!(html.contains("Oldest delayed 18m"));

        // The priority table has exactly the union of queue rows.
        let rows = priority_rows(&queues);
        let expected = queues.incidents.len()
            + queues.due_decisions.len()
            + queues.stale_accounts.len()
            + queues.held_fees.len()
            + queues.overdue_jobs.len();
        assert_eq!(rows.len(), expected);
        assert!(html.contains(&format!(
            "{expected} unresolved item{}",
            if expected == 1 { "" } else { "s" }
        )));
        assert_eq!(rows[0].severity, "critical");
        assert_eq!(rows[0].publisher_id, "northstar");
    }

    #[test]
    fn the_empty_state_is_the_page_97_copy() {
        let empty = OperatorQueues {
            as_of: "2026-10-15T09:10:00.000Z".to_string(),
            incidents: Vec::new(),
            due_decisions: Vec::new(),
            stale_accounts: Vec::new(),
            overdue_jobs: Vec::new(),
            held_fees: Vec::new(),
        };
        let data = OverviewData {
            queues: Ok(empty),
            fee_rows: Ok(Vec::new()),
        };
        let html = render(&meta(), &data);
        assert!(html.contains("No customer-impact incidents are open."));
        assert!(html.contains("Last evaluated at 15 Oct 2026, 09:10 UTC."));
        assert!(html.contains("Data freshness is shown below."));
        assert!(html.contains("Review data readiness."));
    }

    #[test]
    fn unavailable_queues_render_the_telemetry_absent_copy_with_the_reason() {
        let data = OverviewData {
            queues: Err("the analytics store is not configured".to_string()),
            fee_rows: Err("the analytics store is not configured".to_string()),
        };
        let html = render(&meta(), &data);
        assert!(html.contains("This signal is not instrumented."));
        assert!(html.contains("the analytics store is not configured"));
        assert!(!html.contains(">0</p>"), "no fabricated zeros");
    }

    #[test]
    fn age_labels_are_compact() {
        assert_eq!(age_label(45), "45s");
        assert_eq!(age_label(480), "8m");
        assert_eq!(age_label(7_200), "2h");
        assert_eq!(age_label(259_200), "3d");
        assert_eq!(age_label(1_000_000), "11d");
    }
}
