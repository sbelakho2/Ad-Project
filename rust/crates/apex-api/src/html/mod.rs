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
//! - one external stylesheet link (`/assets/apex.css`), no inline style and no `<script>`
//!   element and no external fetch;
//! - exactly one `<h1>` owned by the page body (the shell emits none);
//! - breadcrumb `Workspace / <area>` with Help pointing at the configured
//!   absolute `APEX_SUPPORT_URL` (the marketing pages keep the on-site
//!   relative `/support/`);
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

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use apex_ui::shell::{self, Area, PageCtx, Theme};

pub use apex_ui::escape::html_escape;

/// The deployment context, built once from the environment. Every console
/// handler calls [`ConsoleCtx::from_env`], so read-only requests reuse this
/// value instead of rereading environment variables and reparsing the support
/// URL per render.
static CONSOLE_CTX: Mutex<Option<Arc<ConsoleCtx>>> = Mutex::new(None);
/// How many times the environment was actually read (tests assert one read
/// for many renders).
static CONSOLE_ENV_READS: AtomicU64 = AtomicU64::new(0);

fn console_ctx_cache() -> MutexGuard<'static, Option<Arc<ConsoleCtx>>> {
    CONSOLE_CTX
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Environment variable naming the workspace label shown in the topbar.
pub const PUBLISHER_NAME_ENV: &str = "APEX_CONSOLE_PUBLISHER_NAME";
/// Environment variable naming the display currency for micros amounts.
pub const CURRENCY_ENV: &str = "APEX_CONSOLE_CURRENCY";
/// Default publisher label when the environment does not set one.
pub const DEFAULT_PUBLISHER_LABEL: &str = "Publisher workspace";
/// Default display currency when the environment does not set one.
pub const DEFAULT_CURRENCY: &str = "USD";
/// The on-site Help destination of the marketing-shell pages. Console pages
/// use the configured absolute [`crate::config::SUPPORT_URL_ENV`] instead.
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
    /// The absolute Help destination of the authenticated console pages.
    pub support_url: String,
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
            support_url: crate::config::DEFAULT_SUPPORT_URL.to_string(),
            theme: Theme::Auto,
        }
    }

    /// The same context with an explicit absolute Help destination.
    #[must_use]
    pub fn with_support_url(mut self, support_url: impl Into<String>) -> Self {
        self.support_url = support_url.into();
        self
    }

    /// The same context with a resolved theme.
    #[must_use]
    pub fn with_theme(mut self, theme: Theme) -> Self {
        self.theme = theme;
        self
    }

    /// Read the deployment context from the environment, once.
    ///
    /// The first call reads `APEX_CONSOLE_PUBLISHER_NAME`,
    /// `APEX_CONSOLE_CURRENCY` and `APEX_SUPPORT_URL` and caches the result;
    /// every later render clones the cached value. Tests that need a specific
    /// context install it with [`Self::install_cached_for_tests`] instead of
    /// mutating the process environment after the cache is warm.
    pub fn from_env() -> Self {
        if let Some(cached) = console_ctx_cache().as_ref() {
            return (**cached).clone();
        }
        let mut cache = console_ctx_cache();
        if let Some(cached) = cache.as_ref() {
            return (**cached).clone();
        }
        CONSOLE_ENV_READS.fetch_add(1, Ordering::Relaxed);
        let built = Arc::new(Self::read_env());
        *cache = Some(Arc::clone(&built));
        (*built).clone()
    }

    /// The uncached environment read behind [`Self::from_env`].
    fn read_env() -> Self {
        Self {
            publisher_label: crate::config::env_value(PUBLISHER_NAME_ENV)
                .unwrap_or_else(|| DEFAULT_PUBLISHER_LABEL.to_string()),
            currency: crate::config::env_value(CURRENCY_ENV)
                .unwrap_or_else(|| DEFAULT_CURRENCY.to_string()),
            read_only: deployment_is_read_only(),
            support_url: crate::config::support_url(
                crate::config::env_value(crate::config::SUPPORT_URL_ENV).as_deref(),
            )
            .unwrap_or_else(|_| crate::config::DEFAULT_SUPPORT_URL.to_string()),
            theme: Theme::Auto,
        }
    }

    /// Install (or clear) the cached context explicitly for tests.
    ///
    /// This is the supported override for env-mutating tests: it replaces the
    /// process-wide cache rather than requiring a new environment read.
    #[doc(hidden)]
    pub fn install_cached_for_tests(ctx: Option<Self>) {
        *console_ctx_cache() = ctx.map(Arc::new);
    }

    /// How many times the environment was read to build the cached context
    /// (tests and the performance harness).
    #[doc(hidden)]
    pub fn cached_env_reads() -> u64 {
        CONSOLE_ENV_READS.load(Ordering::Relaxed)
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

/// A checkbox whose submitted value is explicit.
///
/// The shared design-system checkbox renders no `value` attribute, so a
/// browser submits the default `on`. Every form in this crate that reads a
/// checkbox (the free-audit platform choice and the confirmation gates that
/// compare against `yes`) needs a real value; without one the rendered form
/// could never pass its own server validation.
pub fn checkbox_with_value(
    id: &str,
    name: &str,
    value: &str,
    label: &str,
    checked: bool,
) -> String {
    format!(
        "<label class=\"checkbox\"><input type=\"checkbox\" id=\"{}\" name=\"{}\" value=\"{}\"{}> {}</label>",
        html_escape(id),
        html_escape(name),
        html_escape(value),
        if checked { " checked" } else { "" },
        html_escape(label)
    )
}

/// Render a full publisher-console document around `body` with the
/// `Workspace / <area>` breadcrumb.
pub fn console_document(ctx: &ConsoleCtx, title: &str, area: &str, body: &str) -> String {
    shell::page_with_css(
        &PageCtx {
            title: title.to_string(),
            area: Area::PublisherConsole,
            publisher_label: Some(ctx.publisher_label.clone()),
            breadcrumb: vec!["Workspace".to_string(), area.to_string()],
            read_only: ctx.read_only,
            theme: ctx.theme,
            help_href: Some(ctx.support_url.clone()),
            marketing_header: false,
        },
        body,
        crate::fonts::CSS_PATH,
    )
}

/// Render a full marketing document (the free-audit surfaces) with the light
/// header and the documented marketing footer.
pub fn marketing_document(_ctx: &ConsoleCtx, title: &str, body: &str) -> String {
    shell::page_with_css(
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
        crate::fonts::CSS_PATH,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Serialise the two tests that exercise the process-wide context cache.
    static FROM_ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn console_document_frames_with_breadcrumb_help_and_read_only() {
        let ctx = ConsoleCtx::new("Northstar Studio", "USD", true);
        let html = console_document(&ctx, "Audit", "Audit", "<h1>Audit</h1>");
        assert!(html.contains("<strong>Northstar Studio</strong> / Workspace / Audit"));
        assert!(html.contains(
            "<a class=\"topbar-help\" href=\"https://apexmediation.ee/support/\">Help</a>"
        ));
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
    fn console_document_uses_the_configured_absolute_support_url() {
        let ctx = ConsoleCtx::new("Northstar Studio", "USD", true)
            .with_support_url("https://help.example/support/");
        let html = console_document(&ctx, "Audit", "Audit", "<h1>Audit</h1>");
        assert!(
            html.contains(
                "<a class=\"topbar-help\" href=\"https://help.example/support/\">Help</a>"
            ),
            "{html}"
        );
        assert!(!html.contains("href=\"/support/\""), "{html}");
    }

    #[test]
    fn marketing_document_uses_the_light_shell_and_the_on_site_help_link() {
        let html = marketing_document(&ConsoleCtx::default(), "Free audit", "<h1>x</h1>");
        assert!(html.contains("<header class=\"marketing-header\">"));
        assert!(!html.contains("class=\"rail\""));
        assert!(
            html.contains("<a href=\"https://apexmediation.ee/support/\">Support</a>"),
            "marketing pages link the public site absolutely: {html}"
        );
        assert!(
            !html.contains("href=\"/support/\""),
            "a relative site link is dead on the console and API origins: {html}"
        );
    }

    #[test]
    fn ctx_default_is_read_only_and_from_env_is_documented() {
        let _guard = FROM_ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let fallback = ConsoleCtx::default();
        assert_eq!(fallback.publisher_label, "Publisher workspace");
        assert_eq!(fallback.currency, "USD");
        assert_eq!(fallback.support_url, crate::config::DEFAULT_SUPPORT_URL);
        assert!(fallback.read_only, "this deployment is read-only");
        let from_env = ConsoleCtx::from_env();
        assert!(!from_env.publisher_label.is_empty());
        assert!(!from_env.currency.is_empty());
        assert!(!from_env.support_url.is_empty());
    }

    /// One hundred console renders read the environment and parse the support
    /// URL exactly once, and every render is byte-identical.
    #[test]
    fn many_renders_read_the_environment_once_and_render_identically() {
        let _guard = FROM_ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        ConsoleCtx::install_cached_for_tests(None);
        CONSOLE_ENV_READS.store(0, std::sync::atomic::Ordering::Relaxed);
        let mut renders = Vec::with_capacity(100);
        for _ in 0..100 {
            let ctx = ConsoleCtx::from_env();
            renders.push(console_document(
                &ctx,
                "Overview",
                "Overview",
                "<h1>Overview</h1>",
            ));
        }
        assert_eq!(
            ConsoleCtx::cached_env_reads(),
            1,
            "the environment is read once for every render"
        );
        assert!(
            renders.windows(2).all(|pair| pair[0] == pair[1]),
            "every render must be byte-identical"
        );

        // The explicit override replaces the cached value without an env read.
        let override_ctx = ConsoleCtx::new("Test workspace", "EUR", true);
        ConsoleCtx::install_cached_for_tests(Some(override_ctx.clone()));
        assert_eq!(ConsoleCtx::from_env(), override_ctx);
        assert_eq!(ConsoleCtx::cached_env_reads(), 1);
        ConsoleCtx::install_cached_for_tests(None);
    }
}
