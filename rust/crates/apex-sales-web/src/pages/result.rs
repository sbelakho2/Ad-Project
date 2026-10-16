//! Action result pages: what a POST wrote, with the operation id and the
//! confirming read link, or the honest failure state.

use apex_ui::components::{data_table, notice, Cell};
use apex_ui::format::utc_datetime;
use apex_ui::status::StatusKind;

use crate::pages::shared::{self, PageMeta};
use crate::state::ActionResultRecord;

/// The confirmed result of one executed action.
pub fn confirmed(as_of_secs: i64, data_location: &str, record: &ActionResultRecord) -> String {
    let meta = PageMeta::new(
        "Action result",
        "Overview",
        Vec::new(),
        as_of_secs,
        data_location,
        "",
    );
    let rows = vec![
        vec![
            Cell::Text("Operation".to_string()),
            Cell::Mono(record.operation_id.clone()),
        ],
        vec![
            Cell::Text("Action".to_string()),
            Cell::Text(record.action.clone()),
        ],
        vec![
            Cell::Text("Target".to_string()),
            Cell::Mono(record.target.clone()),
        ],
        vec![
            Cell::Text("Recorded at".to_string()),
            Cell::Text(utc_datetime(record.at_ms)),
        ],
        vec![
            Cell::Text("Confirming read".to_string()),
            Cell::Link(record.confirmed_link.clone(), record.confirmed_link.clone()),
        ],
    ];
    let body = format!(
        "{}\n{}\n{}",
        notice(
            StatusKind::Completed,
            "Confirmed",
            &format!(
                "{}. The durable record is written; the confirming read shows it from the engine's own state.",
                record.detail
            ),
            Some(("Open the confirming read", record.confirmed_link.as_str())),
        ),
        data_table(
            Some("What was written"),
            &["Field", "Value"],
            &rows,
            None,
            Some("A replayed form with the same key answers this same result instead of writing again."),
        ),
        shared_link_note(record)
    );
    shared::page(&meta, &body)
}

fn shared_link_note(record: &ActionResultRecord) -> String {
    format!(
        "<p class=\"table-note\">Operation {op} was recorded for {}. The idempotency key that produced it is remembered for the current process lifetime.</p>",
        apex_ui::escape::html_escape(&record.target),
        op = apex_ui::escape::html_escape(&record.operation_id)
    )
}

/// The failed result: the action was refused or the durable write failed;
/// no operation id exists.
pub fn failed(as_of_secs: i64, data_location: &str, reason: &str) -> String {
    let meta = PageMeta::new(
        "Action result",
        "Overview",
        Vec::new(),
        as_of_secs,
        data_location,
        "",
    );
    let body = format!(
        "{}\n{}",
        notice(
            StatusKind::Failure,
            "Failed",
            &format!("No operation id was issued and no durable record is claimed: {reason}"),
            None,
        ),
        shared::page_back_action()
    );
    shared::page(&meta, &body)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record() -> ActionResultRecord {
        ActionResultRecord {
            operation_id: "op_0123456789ab".to_string(),
            action: "suppress".to_string(),
            target: "spam@else.example".to_string(),
            detail: "Suppressed spam@else.example (operator)".to_string(),
            at_ms: 1_791_965_400_000,
            confirmed_link: "/publishers/else.example".to_string(),
        }
    }

    #[test]
    fn the_confirmed_page_shows_the_operation_and_confirming_link() {
        let html = confirmed(1_791_965_400, ".apex/sales", &record());
        assert_eq!(html.matches("<h1").count(), 1);
        assert!(html.contains("Confirmed"));
        assert!(html.contains("op_0123456789ab"));
        assert!(html.contains("href=\"/publishers/else.example\""));
        assert!(html.contains("14 Oct 2026, 08:10 UTC"));
    }

    #[test]
    fn the_failed_page_is_honest_and_escaped() {
        let html = failed(0, ".apex/sales", "<script>x</script>");
        assert!(html.contains("Failed"));
        assert!(!html.contains("<script>"));
        assert!(html.contains("No operation id was issued"));
    }
}
