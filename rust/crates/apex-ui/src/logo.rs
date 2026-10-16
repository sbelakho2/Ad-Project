//! The Apex comparison mark and lockup from the identity master geometry:
//! two rising columns joined by a common reference bar.
//!
//! Master viewBox 22 x 32; column width 8, gap 6, bridge from x6 to x16 at
//! y9 to y15; both top cuts rise 4 units. One solid color, no gradient,
//! outline or shadow.

use crate::escape::attr;

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

/// The mark plus the `Apex` wordmark. The word is styled entirely by the
/// shared stylesheet (`.lockup-word`: `FONT_SANS` at weight 800 with -0.02em
/// tracking); no inline style is emitted. The two design word sizes are the
/// 28px rail size for the 20px mark and the 44px size for the 32px mark and
/// above (`.lockup-word-lg`).
pub fn lockup(fill: &str, height_px: u32) -> String {
    let size_class = if height_px >= 26 {
        " lockup-word-lg"
    } else {
        ""
    };
    format!(
        "<span class=\"lockup\">{}<span class=\"lockup-word{size_class}\">Apex</span></span>",
        mark(height_px, fill)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn lockup_renders_wordmark_through_the_stylesheet() {
        let html = lockup("#182c31", 32);
        assert!(html.contains("<span class=\"lockup\">"));
        assert!(html.contains("class=\"lockup-word lockup-word-lg\">Apex</span>"));
        assert!(
            !html.contains("style="),
            "the lockup must not inline styles"
        );
        let twenty = lockup("#ffffff", 20);
        assert!(twenty.contains("class=\"lockup-word\">Apex</span>"));
        assert!(!twenty.contains("lockup-word-lg"));
        assert!(!twenty.contains("style="));
        // The typography lives in the shared stylesheet, not in the fragment.
        let css = crate::tokens::console_css();
        assert!(css.contains(".lockup-word{"));
        assert!(css.contains(".lockup-word-lg{font-size:44px}"));
    }
}
