//! The server-rendered operator pages.
//!
//! Every page is assembled from the frozen design system in `apex-ui`
//! ([`apex_ui::shell`] for the document, [`apex_ui::components`] for KPIs,
//! panels, tables, forms and notices, [`apex_ui::format`] for money and UTC
//! times) and passes every dynamic string through
//! [`apex_ui::escape::html_escape`]. The frame contract of every page:
//!
//! - exactly one `<h1>`, owned by the body;
//! - the operator shell (`Area::OperatorPlane`) with the operations
//!   navigation;
//! - the environment chip (PRODUCTION or TEST), the as-of time and the
//!   signed-in operator in one meta line;
//! - the `ILLUSTRATIVE DATA` banner whenever the read layer is the fixture
//!   dataset;
//! - no `<script>` and no external fetch.

pub mod access;
pub mod actions;
pub mod audits;
pub mod customers;
pub mod errors;
pub mod experiments;
pub mod finance;
pub mod growth;
pub mod incidents;
pub mod ingestion;
pub mod login;
pub mod overview;
pub mod system;

use apex_ui::components::{kpi_card, kpi_row, panel};
use apex_ui::format::utc_datetime;
use apex_ui::shell::{self, Area, PageCtx, Theme};
use apex_ui::status::{chip, StatusKind};

use crate::capabilities::Role;

/// The per-page rendering context: deployment labels, as-of time and the
/// signed-in operator, never credentials.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageMeta {
    /// Environment chip label (`PRODUCTION`/`TEST`).
    pub environment: String,
    /// The as-of instant of every read on this page.
    pub as_of_ms: i64,
    /// Signed-in operator display name, when a session exists.
    pub operator: Option<String>,
    /// Signed-in operator role, when a session exists.
    pub role: Option<Role>,
    /// Whether the page renders the illustrative dataset.
    pub fixtures: bool,
}

impl PageMeta {
    /// The as-of line used by the pages and empty states.
    pub fn as_of_display(&self) -> String {
        utc_datetime(self.as_of_ms)
    }
}

/// Render the full operator document around `body`, with the breadcrumb
/// `Operator / <area...>`.
pub fn document(_meta: &PageMeta, title: &str, breadcrumb: &[&str], body: &str) -> String {
    let mut crumbs = vec!["Operator".to_string()];
    crumbs.extend(breadcrumb.iter().map(|part| part.to_string()));
    shell::page(
        &PageCtx {
            title: title.to_string(),
            area: Area::OperatorPlane,
            publisher_label: None,
            breadcrumb: crumbs,
            read_only: false,
            theme: Theme::Auto,
            help_href: Some("/support/".to_string()),
            marketing_header: false,
        },
        body,
    )
}

/// The environment chip, as-of time and operator line every page carries.
pub fn meta_line(meta: &PageMeta) -> String {
    let environment_kind = if meta.environment == "PRODUCTION" {
        StatusKind::Verified
    } else {
        StatusKind::Neutral
    };
    let mut line = format!(
        "<p class=\"table-note\">{} / As of {}",
        chip(&meta.environment, environment_kind),
        apex_ui::escape::html_escape(&meta.as_of_display())
    );
    if let Some(operator) = &meta.operator {
        line.push_str(&format!(" / {}", apex_ui::escape::html_escape(operator)));
        if let Some(role) = meta.role {
            line.push_str(&format!(
                " ({})",
                apex_ui::escape::html_escape(role.label())
            ));
        }
    }
    line.push_str("</p>");
    line
}

/// The illustrative-data banner shown when the fixture dataset serves.
pub fn fixtures_banner(meta: &PageMeta) -> String {
    if !meta.fixtures {
        return String::new();
    }
    apex_ui::components::notice(
        StatusKind::Attention,
        "PROPOSED DESIGN | ILLUSTRATIVE DATA",
        "These screens render the design document's illustrative dataset, not live account data.",
        None,
    )
}

/// The page header: one h1, the meta line and the fixture banner.
pub fn header(meta: &PageMeta, title: &str) -> String {
    format!(
        "{}{}<h1 class=\"page-title\">{}</h1>",
        meta_line(meta),
        fixtures_banner(meta),
        apex_ui::escape::html_escape(title)
    )
}

/// A KPI row from `(label, value, note, href)` tuples.
pub fn kpis(cards: &[(&str, &str, &str, Option<&str>)]) -> String {
    let rendered: Vec<String> = cards
        .iter()
        .map(|(label, value, note, href)| kpi_card(label, value, note, *href))
        .collect();
    kpi_row(&rendered)
}

/// A two-column split with a main fragment and a side fragment.
pub fn split(main_html: &str, side_html: &str) -> String {
    apex_ui::components::split(main_html, side_html)
}

/// A section panel.
pub fn section(heading: &str, body_html: &str, next_step: Option<&str>) -> String {
    panel(heading, body_html, next_step)
}

/// The static page footer note for read-only diagnostics.
pub fn diagnostics_note(text: &str) -> String {
    format!(
        "<p class=\"table-note\">{}</p>",
        apex_ui::escape::html_escape(text)
    )
}

/// A hidden input carrying an escaped value.
pub fn hidden(name: &str, value: &str) -> String {
    format!(
        "<input type=\"hidden\" name=\"{}\" value=\"{}\">",
        apex_ui::escape::attr(name),
        apex_ui::escape::attr(value)
    )
}

/// One POST action form: the hidden CSRF, idempotency key, stale-state token,
/// target and return path, the required typed fields and the submit button.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionFormSpec {
    /// The action route suffix (`/control/actions/<kind>`).
    pub action: String,
    /// The capability label on the submit button.
    pub label: String,
    /// The object identity.
    pub target: String,
    /// Per-session CSRF token.
    pub csrf: String,
    /// Fresh idempotency key for this render.
    pub idempotency_key: String,
    /// The stale-state token this form was prepared from.
    pub version_token: String,
    /// The object route to return to.
    pub return_path: String,
    /// Whether the reason field is mandatory.
    pub reason_required: bool,
    /// Whether the source document field is shown.
    pub source_document: bool,
    /// Whether the recovery evidence field is shown.
    pub evidence: bool,
    /// The typed scoped confirmation phrase, when required.
    pub confirmation: Option<String>,
    /// Extra hidden fields (payload values), rendered as escaped pairs.
    pub extra: Vec<(String, String)>,
    /// A form problem to show above the fields.
    pub problem: Option<String>,
}

/// Render one action form.
pub fn action_form(spec: &ActionFormSpec) -> String {
    let mut out = String::from("<form class=\"action-form\" method=\"post\" action=\"");
    out.push_str(&apex_ui::escape::attr(&spec.action));
    out.push_str("\">");
    if let Some(problem) = &spec.problem {
        out.push_str(&apex_ui::components::notice(
            StatusKind::Failure,
            "The action was not requested.",
            problem,
            None,
        ));
    }
    out.push_str(&apex_ui::components::csrf_hidden(&spec.csrf));
    out.push_str(&hidden("idempotency_key", &spec.idempotency_key));
    out.push_str(&hidden("version_token", &spec.version_token));
    out.push_str(&hidden("target", &spec.target));
    out.push_str(&hidden("return_path", &spec.return_path));
    for (name, value) in &spec.extra {
        out.push_str(&hidden(name, value));
    }
    let reason_label = if spec.reason_required {
        "Reason (required)"
    } else {
        "Reason"
    };
    out.push_str(&apex_ui::components::field(
        "reason",
        reason_label,
        Some("Shown in the action journal with the before/after digests."),
        None,
        &apex_ui::components::textarea("reason", "reason", None, 3, Some(2000)),
    ));
    if spec.evidence {
        out.push_str(&apex_ui::components::field(
            "evidence",
            "Recovery evidence (required)",
            Some("A confirming read or verified proof; a retry accepted by a queue is not a successful restore."),
            None,
            &apex_ui::components::textarea("evidence", "evidence", None, 3, Some(2000)),
        ));
    }
    if spec.source_document {
        out.push_str(&apex_ui::components::field(
            "source_document",
            "Source document (required)",
            Some("The reference the reconciliation was checked against."),
            None,
            &apex_ui::components::input(
                "source_document",
                "source_document",
                "text",
                None,
                false,
                None,
            ),
        ));
    }
    if let Some(phrase) = &spec.confirmation {
        out.push_str(&apex_ui::components::field(
            "confirmation",
            "Type the confirmation to continue",
            Some(&format!("Type exactly: {phrase}")),
            None,
            &apex_ui::components::input("confirmation", "confirmation", "text", None, true, None),
        ));
    }
    out.push_str(&apex_ui::components::action_button(
        &spec.label,
        "submit",
        "1",
        true,
    ));
    out.push_str("</form>");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta() -> PageMeta {
        PageMeta {
            environment: "PRODUCTION".to_string(),
            as_of_ms: crate::fixtures::DEMO_NOW_MS,
            operator: Some("Maya".to_string()),
            role: Some(Role::Operator),
            fixtures: true,
        }
    }

    #[test]
    fn document_frames_with_the_operator_shell() {
        let html = document(&meta(), "Overview", &["Overview"], "<h1>Overview</h1>");
        assert!(html.contains("<p class=\"eyebrow\">OPERATOR PLANE</p>"));
        assert!(html.contains("<title>Overview \u{2014} Apex console</title>"));
        assert!(!html.contains("<script"));
        assert_eq!(html.matches("<style").count(), 1);
    }

    #[test]
    fn meta_line_shows_environment_as_of_and_operator() {
        let line = meta_line(&meta());
        assert!(line.contains(">PRODUCTION</span>"));
        assert!(line.contains("As of 15 Oct 2026, 09:10 UTC"));
        assert!(line.contains("Maya"));
        assert!(line.contains("(Operator)"));
    }

    #[test]
    fn fixture_banner_only_renders_for_the_dataset() {
        assert!(fixtures_banner(&meta()).contains("ILLUSTRATIVE DATA"));
        let mut live = meta();
        live.fixtures = false;
        assert_eq!(fixtures_banner(&live), "");
    }
}
