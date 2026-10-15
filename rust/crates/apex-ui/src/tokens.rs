//! The frozen design tokens and the one stylesheet every Apex surface shares.
//!
//! Values are the exact lowercase hex constants of the design document. The
//! stylesheet is assembled from a base block, one dark block (applied both
//! through `:root[data-theme="dark"]` and, for `:root[data-theme="auto"]`,
//! through `prefers-color-scheme`), a responsive block and a reduced-motion
//! block. Tests enforce that every token hex appears, every hex is lowercase,
//! braces balance, every class the components emit is styled, and every
//! foreground/background pairing the stylesheet uses clears WCAG AA.

/// Ink: primary text and shell surfaces.
pub const INK: &str = "#182c31";
/// Paper: the page background.
pub const PAPER: &str = "#f5f6f1";
/// Surface: card and table backgrounds.
pub const SURFACE: &str = "#ffffff";
/// Cobalt: actions and the test series.
pub const COBALT: &str = "#2849d8";
/// Citron: editorial emphasis and attention tint.
pub const CITRON: &str = "#e2f2a6";
/// Secondary: supporting text.
pub const SECONDARY: &str = "#536568";
/// Success: verified completion.
pub const SUCCESS: &str = "#176b52";
/// Warning: attention needed.
pub const WARNING: &str = "#865409";
/// Danger: failure and harm.
pub const DANGER: &str = "#a33638";
/// Input boundary: the essential boundary of an input control.
pub const INPUT_BOUNDARY: &str = "#7e8c91";
/// Separator: row and rule color (never the sole boundary of a control).
pub const SEPARATOR: &str = "#d9dfdd";
/// Dark scheme page background.
pub const PAGE_DARK: &str = "#15262d";
/// Dark scheme panel background.
pub const PANEL_DARK: &str = "#22383f";
/// Dark scheme primary text.
pub const TEXT_DARK: &str = "#f3f7f2";
/// Dark scheme supporting text.
pub const SECONDARY_DARK: &str = "#bccac8";
/// Dark scheme action color.
pub const ACTION_DARK: &str = "#acbeff";
/// Dark scheme link color.
pub const LINK_DARK: &str = "#adc4ff";
/// The spacing scale in CSS pixels.
pub const SPACING_PX: [u32; 9] = [4, 8, 12, 16, 24, 32, 48, 64, 96];
/// Control corner radius in CSS pixels.
pub const RADIUS_CONTROL: u32 = 6;
/// Panel corner radius in CSS pixels.
pub const RADIUS_PANEL: u32 = 8;
/// Major preview (chart) corner radius in CSS pixels.
pub const RADIUS_PREVIEW: u32 = 12;
/// The interface font stack. Manrope is first when the self-hosted font is
/// available; the rest is the system fallback.
pub const FONT_SANS: &str =
    "Manrope, ui-sans-serif, system-ui, -apple-system, \"Segoe UI\", Arial, sans-serif";
/// The monospace stack for identifiers, exact values and technical exports.
pub const FONT_MONO: &str =
    "\"IBM Plex Mono\", ui-monospace, SFMono-Regular, Menlo, Consolas, monospace";
/// Publisher console navigation rail width in CSS pixels.
pub const CONSOLE_NAV_WIDTH: u32 = 216;
/// Console main region maximum width in CSS pixels.
pub const CONSOLE_MAIN_MAX: u32 = 1440;
/// Console page padding in CSS pixels.
pub const PAGE_PADDING: u32 = 32;
/// Vertical gap between console modules in CSS pixels.
pub const MODULE_GAP: u32 = 24;

/// Tint surfaces derived from the frozen hues (not tokens themselves): the
/// table header tint, the neutral chip tint, the success tint, the cobalt
/// tint and the danger tint. The contrast tests resolve them by name; the
/// stylesheet carries the same lowercase hex values as literals.
#[cfg(test)]
pub(crate) const INK_TINT: &str = "#e7eceb";
#[cfg(test)]
pub(crate) const NEUTRAL_TINT: &str = "#e9eceb";
#[cfg(test)]
pub(crate) const SUCCESS_TINT: &str = "#e2efea";
#[cfg(test)]
pub(crate) const COBALT_TINT: &str = "#e9edfc";
#[cfg(test)]
pub(crate) const DANGER_TINT: &str = "#f7e9e9";
/// Dark-scheme status text colors on dark chip surfaces.
#[cfg(test)]
pub(crate) const DANGER_DARK_TEXT: &str = "#f0a3a5";
#[cfg(test)]
pub(crate) const WARNING_DARK_TEXT: &str = "#f0c98a";
#[cfg(test)]
pub(crate) const SUCCESS_DARK_TEXT: &str = "#8fd6b8";

const BASE_CSS: &str = r#"*,*::before,*::after{box-sizing:border-box}
html{-webkit-text-size-adjust:100%}
body{margin:0;font-family:Manrope, ui-sans-serif, system-ui, -apple-system, "Segoe UI", Arial, sans-serif;font-size:14px;line-height:22px;color:#182c31;background:#f5f6f1}
a{color:#2849d8}
:focus-visible{outline:2px solid #2849d8;outline-offset:2px}
.skip-link{position:absolute;left:-9999px;top:0;background:#182c31;color:#ffffff;padding:8px 16px;border-radius:6px;z-index:10}
.skip-link:focus{left:8px;top:8px}
.console{display:grid;grid-template-columns:216px minmax(0,1fr);min-height:100vh}
.console-body{display:flex;flex-direction:column;min-height:100vh}
.rail{background:#15262d;color:#f3f7f2;padding:24px 16px;display:flex;flex-direction:column;gap:24px}
.rail-brand{margin:0}
.eyebrow{font-family:"IBM Plex Mono", ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;font-size:12px;line-height:18px;letter-spacing:0.08em;text-transform:uppercase;margin:0}
.rail .eyebrow{color:#bccac8;background:#15262d}
.rail-nav{list-style:none;margin:0;padding:0;display:flex;flex-direction:column;gap:4px}
.rail-link{display:block;padding:8px 12px;border-radius:6px;color:#bccac8;background:#15262d;text-decoration:none}
.rail-link:hover{color:#f3f7f2;background:#15262d;text-decoration:underline}
.rail-link[aria-current="page"]{color:#f3f7f2;background:#22383f;font-weight:700}
.rail-foot{margin:auto 0 0;padding-top:16px}
.topbar{display:flex;justify-content:space-between;align-items:baseline;gap:16px;padding:16px 32px;border-bottom:1px solid #d9dfdd;color:#182c31;background:#f5f6f1}
.topbar-scope{margin:0}
.topbar-help{color:#2849d8;background:#f5f6f1}
.main{width:100%;max-width:1440px;padding:32px;display:flex;flex-direction:column;gap:24px}
.main h1,.page-title{font-size:28px;line-height:36px;font-weight:700;margin:0}
.main h2,.section-heading,.panel-heading{font-size:18px;line-height:26px;font-weight:700;margin:0}
.footer{padding:16px 32px;color:#536568;background:#f5f6f1;border-top:1px solid #d9dfdd}
.footer p{margin:0}
.lockup{display:inline-flex;align-items:center;gap:8px}
.lockup-word{font-weight:800;letter-spacing:-0.02em}
.mark{display:block}
.action{display:inline-block;padding:8px 16px;border-radius:6px;font-family:inherit;font-size:14px;line-height:22px;font-weight:600;text-decoration:none;border:1px solid transparent;cursor:pointer}
.action-primary{background:#2849d8;color:#ffffff}
.action-secondary{background:#ffffff;color:#182c31;border-color:#7e8c91}
.notice{border-radius:8px;padding:12px 16px;margin:0}
.notice-title{margin:0;font-weight:700}
.notice-body{margin:4px 0 0}
.notice-info{background:#e9edfc;color:#2849d8}
.notice-attention{background:#e2f2a6;color:#865409}
.notice-failure{background:#f7e9e9;color:#a33638}
.notice-action{margin:8px 0 0}
.notice-action a{color:inherit;font-weight:600}
.status{display:inline-block;padding:2px 8px;border-radius:6px;font-size:12px;line-height:18px;font-weight:600;white-space:nowrap}
.status-neutral{background:#e9eceb;color:#536568}
.status-read-only{background:#e9eceb;color:#182c31}
.status-verified{background:#e2efea;color:#176b52}
.status-attention{background:#e2f2a6;color:#865409}
.status-failure{background:#f7e9e9;color:#a33638}
.status-unavailable{background:#e9eceb;color:#536568}
.status-in-progress{background:#e9edfc;color:#2849d8}
.status-completed{background:#e2efea;color:#176b52}
.status-queued{background:#e9eceb;color:#536568}
.status-pair{display:inline-flex;align-items:baseline;gap:8px}
.status-pair-label{color:#536568}
.kpi{background:#ffffff;color:#182c31;border:1px solid #d9dfdd;border-radius:8px;padding:16px;display:flex;flex-direction:column;gap:4px}
.kpi-label{font-family:"IBM Plex Mono", ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;font-size:12px;line-height:18px;letter-spacing:0.08em;text-transform:uppercase;color:#536568;background:#ffffff}
.kpi-value{font-size:28px;line-height:36px;font-weight:700}
.kpi-note{color:#536568;background:#ffffff}
.kpi-link{color:#2849d8;background:#ffffff;text-decoration:none}
.panel{background:#ffffff;color:#182c31;border:1px solid #d9dfdd;border-radius:8px;padding:16px}
.panel-body{margin:8px 0 0}
.panel-next{color:#536568;background:#ffffff;margin:12px 0 0}
.kpi-row{display:grid;grid-template-columns:repeat(auto-fit,minmax(220px,1fr));gap:16px}
.split{display:grid;grid-template-columns:minmax(0,3fr) minmax(0,2fr);gap:24px;align-items:start}
.table-wrap{overflow-x:auto;border:1px solid #d9dfdd;border-radius:8px;background:#ffffff}
.data-table{width:100%;border-collapse:collapse;font-size:14px;line-height:20px;background:#ffffff;color:#182c31}
.data-table caption,.table-caption{text-align:left;padding:8px 12px;color:#536568;background:#ffffff}
.data-table th{background:#e7eceb;color:#182c31;text-align:left;font-weight:700;padding:8px 12px;border-bottom:1px solid #d9dfdd}
.data-table td{padding:8px 12px;border-bottom:1px solid #d9dfdd;vertical-align:top}
.table-dense{font-size:13px;line-height:20px}
.cell-mono code{font-family:"IBM Plex Mono", ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;font-size:inherit}
.cell-num{text-align:right;font-variant-numeric:tabular-nums}
.cell-muted{color:#536568;background:#ffffff}
.cell-status{white-space:nowrap}
.table-note{color:#536568;background:#f5f6f1;margin:8px 0 0;font-size:12px;line-height:18px}
.empty-state{background:#ffffff;color:#182c31;border:1px dashed #7e8c91;border-radius:8px;padding:16px}
.empty-reason{margin:0}
.empty-action{display:inline-block;margin-top:8px;color:#2849d8;background:#ffffff}
.field{display:flex;flex-direction:column;gap:4px;margin:0 0 16px}
.field-label{font-weight:600}
.field-help{color:#536568;background:#ffffff;margin:0;font-size:12px;line-height:18px}
.field-error{color:#a33638;background:#ffffff;margin:0;font-size:12px;line-height:18px}
.field-invalid .input{border-color:#a33638}
.input{background:#ffffff;color:#182c31;border:1px solid #7e8c91;border-radius:6px;padding:8px 12px;font-family:inherit;font-size:14px;line-height:22px;width:100%}
.input::placeholder{color:#536568}
.checkbox{display:inline-flex;align-items:center;gap:8px}
.checkbox input{width:auto}
.form-errors{background:#f7e9e9;color:#a33638;border-radius:8px;padding:12px 16px;margin:0 0 16px}
.form-errors-list{margin:0;padding-left:20px}
.form-errors-list a{color:#a33638}
.form-actions{display:flex;gap:8px;align-items:center;margin:16px 0}
.details{background:#ffffff;color:#182c31;border:1px solid #d9dfdd;border-radius:8px;padding:12px 16px}
.details-body{margin-top:8px}
.filter-bar{display:flex;flex-wrap:wrap;gap:12px;align-items:flex-end;background:#ffffff;color:#182c31;border:1px solid #d9dfdd;border-radius:8px;padding:16px;margin:0}
.filter-field{display:flex;flex-direction:column;gap:4px}
.filter-clear{color:#2849d8;background:#ffffff}
.tabs{display:flex;gap:4px;border-bottom:1px solid #d9dfdd;margin:0}
.tab{display:inline-block;padding:8px 12px;color:#536568;background:#f5f6f1;text-decoration:none;border-bottom:2px solid transparent}
.tab[aria-current="page"]{color:#182c31;background:#ffffff;border-bottom-color:#2849d8;font-weight:700}
.pagination{display:flex;align-items:center;gap:16px;background:#f5f6f1;color:#182c31;padding:8px 0}
.pagination-link{color:#2849d8;background:#f5f6f1}
.pagination-note{color:#536568;background:#f5f6f1}
.breadcrumb{color:#536568;background:#f5f6f1}
.breadcrumb-list{list-style:none;display:flex;gap:8px;margin:0;padding:0}
.breadcrumb-item+.breadcrumb-item::before{content:"/";padding-right:8px}
.theme-form{display:flex;flex-direction:column;gap:12px;align-items:flex-start}
.theme-fieldset{border:1px solid #7e8c91;border-radius:8px;padding:12px 16px;display:flex;gap:16px;align-items:center}
.theme-legend{font-weight:600;padding:0 4px}
.theme-option{display:inline-flex;gap:6px;align-items:center}
.chart{background:#ffffff;color:#182c31;border:1px solid #d9dfdd;border-radius:12px;padding:16px;margin:0}
.chart-svg{display:block;max-width:100%;height:auto}
.chart-text{fill:#536568}
.chart-legend{list-style:none;display:flex;gap:16px;margin:8px 0 0;padding:0;color:#182c31}
.chart-legend-item{display:inline-flex;align-items:center}
.chart-legend-swatch{display:inline-block;width:10px;height:10px;border-radius:2px;margin-right:6px}
.chart-table{margin-top:8px}
.chart-unavailable{margin:0}
.chart-zero{stroke:#7e8c91;stroke-width:1}
.chart-baseline{stroke:#d9dfdd;stroke-width:1}
.sparkline{display:block;width:120px;height:32px}
.sparkline-line{fill:none;stroke-width:1.5}
.marketing-header{display:flex;align-items:center;justify-content:space-between;gap:24px;padding:20px 32px;background:#f5f6f1;color:#182c31;border-bottom:1px solid #d9dfdd}
.marketing-brand{color:#182c31;background:#f5f6f1;text-decoration:none}
.marketing-nav{display:flex;gap:16px;flex-wrap:wrap}
.marketing-nav a{color:#182c31;background:#f5f6f1}
.marketing-cta{white-space:nowrap}
.marketing-footer{padding:32px;background:#182c31;color:#f5f6f1;display:flex;flex-direction:column;gap:12px}
.marketing-line{margin:0}
.marketing-footer a{color:#e2f2a6;background:#182c31}
.marketing-footer-nav{display:flex;gap:16px;flex-wrap:wrap}
.marketing-contact{margin:0}
.label-secondary{font-family:"IBM Plex Mono", ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;font-size:12px;line-height:18px;letter-spacing:0.08em;text-transform:uppercase;color:#536568;background:#ffffff}
"#;

const DARK_CSS: &str = r#":root[data-theme="dark"] body{background:#15262d;color:#f3f7f2}
:root[data-theme="dark"] a{color:#adc4ff}
:root[data-theme="dark"] .topbar{background:#15262d;color:#f3f7f2;border-bottom-color:#22383f}
:root[data-theme="dark"] .topbar-help{color:#adc4ff;background:#15262d}
:root[data-theme="dark"] .panel{background:#22383f;color:#f3f7f2;border-color:#22383f}
:root[data-theme="dark"] .panel-next{color:#bccac8;background:#22383f}
:root[data-theme="dark"] .kpi{background:#22383f;color:#f3f7f2;border-color:#22383f}
:root[data-theme="dark"] .kpi-label{color:#bccac8;background:#22383f}
:root[data-theme="dark"] .kpi-note{color:#bccac8;background:#22383f}
:root[data-theme="dark"] .kpi-link{color:#adc4ff;background:#22383f}
:root[data-theme="dark"] .table-wrap{background:#22383f;border-color:#22383f}
:root[data-theme="dark"] .data-table{background:#22383f;color:#f3f7f2}
:root[data-theme="dark"] .data-table caption{color:#bccac8;background:#22383f}
:root[data-theme="dark"] .data-table th{background:#15262d;color:#f3f7f2;border-bottom-color:#22383f}
:root[data-theme="dark"] .data-table td{border-bottom-color:#22383f}
:root[data-theme="dark"] .cell-muted{color:#bccac8;background:#22383f}
:root[data-theme="dark"] .table-note{color:#bccac8;background:#15262d}
:root[data-theme="dark"] .empty-state{background:#22383f;color:#f3f7f2;border-color:#7e8c91}
:root[data-theme="dark"] .empty-action{color:#adc4ff;background:#22383f}
:root[data-theme="dark"] .field-label{color:#f3f7f2}
:root[data-theme="dark"] .field-help{color:#bccac8;background:#15262d}
:root[data-theme="dark"] .field-error{color:#f0a3a5;background:#15262d}
:root[data-theme="dark"] .input{background:#22383f;color:#f3f7f2;border-color:#7e8c91}
:root[data-theme="dark"] .input::placeholder{color:#bccac8}
:root[data-theme="dark"] .checkbox{color:#f3f7f2}
:root[data-theme="dark"] .details{background:#22383f;color:#f3f7f2;border-color:#22383f}
:root[data-theme="dark"] .filter-bar{background:#22383f;color:#f3f7f2;border-color:#22383f}
:root[data-theme="dark"] .filter-clear{color:#adc4ff;background:#22383f}
:root[data-theme="dark"] .tab{color:#bccac8;background:#15262d}
:root[data-theme="dark"] .tab[aria-current="page"]{color:#f3f7f2;background:#22383f;border-bottom-color:#acbeff}
:root[data-theme="dark"] .pagination{background:#15262d;color:#f3f7f2}
:root[data-theme="dark"] .pagination-link{color:#adc4ff;background:#15262d}
:root[data-theme="dark"] .pagination-note{color:#bccac8;background:#15262d}
:root[data-theme="dark"] .breadcrumb{color:#bccac8;background:#15262d}
:root[data-theme="dark"] .action-secondary{background:#22383f;color:#f3f7f2;border-color:#7e8c91}
:root[data-theme="dark"] .action-primary{background:#acbeff;color:#15262d}
:root[data-theme="dark"] .chart{background:#22383f;color:#f3f7f2;border-color:#22383f}
:root[data-theme="dark"] .chart-legend{color:#f3f7f2}
:root[data-theme="dark"] .chart-text{fill:#bccac8}
:root[data-theme="dark"] .label-secondary{color:#bccac8;background:#15262d}
:root[data-theme="dark"] .status-pair-label{color:#bccac8}
:root[data-theme="dark"] .status-neutral{background:#15262d;color:#bccac8}
:root[data-theme="dark"] .status-read-only{background:#15262d;color:#f3f7f2}
:root[data-theme="dark"] .status-verified{background:#15262d;color:#8fd6b8}
:root[data-theme="dark"] .status-attention{background:#15262d;color:#f0c98a}
:root[data-theme="dark"] .status-failure{background:#15262d;color:#f0a3a5}
:root[data-theme="dark"] .status-unavailable{background:#15262d;color:#bccac8}
:root[data-theme="dark"] .status-in-progress{background:#15262d;color:#acbeff}
:root[data-theme="dark"] .status-completed{background:#15262d;color:#8fd6b8}
:root[data-theme="dark"] .status-queued{background:#15262d;color:#bccac8}
:root[data-theme="dark"] .footer{color:#bccac8;background:#15262d;border-top-color:#22383f}
:root[data-theme="dark"] .marketing-header{background:#15262d;color:#f3f7f2;border-bottom-color:#22383f}
:root[data-theme="dark"] .marketing-brand{color:#f3f7f2;background:#15262d}
:root[data-theme="dark"] .marketing-nav a{color:#adc4ff;background:#15262d}
:root[data-theme="dark"] .theme-option{color:#f3f7f2}
"#;

const TAIL_CSS: &str = r#"@media (max-width:900px){.console{display:block}.rail{position:static;width:auto;min-height:0}.rail-nav{flex-direction:row;flex-wrap:wrap}.main{padding:20px}.split{grid-template-columns:minmax(0,1fr)}}
@media (prefers-reduced-motion: reduce){*,*::before,*::after{transition:none !important;animation:none !important;scroll-behavior:auto !important}}
"#;

/// The complete console stylesheet: one `<style>` block for every component
/// in this crate, covering the light scheme, the dark scheme (both the
/// explicit `data-theme="dark"` attribute and the `auto` media query),
/// responsive stacking below 900 CSS pixels and reduced motion.
pub fn console_css() -> &'static str {
    static CSS: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    CSS.get_or_init(build_css).as_str()
}

fn build_css() -> String {
    let dark_auto = DARK_CSS.replace(":root[data-theme=\"dark\"]", ":root[data-theme=\"auto\"]");
    let mut css = String::with_capacity(BASE_CSS.len() + DARK_CSS.len() * 2 + TAIL_CSS.len() + 64);
    css.push_str(BASE_CSS);
    css.push_str(DARK_CSS);
    css.push_str("@media (prefers-color-scheme: dark){\n");
    css.push_str(&dark_auto);
    css.push_str("}\n");
    css.push_str(TAIL_CSS);
    css
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOKEN_HEXES: [&str; 17] = [
        INK,
        PAPER,
        SURFACE,
        COBALT,
        CITRON,
        SECONDARY,
        SUCCESS,
        WARNING,
        DANGER,
        INPUT_BOUNDARY,
        SEPARATOR,
        PAGE_DARK,
        PANEL_DARK,
        TEXT_DARK,
        SECONDARY_DARK,
        ACTION_DARK,
        LINK_DARK,
    ];

    #[test]
    fn token_values_are_exact_and_lowercase() {
        assert_eq!(INK, "#182c31");
        assert_eq!(PAPER, "#f5f6f1");
        assert_eq!(SURFACE, "#ffffff");
        assert_eq!(COBALT, "#2849d8");
        assert_eq!(CITRON, "#e2f2a6");
        assert_eq!(SECONDARY, "#536568");
        assert_eq!(SUCCESS, "#176b52");
        assert_eq!(WARNING, "#865409");
        assert_eq!(DANGER, "#a33638");
        assert_eq!(INPUT_BOUNDARY, "#7e8c91");
        assert_eq!(SEPARATOR, "#d9dfdd");
        assert_eq!(PAGE_DARK, "#15262d");
        assert_eq!(PANEL_DARK, "#22383f");
        assert_eq!(TEXT_DARK, "#f3f7f2");
        assert_eq!(SECONDARY_DARK, "#bccac8");
        assert_eq!(ACTION_DARK, "#acbeff");
        assert_eq!(LINK_DARK, "#adc4ff");
        for hex in TOKEN_HEXES {
            assert_eq!(hex, hex.to_lowercase(), "token {hex} is not lowercase");
        }
    }

    #[test]
    fn spacing_radii_and_widths_are_frozen() {
        assert_eq!(SPACING_PX, [4, 8, 12, 16, 24, 32, 48, 64, 96]);
        assert_eq!(RADIUS_CONTROL, 6);
        assert_eq!(RADIUS_PANEL, 8);
        assert_eq!(RADIUS_PREVIEW, 12);
        assert_eq!(CONSOLE_NAV_WIDTH, 216);
        assert_eq!(CONSOLE_MAIN_MAX, 1440);
        assert_eq!(PAGE_PADDING, 32);
        assert_eq!(MODULE_GAP, 24);
        assert!(FONT_SANS.starts_with("Manrope"));
        assert!(FONT_MONO.starts_with("\"IBM Plex Mono\""));
    }

    #[test]
    fn stylesheet_contains_every_token_hex() {
        let css = console_css();
        for hex in TOKEN_HEXES {
            assert!(css.contains(hex), "stylesheet is missing token {hex}");
        }
        assert!(css.contains(FONT_SANS), "stylesheet must use FONT_SANS");
        assert!(css.contains(FONT_MONO), "stylesheet must use FONT_MONO");
    }

    #[test]
    fn stylesheet_braces_balance() {
        let css = console_css();
        let mut depth = 0i64;
        for ch in css.chars() {
            match ch {
                '{' => depth += 1,
                '}' => depth -= 1,
                _ => {}
            }
            assert!(depth >= 0, "stylesheet closes a brace it never opened");
        }
        assert_eq!(depth, 0, "stylesheet braces do not balance");
    }

    #[test]
    fn every_hex_in_the_stylesheet_is_lowercase_and_known() {
        let css = console_css();
        let mut found = Vec::new();
        let bytes = css.as_bytes();
        let mut index = 0usize;
        while index < bytes.len() {
            if bytes[index] == b'#' {
                let start = index + 1;
                let mut end = start;
                while end < bytes.len() && bytes[end].is_ascii_hexdigit() {
                    end += 1;
                }
                found.push(css[start..end].to_string());
                index = end;
            } else {
                index += 1;
            }
        }
        assert!(!found.is_empty());
        let known: Vec<&str> = TOKEN_HEXES
            .iter()
            .copied()
            .chain([
                INK_TINT,
                NEUTRAL_TINT,
                SUCCESS_TINT,
                COBALT_TINT,
                DANGER_TINT,
                DANGER_DARK_TEXT,
                WARNING_DARK_TEXT,
                SUCCESS_DARK_TEXT,
            ])
            .collect();
        for hex in &found {
            assert_eq!(
                *hex,
                hex.to_lowercase(),
                "stylesheet hex #{hex} must be lowercase"
            );
            let normalized = format!("#{hex}");
            assert!(
                known.contains(&normalized.as_str()),
                "stylesheet uses #{hex} which is neither a frozen token nor a declared tint"
            );
        }
    }

    #[test]
    fn stylesheet_covers_every_emitted_class() {
        let css = console_css();
        let classes = [
            "skip-link",
            "console",
            "console-body",
            "rail",
            "rail-brand",
            "eyebrow",
            "rail-nav",
            "rail-link",
            "rail-foot",
            "topbar",
            "topbar-scope",
            "topbar-help",
            "main",
            "footer",
            "lockup",
            "lockup-word",
            "mark",
            "action",
            "action-primary",
            "action-secondary",
            "notice",
            "notice-info",
            "notice-attention",
            "notice-failure",
            "notice-title",
            "notice-body",
            "notice-action",
            "status",
            "status-neutral",
            "status-read-only",
            "status-verified",
            "status-attention",
            "status-failure",
            "status-unavailable",
            "status-in-progress",
            "status-completed",
            "status-queued",
            "status-pair",
            "status-pair-label",
            "kpi",
            "kpi-label",
            "kpi-value",
            "kpi-note",
            "kpi-link",
            "panel",
            "panel-heading",
            "panel-body",
            "panel-next",
            "kpi-row",
            "split",
            "table-wrap",
            "data-table",
            "table-caption",
            "table-note",
            "table-dense",
            "cell-mono",
            "cell-num",
            "cell-status",
            "cell-muted",
            "empty-state",
            "empty-reason",
            "empty-action",
            "field",
            "field-invalid",
            "field-label",
            "field-help",
            "field-error",
            "input",
            "checkbox",
            "form-errors",
            "form-errors-list",
            "form-actions",
            "details",
            "details-body",
            "filter-bar",
            "filter-field",
            "filter-clear",
            "tabs",
            "tab",
            "pagination",
            "pagination-link",
            "pagination-note",
            "breadcrumb",
            "breadcrumb-list",
            "breadcrumb-item",
            "theme-form",
            "theme-fieldset",
            "theme-legend",
            "theme-option",
            "chart",
            "chart-svg",
            "chart-text",
            "chart-legend",
            "chart-legend-item",
            "chart-legend-swatch",
            "chart-table",
            "chart-unavailable",
            "chart-zero",
            "chart-baseline",
            "sparkline",
            "sparkline-line",
            "marketing-header",
            "marketing-brand",
            "marketing-nav",
            "marketing-cta",
            "marketing-footer",
            "marketing-line",
            "marketing-footer-nav",
            "marketing-contact",
            "label-secondary",
            "page-title",
            "section-heading",
        ];
        for class in classes {
            assert!(
                css.contains(&format!(".{class}")),
                "stylesheet has no rule for class .{class}"
            );
        }
    }

    #[test]
    fn dark_scheme_applies_by_attribute_and_by_media_query() {
        let css = console_css();
        assert!(css.contains(":root[data-theme=\"dark\"] body{background:#15262d"));
        assert!(css.contains("@media (prefers-color-scheme: dark){"));
        assert!(css.contains(":root[data-theme=\"auto\"] body{background:#15262d"));
        assert!(css.contains("@media (prefers-reduced-motion: reduce)"));
    }
}

/// WCAG contrast analysis over the stylesheet the components actually use.
#[cfg(test)]
mod contrast {
    use super::*;

    fn srgb_to_linear(channel: u8) -> f64 {
        let c = f64::from(channel) / 255.0;
        if c <= 0.040_45 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    }

    fn luminance(hex: &str) -> f64 {
        let digits = hex.trim_start_matches('#');
        let parse = |s: &str| u8::from_str_radix(s, 16).expect("hex channel");
        let (r, g, b) = (
            parse(&digits[0..2]),
            parse(&digits[2..4]),
            parse(&digits[4..6]),
        );
        0.2126 * srgb_to_linear(r) + 0.7152 * srgb_to_linear(g) + 0.0722 * srgb_to_linear(b)
    }

    fn contrast(fg: &str, bg: &str) -> f64 {
        let (l1, l2) = (luminance(fg), luminance(bg));
        let (hi, lo) = if l1 >= l2 { (l1, l2) } else { (l2, l1) };
        (hi + 0.05) / (lo + 0.05)
    }

    fn hex_after(decls: &str, property: &str) -> Option<String> {
        for decl in decls.split(';') {
            let decl = decl.trim();
            let Some((name, value)) = decl.split_once(':') else {
                continue;
            };
            if name.trim() != property {
                continue;
            }
            let value = value.trim();
            let hash = value.find('#')?;
            let rest = &value[hash + 1..];
            let end = rest
                .find(|c: char| !c.is_ascii_hexdigit())
                .unwrap_or(rest.len());
            return Some(format!("#{}", &rest[..end]));
        }
        None
    }

    /// Top-level rules `(selector, declarations)`; at-rule bodies are skipped
    /// because every media block mirrors a top-level block byte for byte.
    fn top_level_rules(css: &str) -> Vec<(String, String)> {
        let mut rules = Vec::new();
        let mut depth = 0usize;
        let mut segment_start = 0usize;
        let mut selector_end: Option<usize> = None;
        let bytes = css.as_bytes();
        for (index, byte) in bytes.iter().enumerate() {
            match byte {
                b'{' => {
                    if depth == 0 {
                        selector_end = Some(index);
                    }
                    depth += 1;
                }
                b'}' => {
                    if depth == 1 {
                        if let Some(sel_end) = selector_end {
                            let selector = css[segment_start..sel_end].trim().to_string();
                            let body = css[sel_end + 1..index].to_string();
                            if !selector.starts_with('@') {
                                rules.push((selector, body));
                            }
                        }
                        segment_start = index + 1;
                        selector_end = None;
                    }
                    depth = depth.saturating_sub(1);
                }
                _ => {}
            }
        }
        rules
    }

    fn font_size_px(decls: &str) -> Option<f64> {
        for decl in decls.split(';') {
            let Some((name, value)) = decl.trim().split_once(':') else {
                continue;
            };
            if name.trim() == "font-size" {
                let digits = value.trim().trim_end_matches("px");
                return digits.parse::<f64>().ok();
            }
        }
        None
    }

    fn font_weight(decls: &str) -> Option<f64> {
        for decl in decls.split(';') {
            let Some((name, value)) = decl.trim().split_once(':') else {
                continue;
            };
            if name.trim() == "font-weight" {
                return value.trim().parse::<f64>().ok();
            }
        }
        None
    }

    /// WCAG large text: at least 24px, or at least 18.66px bold.
    fn large_text(decls: &str) -> bool {
        let size = font_size_px(decls);
        match size {
            Some(size) if size >= 24.0 => true,
            Some(size) if size >= 18.66 => font_weight(decls).unwrap_or(400.0) >= 700.0,
            _ => false,
        }
    }

    #[test]
    fn contrast_formula_matches_reference_values() {
        let black_white = contrast("#000000", "#ffffff");
        assert!(
            (black_white - 21.0).abs() < 0.01,
            "black on white is {black_white}"
        );
        assert!((contrast("#ffffff", "#ffffff") - 1.0).abs() < 0.001);
        assert!((contrast(INK, SURFACE) - 14.55).abs() < 0.05);
        assert!((contrast(CITRON, INK) - 12.11).abs() < 0.05);
        assert!((contrast(SECONDARY, SURFACE) - 6.12).abs() < 0.05);
        assert!((contrast(COBALT, SURFACE) - 6.92).abs() < 0.05);
        assert!((contrast(SUCCESS, SURFACE) - 6.44).abs() < 0.05);
        assert!((contrast(WARNING, SURFACE) - 6.39).abs() < 0.05);
        assert!((contrast(DANGER, SURFACE) - 6.68).abs() < 0.05);
    }

    #[test]
    fn every_same_rule_pair_in_the_stylesheet_clears_wcag_aa() {
        let css = console_css();
        let mut pairs_checked = 0usize;
        for (selector, decls) in top_level_rules(css) {
            let fg = hex_after(&decls, "color");
            let bg =
                hex_after(&decls, "background").or_else(|| hex_after(&decls, "background-color"));
            let (Some(fg), Some(bg)) = (fg, bg) else {
                continue;
            };
            let ratio = contrast(&fg, &bg);
            let threshold = if large_text(&decls) { 3.0 } else { 4.5 };
            assert!(
                ratio >= threshold,
                "rule `{selector}` uses {fg} on {bg} at {ratio:.2}:1, below {threshold}"
            );
            pairs_checked += 1;
        }
        assert!(
            pairs_checked >= 60,
            "expected the stylesheet to carry many checked pairs, saw {pairs_checked}"
        );
    }

    #[test]
    fn declared_class_pairings_clear_wcag_aa() {
        // Pairs the stylesheet uses across rules (inherited backgrounds and
        // deliberate surface pairings). (name, fg, bg, large_text)
        let pairs: &[(&str, &str, &str, bool)] = &[
            ("body light", INK, PAPER, false),
            ("body dark", TEXT_DARK, PAGE_DARK, false),
            ("link light", COBALT, PAPER, false),
            ("link dark", LINK_DARK, PAGE_DARK, false),
            ("page title", INK, PAPER, true),
            ("section heading", INK, PAPER, true),
            ("rail base", TEXT_DARK, PAGE_DARK, false),
            ("rail eyebrow", SECONDARY_DARK, PAGE_DARK, false),
            ("rail link", SECONDARY_DARK, PAGE_DARK, false),
            ("rail link hover", TEXT_DARK, PAGE_DARK, false),
            ("rail link current", TEXT_DARK, PANEL_DARK, false),
            ("skip link", SURFACE, INK, false),
            ("kpi label", SECONDARY, SURFACE, false),
            ("kpi value", INK, SURFACE, true),
            ("kpi note", SECONDARY, SURFACE, false),
            ("panel heading", INK, SURFACE, true),
            ("panel body", INK, SURFACE, false),
            ("panel next", SECONDARY, SURFACE, false),
            ("panel next dark", SECONDARY_DARK, PANEL_DARK, false),
            ("table header", INK, INK_TINT, false),
            ("table cell", INK, SURFACE, false),
            ("table number", INK, SURFACE, false),
            ("table mono", INK, SURFACE, false),
            ("table muted", SECONDARY, SURFACE, false),
            ("table muted dark", SECONDARY_DARK, PANEL_DARK, false),
            ("table caption", SECONDARY, SURFACE, false),
            ("table note", SECONDARY, PAPER, false),
            ("empty reason", INK, SURFACE, false),
            ("empty action", COBALT, SURFACE, false),
            ("empty action dark", LINK_DARK, PANEL_DARK, false),
            ("field label", INK, SURFACE, false),
            ("field label dark", TEXT_DARK, PAGE_DARK, false),
            ("field help", SECONDARY, SURFACE, false),
            ("field help dark", SECONDARY_DARK, PAGE_DARK, false),
            ("field error", DANGER, SURFACE, false),
            ("field error dark", DANGER_DARK_TEXT, PAGE_DARK, false),
            ("input text", INK, SURFACE, false),
            ("input text dark", TEXT_DARK, PANEL_DARK, false),
            ("input placeholder", SECONDARY, SURFACE, false),
            ("input placeholder dark", SECONDARY_DARK, PANEL_DARK, false),
            ("checkbox label", INK, SURFACE, false),
            ("details body", INK, SURFACE, false),
            ("details dark", TEXT_DARK, PANEL_DARK, false),
            ("filter label", INK, SURFACE, false),
            ("tab idle", SECONDARY, PAPER, false),
            ("tab idle dark", SECONDARY_DARK, PAGE_DARK, false),
            ("tab current", INK, SURFACE, false),
            ("tab current dark", TEXT_DARK, PANEL_DARK, false),
            ("pagination link", COBALT, PAPER, false),
            ("pagination link dark", LINK_DARK, PAGE_DARK, false),
            ("pagination note", SECONDARY, PAPER, false),
            ("breadcrumb", SECONDARY, PAPER, false),
            ("breadcrumb dark", SECONDARY_DARK, PAGE_DARK, false),
            ("footer", SECONDARY, PAPER, false),
            ("footer dark", SECONDARY_DARK, PAGE_DARK, false),
            ("marketing header", INK, PAPER, false),
            ("marketing header dark", TEXT_DARK, PAGE_DARK, false),
            ("marketing nav", INK, PAPER, false),
            ("marketing nav dark", LINK_DARK, PAGE_DARK, false),
            ("marketing footer", PAPER, INK, false),
            ("marketing footer link", CITRON, INK, false),
            ("marketing line", PAPER, INK, false),
            ("theme legend", INK, PAPER, false),
            ("theme option", INK, PAPER, false),
            ("theme option dark", TEXT_DARK, PAGE_DARK, false),
            ("label secondary", SECONDARY, SURFACE, false),
            ("status pair label", SECONDARY, SURFACE, false),
            ("status pair label dark", SECONDARY_DARK, PANEL_DARK, false),
            ("chart text", SECONDARY, SURFACE, false),
            ("chart text dark", SECONDARY_DARK, PANEL_DARK, false),
            ("chart legend", INK, SURFACE, false),
            ("chart legend dark", TEXT_DARK, PANEL_DARK, false),
            ("notice info", COBALT, COBALT_TINT, false),
            ("notice attention", WARNING, CITRON, false),
            ("notice failure", DANGER, DANGER_TINT, false),
            ("action primary", SURFACE, COBALT, false),
            ("action primary dark", PAGE_DARK, ACTION_DARK, false),
            ("action secondary", INK, SURFACE, false),
            ("action secondary dark", TEXT_DARK, PANEL_DARK, false),
            ("form errors", DANGER, DANGER_TINT, false),
            ("chip neutral", SECONDARY, NEUTRAL_TINT, false),
            ("chip read only", INK, NEUTRAL_TINT, false),
            ("chip verified", SUCCESS, SUCCESS_TINT, false),
            ("chip attention", WARNING, CITRON, false),
            ("chip failure", DANGER, DANGER_TINT, false),
            ("chip unavailable", SECONDARY, NEUTRAL_TINT, false),
            ("chip in progress", COBALT, COBALT_TINT, false),
            ("chip completed", SUCCESS, SUCCESS_TINT, false),
            ("chip queued", SECONDARY, NEUTRAL_TINT, false),
            ("chip verified dark", SUCCESS_DARK_TEXT, PAGE_DARK, false),
            ("chip completed dark", SUCCESS_DARK_TEXT, PAGE_DARK, false),
            ("chip attention dark", WARNING_DARK_TEXT, PAGE_DARK, false),
            ("chip failure dark", DANGER_DARK_TEXT, PAGE_DARK, false),
            ("chip neutral dark", SECONDARY_DARK, PAGE_DARK, false),
            ("chip read only dark", TEXT_DARK, PAGE_DARK, false),
            ("chip unavailable dark", SECONDARY_DARK, PAGE_DARK, false),
            ("chip in progress dark", ACTION_DARK, PAGE_DARK, false),
            ("chip queued dark", SECONDARY_DARK, PAGE_DARK, false),
            ("topbar", INK, PAPER, false),
            ("topbar help", COBALT, PAPER, false),
            ("topbar dark", TEXT_DARK, PAGE_DARK, false),
            ("topbar help dark", LINK_DARK, PAGE_DARK, false),
        ];
        for (name, fg, bg, large) in pairs {
            let ratio = contrast(fg, bg);
            let threshold = if *large { 3.0 } else { 4.5 };
            assert!(
                ratio >= threshold,
                "pair `{name}` ({fg} on {bg}) is {ratio:.2}:1, below {threshold}"
            );
        }
        // The dark chip text colors resolve on panel-dark surfaces only after
        // the dark override; assert the light chip styles are untouched.
        let css = console_css();
        assert!(css.contains(".status-verified{background:#e2efea;color:#176b52}"));
    }
}
