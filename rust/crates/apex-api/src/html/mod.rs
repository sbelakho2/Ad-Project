//! The server-rendered publisher console: plain HTML, zero JavaScript.
//!
//! Every page is assembled from the frozen design system in `apex-ui`
//! ([`apex_ui::shell`] for the document, [`apex_ui::components`] for panels,
//! tables, forms and notices, [`apex_ui::format`] for money and UTC dates,
//! [`apex_ui::status`] for chips) and rendered through
//! [`apex_ui::escape::html_escape`] for every dynamic string, element text and
//! quoted attribute alike.
//!
//! Layout contract of every console page:
//!
//! - one `<style>` block from [`apex_ui::tokens::console_css`], no `<script>`
//!   element and no external fetch;
//! - exactly one `<h1>` owned by the page body (the shell emits none);
//! - breadcrumb `Workspace / <area>` with Help pointing at `/support/`;
//! - the read-only chip and footer line whenever the deployment serves
//!   [`crate::cycle::READ_ONLY_MODE`];
//! - the publisher label from `APEX_CONSOLE_PUBLISHER_NAME` (default
//!   `Publisher workspace`) and money in the currency from
//!   `APEX_CONSOLE_CURRENCY` (default `USD`), never an unlabelled amount.
//!
//! The marketing-shaped free-audit pages use the same shell with
//! [`apex_ui::shell::Area::Marketing`].

pub mod analytics;
pub mod audit;
pub mod billing;
pub mod connections;
pub mod errors;
pub mod experiments;
pub mod leads_html;
pub mod ledger;
pub mod overview;
pub mod proposals;
pub mod settings;
pub mod watch_html;

use apex_ui::shell::{self, Area, PageCtx, Theme};

pub use apex_ui::escape::html_escape;

/// Environment variable naming the workspace label shown in the topbar.
pub const PUBLISHER_NAME_ENV: &str = "APEX_CONSOLE_PUBLISHER_NAME";
/// Environment variable naming the display currency for micros amounts.
pub const CURRENCY_ENV: &str = "APEX_CONSOLE_CURRENCY";
/// Default publisher label when the environment does not set one.
pub const DEFAULT_PUBLISHER_LABEL: &str = "Publisher workspace";
/// Default display currency when the environment does not set one.
pub const DEFAULT_CURRENCY: &str = "USD";
/// The Help destination of every console page.
pub const HELP_HREF: &str = "/support/";

/// Per-deployment rendering context shared by every page: the publisher
/// label, the display currency and the deployment's read-only mode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsoleCtx {
    /// The label shown strong in the topbar.
    pub publisher_label: String,
    /// The currency every micros amount is rendered in.
    pub currency: String,
    /// Whether the shell shows the read-only chip and footer line.
    pub read_only: bool,
    /// The resolved theme of this request (the `apex_theme` cookie).
    pub theme: Theme,
}

impl ConsoleCtx {
    /// A context with explicit values (tests and the fixture renderer).
    pub fn new(
        publisher_label: impl Into<String>,
        currency: impl Into<String>,
        read_only: bool,
    ) -> Self {
        Self {
            publisher_label: publisher_label.into(),
            currency: currency.into(),
            read_only,
            theme: Theme::Auto,
        }
    }

    /// The same context with a resolved theme.
    #[must_use]
    pub fn with_theme(mut self, theme: Theme) -> Self {
        self.theme = theme;
        self
    }

    /// Read the deployment context from the environment.
    pub fn from_env() -> Self {
        Self {
            publisher_label: crate::config::env_value(PUBLISHER_NAME_ENV)
                .unwrap_or_else(|| DEFAULT_PUBLISHER_LABEL.to_string()),
            currency: crate::config::env_value(CURRENCY_ENV)
                .unwrap_or_else(|| DEFAULT_CURRENCY.to_string()),
            read_only: deployment_is_read_only(),
            theme: Theme::Auto,
        }
    }
}

impl Default for ConsoleCtx {
    fn default() -> Self {
        Self::new(
            DEFAULT_PUBLISHER_LABEL,
            DEFAULT_CURRENCY,
            deployment_is_read_only(),
        )
    }
}

/// Whether this deployment applies platform changes. The binary only ships
/// [`crate::cycle::READ_ONLY_MODE`]; the check keeps the shell wired to the
/// cycle module instead of a copied literal.
pub fn deployment_is_read_only() -> bool {
    crate::cycle::READ_ONLY_MODE == "read_only"
}

/// Render a full publisher-console document around `body` with the
/// `Workspace / <area>` breadcrumb.
pub fn console_document(ctx: &ConsoleCtx, title: &str, area: &str, body: &str) -> String {
    shell::page(
        &PageCtx {
            title: title.to_string(),
            area: Area::PublisherConsole,
            publisher_label: Some(ctx.publisher_label.clone()),
            breadcrumb: vec!["Workspace".to_string(), area.to_string()],
            read_only: ctx.read_only,
            theme: ctx.theme,
            help_href: Some(HELP_HREF.to_string()),
            marketing_header: false,
        },
        body,
    )
}

/// Render a full marketing document (the free-audit surfaces) with the light
/// header and the documented marketing footer.
pub fn marketing_document(_ctx: &ConsoleCtx, title: &str, body: &str) -> String {
    shell::page(
        &PageCtx {
            title: title.to_string(),
            area: Area::Marketing,
            publisher_label: None,
            breadcrumb: Vec::new(),
            read_only: false,
            theme: Theme::Auto,
            help_href: Some(HELP_HREF.to_string()),
            marketing_header: true,
        },
        body,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn console_document_frames_with_breadcrumb_help_and_read_only() {
        let ctx = ConsoleCtx::new("Northstar Studio", "USD", true);
        let html = console_document(&ctx, "Audit", "Audit", "<h1>Audit</h1>");
        assert!(html.contains("<strong>Northstar Studio</strong> / Workspace / Audit"));
        assert!(html.contains("<a class=\"topbar-help\" href=\"/support/\">Help</a>"));
        assert!(html.contains("<span class=\"status status-read-only\">Read-only</span>"));
        assert!(html.contains("Read-only mode. No platform settings are changed here."));
        assert!(html.contains("<title>Audit \u{2014} Apex console</title>"));
        assert!(!html.contains("<script"));
    }

    #[test]
    fn writable_context_drops_the_chip() {
        let ctx = ConsoleCtx::new("Pub", "EUR", false);
        let html = console_document(&ctx, "Overview", "Overview", "");
        assert!(!html.contains("<span class=\"status status-read-only\">"));
        assert!(!html.contains("Read-only mode. No platform settings are changed here."));
    }

    #[test]
    fn marketing_document_uses_the_light_shell() {
        let html = marketing_document(&ConsoleCtx::default(), "Free audit", "<h1>x</h1>");
        assert!(html.contains("<header class=\"marketing-header\">"));
        assert!(!html.contains("class=\"rail\""));
    }

    #[test]
    fn ctx_default_is_read_only_and_from_env_is_documented() {
        let fallback = ConsoleCtx::default();
        assert_eq!(fallback.publisher_label, "Publisher workspace");
        assert_eq!(fallback.currency, "USD");
        assert!(fallback.read_only, "this deployment is read-only");
        let from_env = ConsoleCtx::from_env();
        assert!(!from_env.publisher_label.is_empty());
        assert!(!from_env.currency.is_empty());
    }
}
