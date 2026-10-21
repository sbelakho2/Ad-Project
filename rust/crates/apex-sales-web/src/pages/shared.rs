//! Shared page scaffolding: the page meta line every page shows, the
//! document frame and small status/count helpers.

use apex_ui::components::Cell;
use apex_ui::escape::html_escape;
use apex_ui::format::utc_datetime;
use apex_ui::shell::{Area, PageCtx, Theme};
use apex_ui::status::StatusKind;

use crate::render;

/// Everything a sales page shows about itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageMeta {
    /// The page title (document title and `h1`).
    pub title: String,
    /// The navigation label whose rail entry is marked current.
    pub nav: &'static str,
    /// The extra breadcrumb parts after `Sales` and `nav`.
    pub trail: Vec<String>,
    /// The as-of time in Unix milliseconds (UTC display).
    pub as_of_ms: i64,
    /// Where the data lives (the engine state directory).
    pub data_location: String,
    /// The per-session CSRF token for forms on the page (empty when the
    /// page has no forms).
    pub csrf: String,
}

impl PageMeta {
    /// Build a page meta from an engine as-of time in Unix seconds.
    pub fn new(
        title: impl Into<String>,
        nav: &'static str,
        trail: Vec<String>,
        as_of_secs: i64,
        data_location: impl Into<String>,
        csrf: impl Into<String>,
    ) -> Self {
        Self {
            title: title.into(),
            nav,
            trail,
            as_of_ms: as_of_secs.saturating_mul(1000),
            data_location: data_location.into(),
            csrf: csrf.into(),
        }
    }

    /// The breadcrumb: `Sales`, the nav label, then the trail.
    pub fn breadcrumb(&self) -> Vec<String> {
        let mut parts = vec!["Sales".to_string(), self.nav.to_string()];
        parts.extend(self.trail.iter().cloned());
        parts
    }
}

/// The store name safe to print in a page: a relative path is shown as
/// written, while an absolute deployment path keeps only its final segment.
pub fn store_label(path: &std::path::Path) -> String {
    if path.is_absolute() {
        path.file_name()
            .map(|name| format!("…/{}", name.to_string_lossy()))
            .unwrap_or_else(|| "configured store".to_string())
    } else {
        path.display().to_string()
    }
}

/// The standard meta line: scope, as-of (UTC) and data location.
pub fn meta_line(meta: &PageMeta) -> String {
    format!(
        "<p class=\"table-note\">Scope: sales workspace / as of {} / data: {}</p>",
        utc_datetime(meta.as_of_ms),
        html_escape(&meta.data_location)
    )
}

/// Render the full document for a page body: the `h1`, the meta line and the
/// body, wrapped in the shell with the one stylesheet link.
pub fn page(meta: &PageMeta, body: &str) -> String {
    let ctx = PageCtx {
        title: meta.title.clone(),
        area: Area::SalesPlane,
        publisher_label: None,
        breadcrumb: meta.breadcrumb(),
        read_only: false,
        theme: Theme::Auto,
        help_href: Some(apex_ui::shell::DEFAULT_SUPPORT_URL.to_string()),
        marketing_header: false,
    };
    let sign_out = if meta.csrf.is_empty() {
        String::new()
    } else {
        format!(
            "\n<form method=\"post\" action=\"/logout\" class=\"inline-form\">\
             {}<button type=\"submit\" class=\"action action-secondary\">Sign out</button></form>",
            apex_ui::components::csrf_hidden(&meta.csrf)
        )
    };
    let body = format!(
        "<h1 class=\"page-title\">{}</h1>\n{}\n{body}{sign_out}",
        html_escape(&meta.title),
        meta_line(meta)
    );
    render::document(&ctx, &body)
}

/// A count note in the house form: `Showing X of Y contacts`.
pub fn count_note(shown: usize, total: usize, unit: &str) -> String {
    if shown == total {
        format!("{total} {unit}")
    } else {
        format!("Showing {shown} of {total} {unit}")
    }
}

/// A rate as `n/N` plus the one-decimal percentage: `3/40 (7.5%)`.
pub fn rate(k: usize, n: usize) -> String {
    if n == 0 {
        return apex_ui::format::or_not_available("no observations");
    }
    let value = k as f64 / n as f64;
    format!("{k}/{n} ({})", apex_ui::format::percent(value))
}

/// A rate as `n/N` with fractional counts kept to one decimal: used for the
/// health module's discounted counts.
pub fn rate_float(k: f64, n: f64) -> String {
    if n <= 0.0 {
        return apex_ui::format::or_not_available("no observations");
    }
    let percentage = apex_ui::format::percent(k / n);
    format!("{k:.1}/{n:.1} ({percentage})")
}

/// A yes/no cell.
pub fn yes_no(value: bool) -> Cell {
    Cell::Text(if value { "Yes" } else { "No" }.to_string())
}

/// A status cell with an explicit label.
pub fn status_cell(label: &str, kind: StatusKind) -> Cell {
    Cell::Status(label.to_string(), kind)
}

/// A stable link cell.
pub fn link_cell(label: &str, href: &str) -> Cell {
    Cell::Link(label.to_string(), href.to_string())
}

/// The standard "back to overview" action for result pages.
pub fn page_back_action() -> String {
    apex_ui::components::action("Back to overview", "/", false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pages_carry_one_h1_and_the_meta_line() {
        let meta = PageMeta::new(
            "Overview",
            "Overview",
            Vec::new(),
            1_791_965_400,
            ".apex/sales",
            "",
        );
        let html = page(&meta, "<p>body</p>");
        assert_eq!(html.matches("<h1").count(), 1);
        assert!(html.contains("<h1 class=\"page-title\">Overview</h1>"));
        assert!(html
            .contains("Scope: sales workspace / as of 14 Oct 2026, 08:10 UTC / data: .apex/sales"));
        assert!(html.contains("<p>body</p>"));
        assert_eq!(html.matches("<link rel=\"stylesheet\"").count(), 1);
    }

    #[test]
    fn meta_escapes_the_data_location() {
        let meta = PageMeta::new(
            "Overview",
            "Overview",
            Vec::new(),
            0,
            "<script>alert(1)</script>",
            "",
        );
        let html = page(&meta, "");
        assert!(!html.contains("<script>"));
        assert!(html.contains("&lt;script&gt;"));
    }

    #[test]
    fn counts_and_rates_follow_the_house_style() {
        assert_eq!(count_note(3, 3, "contacts"), "3 contacts");
        assert_eq!(count_note(3, 40, "contacts"), "Showing 3 of 40 contacts");
        assert_eq!(rate(3, 40), "3/40 (7.5%)");
        assert_eq!(rate(0, 0), "Not available (no observations)");
        assert_eq!(rate_float(1.5, 30.0), "1.5/30.0 (5.0%)");
        assert_eq!(rate_float(0.0, 0.0), "Not available (no observations)");
    }

    #[test]
    fn absolute_store_paths_keep_only_their_last_segment() {
        assert_eq!(
            store_label(std::path::Path::new("/srv/private/sales")),
            "…/sales"
        );
        assert_eq!(
            store_label(std::path::Path::new(".apex/sales")),
            ".apex/sales"
        );
    }
}
