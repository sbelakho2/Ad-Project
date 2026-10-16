//! The error and refusal pages: 404, 400, 409, 503, and the "not available"
//! data state. Every one carries a reason and a way back.

use apex_ui::components::{action, notice, panel};
use apex_ui::status::StatusKind;

use crate::pages::shared::{self, PageMeta};

/// A 404: the address or record does not exist here.
pub fn not_found(as_of_secs: i64, data_location: &str) -> String {
    let meta = PageMeta::new(
        "Not found",
        "Overview",
        Vec::new(),
        as_of_secs,
        data_location,
        "",
    );
    let body = format!(
        "{}\n{}",
        notice(
            StatusKind::Neutral,
            "There is nothing at this address",
            "The record does not exist in this deployment's engine state, or the address is not a sales workspace route.",
            None,
        ),
        action("Back to overview", "/", true)
    );
    shared::page(&meta, &body)
}

/// A 400: the submitted form could not be accepted.
pub fn bad_request(as_of_secs: i64, data_location: &str, reason: &str) -> String {
    let meta = PageMeta::new(
        "Request refused",
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
            "The request was refused",
            reason,
            None,
        ),
        "<p class=\"table-note\">Nothing was written. Use the browser's back action to fix the form; a fresh idempotency key is minted when the page renders again.</p>"
    );
    shared::page(&meta, &body)
}

/// A 409: the idempotency key was already used for a different action.
pub fn conflict(as_of_secs: i64, data_location: &str) -> String {
    let meta = PageMeta::new(
        "Key conflict",
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
            "That form key was already used for a different action",
            "A replayed form must carry the same payload as its first submission. Nothing was written for this conflicting submission.",
            None,
        ),
        action("Back to overview", "/", true)
    );
    shared::page(&meta, &body)
}

/// A 503: the engine state could not be read.
pub fn data_unavailable(as_of_secs: i64, data_location: &str, reason: &str) -> String {
    let meta = PageMeta::new(
        "Data not available",
        "Overview",
        Vec::new(),
        as_of_secs,
        data_location,
        "",
    );
    let body = format!(
        "{}\n{}",
        notice(
            StatusKind::Unavailable,
            "The engine state could not be read",
            &format!("Not available ({reason})"),
            None,
        ),
        panel(
            "What can still be checked",
            "<p>The state directory path comes from <code>APEX_SALES_DIR</code> (default <code>.apex/sales</code>). The CLI (<code>apex-sales report</code>) reads the same directory and prints the same failure verbatim.</p>",
            None,
        )
    );
    shared::page(&meta, &body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_error_page_has_one_h1_and_a_reason() {
        for html in [
            not_found(0, ".apex/sales"),
            bad_request(0, ".apex/sales", "Not available (x)"),
            conflict(0, ".apex/sales"),
            data_unavailable(0, ".apex/sales", "no such file"),
        ] {
            assert_eq!(html.matches("<h1").count(), 1);
            assert!(html.contains("Scope: sales workspace"));
        }
    }

    #[test]
    fn the_bad_request_page_escapes_the_reason() {
        let html = bad_request(0, ".apex/sales", "<script>alert(1)</script>");
        assert!(!html.contains("<script>"));
        assert!(html.contains("&lt;script&gt;"));
    }
}
