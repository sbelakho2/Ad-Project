//! The document wrapper: apex-ui's global-stylesheet shell.
//!
//! The sales workspace serves the design system's stylesheet once at
//! `/assets/apex.css` (see [`crate::assets`]) and every page links it via
//! [`apex_ui::shell::page_with_css`], so responses contain exactly one
//! stylesheet link, no `<style>` block and no `style="` attribute anywhere.

use apex_ui::shell::{page_with_css, PageCtx};

/// The one stylesheet link every sales page carries.
pub const STYLESHEET_HREF: &str = "/assets/apex.css";

/// The link element every sales page carries in its head.
pub const STYLESHEET_LINK: &str = "<link rel=\"stylesheet\" href=\"/assets/apex.css\">";

/// Render the full document for `body_html` with the stylesheet link as the
/// only styles.
pub fn document(ctx: &PageCtx, body_html: &str) -> String {
    page_with_css(ctx, body_html, STYLESHEET_HREF)
}

#[cfg(test)]
mod tests {
    use super::*;
    use apex_ui::shell::{Area, Theme};

    fn ctx() -> PageCtx {
        PageCtx {
            title: "Overview".to_string(),
            area: Area::SalesPlane,
            publisher_label: None,
            breadcrumb: vec!["Sales".to_string(), "Overview".to_string()],
            read_only: false,
            theme: Theme::Auto,
            help_href: None,
            marketing_header: false,
        }
    }

    #[test]
    fn the_document_links_the_one_stylesheet_and_embeds_none() {
        let html = document(&ctx(), "<h1>Overview</h1>");
        assert!(html.starts_with("<!DOCTYPE html>\n"));
        assert_eq!(html.matches(STYLESHEET_LINK).count(), 1);
        assert!(!html.contains("<style"), "no style block may remain");
        assert!(!html.contains("style=\""), "no inline style attribute");
        assert!(!html.contains(apex_ui::tokens::console_css()));
        assert!(html.contains("<h1>Overview</h1>"));
        assert!(html.contains("SALES WORKSPACE"));
        assert!(html.ends_with("</body>\n</html>\n"));
    }

    #[test]
    fn the_rest_of_the_shell_is_untouched() {
        let html = document(&ctx(), "");
        assert!(html.contains("<nav class=\"rail\" aria-label=\"Sales workspace\">"));
        assert!(
            html.contains("<a class=\"rail-link\" aria-current=\"page\" href=\"/\">Overview</a>")
        );
        assert!(html.contains("<footer class=\"footer\"><p>Apex sales workspace.</p></footer>"));
        assert!(html.contains("<a class=\"skip-link\" href=\"#main\">Skip to content</a>"));
    }
}
