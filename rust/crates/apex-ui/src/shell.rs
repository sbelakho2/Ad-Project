//! The full document shell: doctype, head, skip link, dark rail, topbar,
//! main region and footer for the publisher console, the operator control
//! plane, the sales workspace and the marketing pages.
//!
//! The shell emits no `<h1>` (callers own exactly one) and no `<script>`
//! (the console is server-rendered with zero JavaScript). App areas render
//! the 216px dark navigation rail with the Apex lockup, the mono eyebrow,
//! the area navigation with `aria-current="page"` matching the breadcrumb,
//! the Read-only chip when the deployment is read-only, and a neutral
//! footer. Marketing pages render the light header and the documented
//! marketing footer instead.

use crate::escape::{attr, html_escape};
use crate::logo;
use crate::theme;
use crate::tokens;

pub use crate::theme::Theme;

/// The area a page belongs to; it selects the shell chrome, title suffix
/// and footer copy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Area {
    /// The publisher console (`/`, `/console/*`).
    PublisherConsole,
    /// The operator control plane (`/control/*`).
    OperatorPlane,
    /// The sales workspace (`/leads`, `/plan`, `/sends`, ...).
    SalesPlane,
    /// The public marketing site.
    Marketing,
}

/// Everything the shell needs to frame one page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageCtx {
    /// The page title without the area suffix.
    pub title: String,
    /// The surface this page belongs to.
    pub area: Area,
    /// The publisher or workspace label shown strong in the topbar.
    pub publisher_label: Option<String>,
    /// The breadcrumb parts (`Workspace`, `Audit`); the topbar joins them
    /// with slashes and the rail matches them against its navigation.
    pub breadcrumb: Vec<String>,
    /// Whether the deployment is in read-only mode.
    pub read_only: bool,
    /// The resolved theme preference.
    pub theme: Theme,
    /// The Help destination; defaults to `/support/` when absent.
    pub help_href: Option<String>,
    /// Marketing pages set this to render the light marketing header; app
    /// areas ignore it (their rail and topbar always render).
    pub marketing_header: bool,
}

/// Publisher console navigation. It lists exactly the routes the console
/// serves, so the rail never points at an address without a handler.
const PUBLISHER_NAV: [(&str, &str); 9] = [
    ("Overview", "/"),
    ("Audit", "/console/audit"),
    ("Analytics", "/console/analytics"),
    ("Proposals", "/console/proposals"),
    ("Experiments", "/console/experiments"),
    ("Change log", "/console/ledger"),
    ("Billing", "/console/billing"),
    ("Connections", "/console/connections"),
    ("Settings", "/console/settings"),
];

const OPERATOR_NAV: [(&str, &str); 10] = [
    ("Overview", "/control/"),
    ("Customers", "/control/customers"),
    ("Ingestion", "/control/ingestion"),
    ("Audits", "/control/audits"),
    ("Tests", "/control/experiments"),
    ("Incidents", "/control/incidents"),
    ("Finance", "/control/finance"),
    ("Growth", "/control/growth"),
    ("System", "/control/system"),
    ("Access", "/control/access"),
];

/// Sales workspace navigation, in the frozen order: the sales UI's own
/// routes only, never the publisher console or operator control plane.
const SALES_NAV: [(&str, &str); 7] = [
    ("Overview", "/"),
    ("Leads", "/leads"),
    ("Plan", "/plan"),
    ("Sends", "/sends"),
    ("Replies", "/replies"),
    ("Domains", "/domains"),
    ("Models", "/models"),
];

/// The public marketing site used when `APEX_SITE_URL` is unset.
pub const DEFAULT_SITE_URL: &str = "https://apexmediation.ee";
/// The default Help destination when a surface configures none.
pub const DEFAULT_SUPPORT_URL: &str = "https://apexmediation.ee/support/";

const MARKETING_NAV: [(&str, &str); 4] = [
    ("How it works", "/how-it-works/"),
    ("Pricing", "/pricing/"),
    ("Proof", "/proof/"),
    ("Security", "/security/"),
];

/// Render the full document around `body_html` (a fragment from
/// [`crate::components`] or caller markup that already follows the design
/// system).
pub fn page(ctx: &PageCtx, body_html: &str) -> String {
    page_inner(ctx, body_html, None)
}

/// The page with the stylesheet linked instead of embedded: the served
/// stylesheet at `css_href` must be exactly [`tokens::console_css`] bytes.
/// Surfaces that share one global stylesheet across pages (one request, one
/// cache entry) use this; every other property matches [`page`].
pub fn page_with_css(ctx: &PageCtx, body_html: &str, css_href: &str) -> String {
    page_inner(ctx, body_html, Some(css_href))
}

fn page_inner(ctx: &PageCtx, body_html: &str, css_href: Option<&str>) -> String {
    let marketing = ctx.area == Area::Marketing;
    let suffix = if marketing { "Apex" } else { "Apex console" };
    let title = format!("{} \u{2014} {suffix}", ctx.title);

    let mut out = String::with_capacity(16_384);
    out.push_str("<!DOCTYPE html>\n");
    let styles = match css_href {
        Some(href) => format!("<link rel=\"stylesheet\" href=\"{}\">", html_escape(href)),
        None => format!("<style>{}</style>", tokens::console_css()),
    };
    out.push_str(&format!(
        "<html lang=\"en\" {}>\n<head>\n<meta charset=\"utf-8\">\n\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
         <meta name=\"color-scheme\" content=\"light dark\">\n\
         <title>{}</title>\n{}\n</head>\n<body{}>\n",
        theme::html_attrs(ctx.theme),
        html_escape(&title),
        styles,
        if marketing {
            " class=\"marketing-body\""
        } else {
            ""
        }
    ));
    out.push_str("<a class=\"skip-link\" href=\"#main\">Skip to content</a>\n");
    if marketing {
        if ctx.marketing_header {
            out.push_str(&marketing_header());
        }
        // The block wrapper keeps page fragments in normal flow: `.main` is
        // a flex column, and without it every direct-child inline action
        // (an `action` link) would be blockified and stretched full width.
        out.push_str(&format!(
            "<main id=\"main\" class=\"main\">\n<div class=\"page-body\">\n{body_html}\n</div>\n</main>\n"
        ));
        out.push_str(&marketing_footer());
    } else {
        out.push_str(&console_shell(ctx, body_html));
    }
    out.push_str("</body>\n</html>\n");
    out
}

fn console_shell(ctx: &PageCtx, body_html: &str) -> String {
    let (nav, eyebrow, rail_label) = match ctx.area {
        Area::OperatorPlane => (
            &OPERATOR_NAV[..],
            "OPERATOR PLANE",
            "Operator control plane",
        ),
        Area::SalesPlane => (&SALES_NAV[..], "SALES WORKSPACE", "Sales workspace"),
        _ => (
            &PUBLISHER_NAV[..],
            "PUBLISHER WORKSPACE",
            "Publisher console",
        ),
    };
    let mut out = String::from("<div class=\"console\">\n");
    out.push_str(&format!(
        "<nav class=\"rail\" aria-label=\"{rail_label}\">\n"
    ));
    out.push_str(&format!(
        "<p class=\"rail-brand\">{}</p>\n",
        logo::lockup("#ffffff", 20)
    ));
    out.push_str(&format!("<p class=\"eyebrow\">{eyebrow}</p>\n"));
    out.push_str("<ul class=\"rail-nav\">\n");
    for (label, href) in nav {
        let current = if nav_current(label, &ctx.breadcrumb) {
            " aria-current=\"page\""
        } else {
            ""
        };
        out.push_str(&format!(
            "<li><a class=\"rail-link\"{current} href=\"{}\">{}</a></li>\n",
            attr(href),
            html_escape(label)
        ));
    }
    out.push_str("</ul>\n");
    if ctx.read_only {
        out.push_str(
            "<p class=\"rail-foot\"><span class=\"status status-read-only\">Read-only</span></p>\n",
        );
    }
    out.push_str("</nav>\n<div class=\"console-body\">\n");
    out.push_str(&topbar(ctx));
    // The block wrapper keeps page fragments in normal flow: `.main` is a
    // flex column, and without it every direct-child inline action (an
    // `action` link) would be blockified and stretched full width.
    out.push_str(&format!(
        "<main id=\"main\" class=\"main\">\n<div class=\"page-body\">\n{body_html}\n</div>\n</main>\n"
    ));
    out.push_str(&app_footer(ctx));
    out.push_str("</div>\n</div>\n");
    out
}

/// The rail highlights the navigation item whose label appears in the
/// breadcrumb, so the current area is marked with `aria-current="page"`.
fn nav_current(label: &str, breadcrumb: &[String]) -> bool {
    breadcrumb
        .iter()
        .any(|part| part.trim().eq_ignore_ascii_case(label))
}

fn topbar(ctx: &PageCtx) -> String {
    let mut scope = String::new();
    if let Some(label) = &ctx.publisher_label {
        if !label.trim().is_empty() {
            scope.push_str(&format!("<strong>{}</strong>", html_escape(label)));
        }
    }
    let crumbs: Vec<String> = ctx.breadcrumb.iter().map(|p| html_escape(p)).collect();
    let crumb_text = crumbs.join(" / ");
    let scope_html = match (scope.is_empty(), crumb_text.is_empty()) {
        (false, false) => format!("{scope} / {crumb_text}"),
        (false, true) => scope,
        (true, false) => crumb_text,
        (true, true) => String::new(),
    };
    let help = ctx
        .help_href
        .clone()
        .unwrap_or_else(|| DEFAULT_SUPPORT_URL.to_string());
    format!(
        "<header class=\"topbar\"><p class=\"topbar-scope\">{scope_html}</p>\
         <a class=\"topbar-help\" href=\"{}\">Help</a></header>\n",
        attr(&help)
    )
}

fn app_footer(ctx: &PageCtx) -> String {
    let line = if ctx.read_only {
        "Read-only mode. No platform settings are changed here."
    } else {
        match ctx.area {
            Area::OperatorPlane => "Apex operator control plane.",
            Area::SalesPlane => "Apex sales workspace.",
            _ => "Apex publisher console.",
        }
    };
    format!("<footer class=\"footer\"><p>{line}</p></footer>\n")
}

/// The configured marketing origin: `APEX_SITE_URL` when it is an absolute
/// http(s) URL, else the public default. Marketing-shell pages are served by
/// apex-api on the console and API origins, so their site links must be
/// absolute; relative paths there are 404s.
fn marketing_base() -> String {
    std::env::var("APEX_SITE_URL")
        .ok()
        .map(|value| value.trim().trim_end_matches('/').to_string())
        .filter(|value| value.starts_with("https://") || value.starts_with("http://"))
        .unwrap_or_else(|| DEFAULT_SITE_URL.to_string())
}

fn marketing_href(base: &str, path: &str) -> String {
    format!("{base}{path}")
}

/// An absolute public-site link for pages rendered by apex-api on the
/// console/API origins, where relative site paths are dead. Honors
/// `APEX_SITE_URL` and falls back to the public site.
pub fn site_href(path: &str) -> String {
    let base = marketing_base();
    marketing_href(&base, path)
}

/// A link that lives on the console origin (the watch form), not the static
/// marketing host. Falls back to the API-relative path when the console origin
/// is not configured.
pub fn console_href(path: &str) -> String {
    console_href_from(std::env::var("APEX_CONSOLE_URL").ok().as_deref(), path)
}

fn console_href_from(value: Option<&str>, path: &str) -> String {
    match value {
        Some(value) if value.starts_with("https://") || value.starts_with("http://") => {
            marketing_href(value.trim().trim_end_matches('/'), path)
        }
        _ => path.to_string(),
    }
}

fn marketing_header() -> String {
    let base = marketing_base();
    let mut out = String::from("<header class=\"marketing-header\">");
    out.push_str(&format!(
        "<a class=\"marketing-brand\" href=\"{}\">{}</a>",
        attr(&format!("{base}/")),
        logo::lockup(tokens::INK, 20)
    ));
    out.push_str("<nav class=\"marketing-nav\" aria-label=\"Site\">");
    for (label, href) in MARKETING_NAV {
        out.push_str(&format!(
            "<a href=\"{}\">{}</a>",
            attr(&marketing_href(&base, href)),
            html_escape(label)
        ));
    }
    out.push_str("</nav>");
    out.push_str(&format!(
        "<a class=\"marketing-cta action action-primary\" href=\"{}\">Get a free audit</a>",
        attr(&marketing_href(&base, "/free-audit/"))
    ));
    out.push_str("</header>\n");
    out
}

fn marketing_footer() -> String {
    let base = marketing_base();
    let mut out = String::from("<footer class=\"marketing-footer\">");
    out.push_str("<div class=\"marketing-footer-inner\">");
    out.push_str("<div class=\"marketing-footer-brand\">");
    out.push_str(&format!(
        "<p class=\"marketing-brand\">{}</p>",
        logo::lockup(tokens::PAPER, 20)
    ));
    out.push_str(
        "<p class=\"marketing-line\">Revenue operations for the mediation you already run.</p>",
    );
    out.push_str("</div>");
    out.push_str("<nav class=\"marketing-footer-nav\" aria-label=\"Product\">");
    let watch_href = console_href("/watch/");
    for (label, href) in [
        ("Setup Watch", watch_href.as_str()),
        ("How it works", "/how-it-works/"),
        ("Pricing", "/pricing/"),
        ("Proof", "/proof/"),
        ("Security", "/security/"),
    ] {
        out.push_str(&format!(
            "<a href=\"{}\">{}</a>",
            attr(&marketing_href(&base, href)),
            html_escape(label)
        ));
    }
    out.push_str("</nav>");
    out.push_str("<nav class=\"marketing-footer-nav\" aria-label=\"Resources\">");
    for (label, href) in [
        ("For publishers", "/for-publishers/"),
        ("Guides", "/guides/"),
        ("Support", "/support/"),
    ] {
        out.push_str(&format!(
            "<a href=\"{}\">{}</a>",
            attr(&marketing_href(&base, href)),
            html_escape(label)
        ));
    }
    out.push_str("</nav>");
    out.push_str("<nav class=\"marketing-footer-nav\" aria-label=\"Legal\">");
    for (label, href) in [("Privacy", "/privacy/"), ("Terms", "/terms/")] {
        out.push_str(&format!(
            "<a href=\"{}\">{}</a>",
            attr(&marketing_href(&base, href)),
            html_escape(label)
        ));
    }
    out.push_str("</nav>");
    out.push_str("<div class=\"marketing-footer-meta\">");
    out.push_str(
        "<p class=\"marketing-contact\"><a href=\"mailto:hello@apexmediation.ee\">hello@apexmediation.ee</a></p>",
    );
    out.push_str(
        "<p class=\"marketing-line\">Apex works through platform APIs. Your apps keep their existing ad-serving path.</p>",
    );
    out.push_str("</div>");
    out.push_str("</div>");
    out.push_str("</footer>\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn console_links_move_only_for_an_http_origin() {
        assert_eq!(
            console_href_from(Some("https://console.example/"), "/watch/"),
            "https://console.example/watch/"
        );
        assert_eq!(
            console_href_from(Some("http://127.0.0.1:8787"), "/watch/"),
            "http://127.0.0.1:8787/watch/"
        );
        assert_eq!(
            console_href_from(Some("ftp://console.example"), "/watch/"),
            "/watch/"
        );
        assert_eq!(console_href_from(None, "/watch/"), "/watch/");
    }

    fn publisher_ctx() -> PageCtx {
        PageCtx {
            title: "Audit".to_string(),
            area: Area::PublisherConsole,
            publisher_label: Some("Northstar Studio".to_string()),
            breadcrumb: vec!["Workspace".to_string(), "Audit".to_string()],
            read_only: true,
            theme: Theme::Dark,
            help_href: None,
            marketing_header: false,
        }
    }

    #[test]
    fn publisher_shell_is_a_complete_document_with_one_stylesheet() {
        let html = page(&publisher_ctx(), "<h1>Audit</h1>");
        assert!(html.starts_with("<!DOCTYPE html>\n"));
        assert!(html.contains("<html lang=\"en\" data-theme=\"dark\">"));
        assert!(html.contains("<meta charset=\"utf-8\">"));
        assert!(html.contains("name=\"viewport\""));
        assert!(html.contains("<title>Audit \u{2014} Apex console</title>"));
        assert_eq!(html.matches("<style").count(), 1);
        assert_eq!(html.matches("</style>").count(), 1);
        assert!(html.contains(tokens::console_css()));
        assert!(html.contains("<a class=\"skip-link\" href=\"#main\">Skip to content</a>"));
        assert!(html.contains("<main id=\"main\" class=\"main\">"));
        assert!(html.ends_with("</body>\n</html>\n"));
    }

    #[test]
    fn page_with_css_links_the_global_stylesheet_and_embeds_none() {
        let html = page_with_css(&sales_ctx(), "<h1>Leads</h1>", "/assets/apex.css");
        assert_eq!(html.matches("<style").count(), 0, "no embedded stylesheet");
        assert_eq!(
            html.matches("<link rel=\"stylesheet\"").count(),
            1,
            "exactly one stylesheet link"
        );
        assert!(html.contains("href=\"/assets/apex.css\""));
        // Everything else matches the embedded-styles page.
        let embedded = page(&sales_ctx(), "<h1>Leads</h1>");
        let linked_body = html
            .split("</head>")
            .nth(1)
            .expect("head close")
            .to_string();
        let embedded_body = embedded
            .split("</head>")
            .nth(1)
            .expect("head close")
            .to_string();
        assert_eq!(linked_body, embedded_body);
    }

    #[test]
    fn publisher_shell_has_no_h1_and_no_script() {
        let html = page(&publisher_ctx(), "<p>body</p>");
        assert!(!html.contains("<h1"));
        assert!(!html.contains("<script"));
        assert!(!html.contains("javascript:"));
    }

    #[test]
    fn rail_marks_the_breadcrumb_area_current_and_shows_read_only() {
        let html = page(&publisher_ctx(), "");
        assert!(html.contains("<p class=\"eyebrow\">PUBLISHER WORKSPACE</p>"));
        assert!(html.contains(
            "<a class=\"rail-link\" aria-current=\"page\" href=\"/console/audit\">Audit</a>"
        ));
        assert!(html.contains("<a class=\"rail-link\" href=\"/\">Overview</a>"));
        assert!(
            !html.contains("<a class=\"rail-link\" aria-current=\"page\" href=\"/\">Overview</a>")
        );
        assert!(html.contains("<span class=\"status status-read-only\">Read-only</span>"));
        assert!(html.contains(
            "<footer class=\"footer\"><p>Read-only mode. No platform settings are changed here.</p></footer>"
        ));
    }

    #[test]
    fn publisher_nav_lists_only_served_routes() {
        let html = page(&publisher_ctx(), "");
        assert_eq!(PUBLISHER_NAV.len(), 9);
        for served in [
            "/",
            "/console/audit",
            "/console/analytics",
            "/console/proposals",
            "/console/experiments",
            "/console/ledger",
            "/console/billing",
            "/console/connections",
            "/console/settings",
        ] {
            assert!(
                html.contains(served),
                "rail is missing the served route {served}"
            );
        }
        assert!(!html.contains("/control/"), "rail links an operator route");
    }

    fn sales_ctx() -> PageCtx {
        PageCtx {
            title: "Leads".to_string(),
            area: Area::SalesPlane,
            publisher_label: None,
            breadcrumb: vec!["Sales".to_string(), "Leads".to_string()],
            read_only: true,
            theme: Theme::Auto,
            help_href: None,
            marketing_header: false,
        }
    }

    #[test]
    fn sales_shell_uses_its_own_eyebrow_and_navigation() {
        let html = page(&sales_ctx(), "<h1>Leads</h1>");
        assert!(html.contains("<title>Leads \u{2014} Apex console</title>"));
        assert!(html.contains("<p class=\"eyebrow\">SALES WORKSPACE</p>"));
        // The current-state marking and the read-only chip match the other
        // app areas exactly.
        assert!(
            html.contains("<a class=\"rail-link\" aria-current=\"page\" href=\"/leads\">Leads</a>")
        );
        assert!(html.contains("<span class=\"status status-read-only\">Read-only</span>"));
        assert!(html.contains(
            "<footer class=\"footer\"><p>Read-only mode. No platform settings are changed here.</p></footer>"
        ));
        assert!(html.contains("<nav class=\"rail\" aria-label=\"Sales workspace\">"));
        let mut writable = sales_ctx();
        writable.read_only = false;
        let html = page(&writable, "");
        assert!(!html.contains("class=\"status status-read-only\""));
        assert!(!html.contains("class=\"rail-foot\""));
        assert!(html.contains("<footer class=\"footer\"><p>Apex sales workspace.</p></footer>"));
    }

    #[test]
    fn sales_nav_lists_only_served_routes() {
        let html = page(&sales_ctx(), "");
        assert_eq!(SALES_NAV.len(), 7);
        // The seven sales routes appear in the exact frozen order.
        let mut cursor = 0usize;
        for (label, href) in SALES_NAV {
            let needle = format!("href=\"{href}\"");
            let position = html[cursor..]
                .find(&needle)
                .unwrap_or_else(|| panic!("sales rail is missing the served route {href}"));
            cursor += position + needle.len();
            assert!(
                html.contains(&format!(">{label}</a>")),
                "sales rail is missing the {label} label"
            );
        }
        assert!(
            !html.contains("/console"),
            "sales rail links a console route"
        );
        assert!(
            !html.contains("/control"),
            "sales rail links an operator route"
        );
    }

    #[test]
    fn writable_shell_drops_the_chip_and_uses_the_neutral_footer() {
        let mut ctx = publisher_ctx();
        ctx.read_only = false;
        let html = page(&ctx, "");
        assert!(!html.contains("class=\"status status-read-only\""));
        assert!(!html.contains("class=\"rail-foot\""));
        assert!(html.contains("<footer class=\"footer\"><p>Apex publisher console.</p></footer>"));
    }

    #[test]
    fn topbar_shows_publisher_breadcrumb_and_help() {
        let html = page(&publisher_ctx(), "");
        assert!(html.contains(
            "<p class=\"topbar-scope\"><strong>Northstar Studio</strong> / Workspace / Audit</p>"
        ));
        assert!(html.contains(
            "<a class=\"topbar-help\" href=\"https://apexmediation.ee/support/\">Help</a>"
        ));
        let mut with_help = publisher_ctx();
        with_help.help_href = Some("/guides/".to_string());
        assert!(page(&with_help, "").contains("href=\"/guides/\">Help</a>"));
    }

    #[test]
    fn operator_shell_uses_its_own_eyebrow_and_navigation() {
        let ctx = PageCtx {
            title: "Customers".to_string(),
            area: Area::OperatorPlane,
            publisher_label: None,
            breadcrumb: vec!["Operator".to_string(), "Customers".to_string()],
            read_only: false,
            theme: Theme::Auto,
            help_href: None,
            marketing_header: false,
        };
        let html = page(&ctx, "");
        assert!(html.contains("<title>Customers \u{2014} Apex console</title>"));
        assert!(html.contains("<p class=\"eyebrow\">OPERATOR PLANE</p>"));
        assert!(html.contains("<a class=\"rail-link\" aria-current=\"page\" href=\"/control/customers\">Customers</a>"));
        assert!(html.contains("href=\"/control/ingestion\">Ingestion</a>"));
        assert!(html.contains("href=\"/control/system\">System</a>"));
        assert!(
            html.contains("<footer class=\"footer\"><p>Apex operator control plane.</p></footer>")
        );
    }

    #[test]
    fn marketing_shell_is_light_with_documented_header_and_footer() {
        let ctx = PageCtx {
            title: "Pricing".to_string(),
            area: Area::Marketing,
            publisher_label: None,
            breadcrumb: Vec::new(),
            read_only: false,
            theme: Theme::Light,
            help_href: None,
            marketing_header: true,
        };
        let html = page(&ctx, "<h1>Pricing</h1>");
        assert!(html.contains("<title>Pricing \u{2014} Apex</title>"));
        assert!(!html.contains("class=\"rail\""));
        assert!(!html.contains("class=\"topbar\""));
        assert!(html.contains("<header class=\"marketing-header\">"));
        assert!(html.contains("href=\"https://apexmediation.ee/how-it-works/\">How it works</a>"));
        assert!(html.contains("href=\"https://apexmediation.ee/pricing/\">Pricing</a>"));
        assert!(html.contains("href=\"https://apexmediation.ee/proof/\">Proof</a>"));
        assert!(html.contains("href=\"https://apexmediation.ee/security/\">Security</a>"));
        assert!(html.contains("href=\"https://apexmediation.ee/free-audit/\">Get a free audit</a>"));
        assert!(html.contains("Revenue operations for the mediation you already run."));
        assert!(html.contains("For publishers</a>"));
        assert!(html.contains("mailto:hello@apexmediation.ee"));
        assert!(html.contains(
            "Apex works through platform APIs. Your apps keep their existing ad-serving path."
        ));
        assert!(html.contains("<a class=\"skip-link\" href=\"#main\">Skip to content</a>"));
        assert_eq!(html.matches("<style").count(), 1);
        assert!(!html.contains("<script"));
    }

    #[test]
    fn marketing_shell_without_header_flag_omits_the_light_header() {
        let ctx = PageCtx {
            title: "Privacy".to_string(),
            area: Area::Marketing,
            publisher_label: None,
            breadcrumb: Vec::new(),
            read_only: false,
            theme: Theme::Auto,
            help_href: None,
            marketing_header: false,
        };
        let html = page(&ctx, "");
        assert!(!html.contains("<header class=\"marketing-header\""));
        assert!(html.contains("<footer class=\"marketing-footer\">"));
    }

    #[test]
    fn shell_escapes_hostile_context_values() {
        let hostile = "<script>alert(\"x\")</script>&'".to_string();
        let ctx = PageCtx {
            title: hostile.clone(),
            area: Area::PublisherConsole,
            publisher_label: Some(hostile.clone()),
            breadcrumb: vec![hostile.clone(), hostile.clone()],
            read_only: true,
            theme: Theme::Auto,
            help_href: Some(hostile.clone()),
            marketing_header: false,
        };
        let html = page(&ctx, "");
        assert!(!html.contains("<script>alert"));
        assert!(!html.contains("</title><script"));
        assert!(html.contains("&lt;script&gt;"));
        assert!(html.contains("<strong>&lt;script&gt;"));
        assert!(html.contains("href=\"&lt;script&gt;"));
    }

    #[test]
    fn theme_attribute_matches_the_context() {
        let mut ctx = publisher_ctx();
        ctx.theme = Theme::Auto;
        assert!(page(&ctx, "").contains("<html lang=\"en\" data-theme=\"auto\">"));
        ctx.theme = Theme::Light;
        assert!(page(&ctx, "").contains("<html lang=\"en\" data-theme=\"light\">"));
    }
}
