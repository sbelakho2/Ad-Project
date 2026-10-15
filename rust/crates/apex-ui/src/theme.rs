//! The light/auto/dark theme contract.
//!
//! The preference lives in the `apex_theme` cookie with the values `auto`,
//! `light` or `dark`; [`resolve`] reads an incoming cookie value and
//! [`html_attrs`] renders the attribute the stylesheet keys on. [`theme_form`]
//! is the POST settings form (three radios plus one submit button).

use crate::escape::attr;

/// The cookie that stores the theme preference.
pub const THEME_COOKIE: &str = "apex_theme";

/// The resolved theme.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Theme {
    /// Follow the operating system preference.
    Auto,
    /// Always the light scheme.
    Light,
    /// Always the dark scheme.
    Dark,
}

/// Resolve a cookie value to a theme. `dark` and `light` match
/// case-insensitively; any other value (including an absent cookie) resolves
/// to [`Theme::Auto`], so an unknown preference never traps the reader in a
/// scheme they did not choose.
pub fn resolve(cookie: Option<&str>) -> Theme {
    match cookie.map(str::trim) {
        Some(value) if value.eq_ignore_ascii_case("dark") => Theme::Dark,
        Some(value) if value.eq_ignore_ascii_case("light") => Theme::Light,
        _ => Theme::Auto,
    }
}

/// The `<html>` attribute for a resolved theme: `data-theme="auto"`,
/// `data-theme="light"` or `data-theme="dark"`.
pub fn html_attrs(theme: Theme) -> String {
    format!("data-theme=\"{}\"", name(theme))
}

/// The settings form: a POST form with the System/Light/Dark radios and a
/// submit button. `current` is checked in the group.
pub fn theme_form(action: &str, current: Theme) -> String {
    let mut out = format!(
        "<form class=\"theme-form\" method=\"post\" action=\"{}\">\
         <fieldset class=\"theme-fieldset\"><legend class=\"theme-legend\">Theme</legend>",
        attr(action)
    );
    for (value, label, theme) in [
        ("auto", "System", Theme::Auto),
        ("light", "Light", Theme::Light),
        ("dark", "Dark", Theme::Dark),
    ] {
        let checked = if theme == current { " checked" } else { "" };
        out.push_str(&format!(
            "<label class=\"theme-option\" for=\"theme-{value}\">\
             <input type=\"radio\" id=\"theme-{value}\" name=\"theme\" value=\"{value}\"{checked}> {label}</label>"
        ));
    }
    out.push_str(
        "</fieldset><button class=\"action action-secondary\" type=\"submit\">Save theme</button></form>",
    );
    out
}

pub(crate) fn name(theme: Theme) -> &'static str {
    match theme {
        Theme::Auto => "auto",
        Theme::Light => "light",
        Theme::Dark => "dark",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_maps_cookie_values_and_defaults_to_auto() {
        assert_eq!(resolve(Some("dark")), Theme::Dark);
        assert_eq!(resolve(Some("DARK")), Theme::Dark);
        assert_eq!(resolve(Some(" light ")), Theme::Light);
        assert_eq!(resolve(Some("auto")), Theme::Auto);
        assert_eq!(resolve(Some("sepia")), Theme::Auto);
        assert_eq!(resolve(None), Theme::Auto);
        assert_eq!(resolve(Some("")), Theme::Auto);
    }

    #[test]
    fn html_attrs_renders_the_attribute() {
        assert_eq!(html_attrs(Theme::Auto), "data-theme=\"auto\"");
        assert_eq!(html_attrs(Theme::Light), "data-theme=\"light\"");
        assert_eq!(html_attrs(Theme::Dark), "data-theme=\"dark\"");
    }

    #[test]
    fn theme_form_has_three_radios_and_a_submit() {
        let html = theme_form("/console/settings", Theme::Dark);
        assert!(html.contains("method=\"post\""));
        assert!(html.contains("action=\"/console/settings\""));
        assert_eq!(html.matches("type=\"radio\"").count(), 3);
        assert!(html.contains("id=\"theme-auto\" name=\"theme\" value=\"auto\""));
        assert!(html.contains("id=\"theme-light\" name=\"theme\" value=\"light\""));
        assert!(html.contains("id=\"theme-dark\" name=\"theme\" value=\"dark\" checked"));
        assert!(html.contains("> System</label>"));
        assert!(html.contains("> Light</label>"));
        assert!(html.contains("> Dark</label>"));
        assert!(html.contains("type=\"submit\">Save theme</button>"));
        assert!(html.contains("<legend class=\"theme-legend\">Theme</legend>"));
    }

    #[test]
    fn theme_form_checks_the_current_value_and_escapes_the_action() {
        let auto = theme_form("/settings", Theme::Auto);
        assert!(auto.contains("value=\"auto\" checked"));
        assert!(!auto.contains("value=\"light\" checked"));
        assert!(!auto.contains("value=\"dark\" checked"));
        let hostile = theme_form("\" onsubmit=\"alert(1)", Theme::Light);
        assert!(!hostile.contains("onsubmit=\"alert"));
        assert!(hostile.contains("&quot;"));
    }
}
