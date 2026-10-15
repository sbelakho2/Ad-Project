//! The Apex comparison mark and lockup from the identity master geometry:
//! two rising columns joined by a common reference bar.
//!
//! Master viewBox 22 x 32; column width 8, gap 6, bridge from x6 to x16 at
//! y9 to y15; both top cuts rise 4 units. One solid color, no gradient,
//! outline or shadow.

use crate::escape::attr;
use crate::tokens::FONT_SANS;

/// The three master path definitions of the mark, in drawing order.
pub const MARK_PATHS: [&str; 3] = [
    "M0 32 L8 32 L8 12 L0 16 Z",
    "M14 32 L22 32 L22 0 L14 4 Z",
    "M6 23 L16 23 L16 17 L6 17 Z",
];

/// The inline comparison mark, sized by height and filled with one solid
/// color. It is decorative (`aria-hidden`); the lockup's word carries the
/// accessible name.
pub fn mark(height_px: u32, fill: &str) -> String {
    let width = (u64::from(height_px) * 22 + 16) / 32;
    format!(
        "<svg class=\"mark\" viewBox=\"0 0 22 32\" width=\"{width}\" height=\"{height_px}\" \
         fill=\"{}\" aria-hidden=\"true\" focusable=\"false\">\
         <path d=\"{}\"/><path d=\"{}\"/><path d=\"{}\"/></svg>",
        attr(fill),
        MARK_PATHS[0],
        MARK_PATHS[1],
        MARK_PATHS[2]
    )
}

/// The mark plus the `Apex` wordmark. The word uses [`FONT_SANS`] at weight
/// 800 with -0.02em tracking and a font size of 1.375 times the mark height.
pub fn lockup(fill: &str, height_px: u32) -> String {
    let font_size = (u64::from(height_px) * 11 + 4) / 8;
    format!(
        "<span class=\"lockup\">{}<span class=\"lockup-word\" \
         style=\"font-family:{};font-weight:800;letter-spacing:-0.02em;font-size:{font_size}px\">Apex</span></span>",
        mark(height_px, fill),
        attr(FONT_SANS)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::escape::html_escape;

    #[test]
    fn master_geometry_is_exact() {
        assert_eq!(MARK_PATHS[0], "M0 32 L8 32 L8 12 L0 16 Z");
        assert_eq!(MARK_PATHS[1], "M14 32 L22 32 L22 0 L14 4 Z");
        assert_eq!(MARK_PATHS[2], "M6 23 L16 23 L16 17 L6 17 Z");
    }

    #[test]
    fn mark_scales_width_from_height_and_stays_decorative() {
        let svg = mark(32, "#ffffff");
        assert!(svg.contains("viewBox=\"0 0 22 32\""));
        assert!(svg.contains("height=\"32\""));
        assert!(svg.contains("width=\"22\""));
        assert!(svg.contains("aria-hidden=\"true\""));
        assert!(svg.contains("fill=\"#ffffff\""));
        assert_eq!(svg.matches("<path d=").count(), 3);
        let small = mark(16, "#182c31");
        assert!(small.contains("height=\"16\""));
        assert!(small.contains("width=\"11\""));
    }

    #[test]
    fn mark_escapes_the_fill_and_survives_extreme_heights() {
        let hostile = mark(20, "\" onload=\"alert(1)");
        assert!(!hostile.contains("onload=\"alert"));
        assert!(hostile.contains("&quot;"));
        let huge = mark(u32::MAX, "#000");
        assert!(huge.contains("height=\"4294967295\""));
        assert!(mark(0, "#000").contains("width=\"0\""));
    }

    #[test]
    fn lockup_renders_wordmark_with_frozen_typography() {
        let html = lockup("#182c31", 32);
        assert!(html.contains("<span class=\"lockup\">"));
        assert!(html.contains("class=\"lockup-word\""));
        assert!(html.contains(">Apex</span>"));
        assert!(html.contains("font-weight:800"));
        assert!(html.contains("letter-spacing:-0.02em"));
        assert!(html.contains("font-size:44px"));
        assert!(html.contains(html_escape(FONT_SANS).as_str()) || html.contains("Manrope"));
        let twenty = lockup("#ffffff", 20);
        assert!(twenty.contains("font-size:28px"));
    }
}
