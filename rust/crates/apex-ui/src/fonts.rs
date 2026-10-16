//! The self-hosted WOFF2 faces of the design system.
//!
//! The design mandates Manrope and IBM Plex Mono served from the app origin
//! itself (no third-party font CDN): [`crate::tokens::console_css`] emits one
//! `@font-face` block per face pointing at `/assets/fonts/<name>.woff2`, and
//! the app origins answer those requests with the committed bytes through
//! this module. It is the single name-to-bytes map every origin shares.

/// The six committed faces in stylesheet order: Manrope 400/700/800 and
/// IBM Plex Mono 400/500/600.
pub const FONT_NAMES: [&str; 6] = [
    "manrope-400",
    "manrope-700",
    "manrope-800",
    "ibm-plex-mono-400",
    "ibm-plex-mono-500",
    "ibm-plex-mono-600",
];

/// The committed WOFF2 bytes for a face name, or `None` for an unknown name.
///
/// The files live in the site's static tree; the relative path mirrors the
/// cross-tree include pattern the app origins already use for the favicon.
pub fn font_bytes(name: &str) -> Option<&'static [u8]> {
    match name {
        "manrope-400" => Some(include_bytes!(
            "../../../../site/static/fonts/manrope-400.woff2"
        )),
        "manrope-700" => Some(include_bytes!(
            "../../../../site/static/fonts/manrope-700.woff2"
        )),
        "manrope-800" => Some(include_bytes!(
            "../../../../site/static/fonts/manrope-800.woff2"
        )),
        "ibm-plex-mono-400" => Some(include_bytes!(
            "../../../../site/static/fonts/ibm-plex-mono-400.woff2"
        )),
        "ibm-plex-mono-500" => Some(include_bytes!(
            "../../../../site/static/fonts/ibm-plex-mono-500.woff2"
        )),
        "ibm-plex-mono-600" => Some(include_bytes!(
            "../../../../site/static/fonts/ibm-plex-mono-600.woff2"
        )),
        _ => None,
    }
}

/// Resolve a `/assets/fonts/{name}` request segment to the committed face
/// bytes.
///
/// The stylesheet names faces in URL form (`<stem>.woff2`), which is what
/// browsers actually request; the bare stem is accepted too. Any other
/// extension or shape is refused, so only the six committed faces resolve.
pub fn request_bytes(name: &str) -> Option<&'static [u8]> {
    let stem = match name.strip_suffix(".woff2") {
        Some(stem) => stem,
        None => name,
    };
    if stem.contains('.') {
        return None;
    }
    font_bytes(stem)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_stylesheet_font_url_resolves_through_request_bytes() {
        // The contract that silently broke once: every URL the stylesheet
        // emits must resolve through the request path the origins serve.
        let css = crate::tokens::console_css();
        let mut resolved = 0;
        for name in FONT_NAMES {
            let url = format!("/assets/fonts/{name}.woff2");
            assert!(css.contains(&url), "the stylesheet must reference {url}");
            let bytes = request_bytes(&format!("{name}.woff2"))
                .unwrap_or_else(|| panic!("{url} does not resolve"));
            assert!(bytes.starts_with(b"wOF2"), "{url} is not WOFF2");
            assert!(request_bytes(name).is_some(), "the bare stem stays valid");
            resolved += 1;
        }
        assert_eq!(resolved, 6);
        assert!(request_bytes("manrope-400.ttf").is_none());
        assert!(request_bytes("manrope-400.woff").is_none());
        assert!(request_bytes("../secrets").is_none());
        assert!(request_bytes("manrope-999").is_none());
    }

    #[test]
    fn every_face_is_the_real_woff2_file() {
        for name in FONT_NAMES {
            let bytes = font_bytes(name).unwrap_or_else(|| panic!("{name} has no bytes"));
            assert!(
                bytes.starts_with(b"wOF2"),
                "{name} does not start with the WOFF2 magic"
            );
            assert!(!bytes.is_empty(), "{name} is empty");
        }
    }

    #[test]
    fn unknown_names_have_no_bytes() {
        assert!(font_bytes("arial-400").is_none());
        assert!(font_bytes("manrope-500").is_none());
        assert!(font_bytes("").is_none());
        assert!(font_bytes("MANROPE-400").is_none());
        assert!(font_bytes("../manrope-400").is_none());
    }

    #[test]
    fn the_face_list_is_exactly_the_six_committed_names() {
        assert_eq!(FONT_NAMES.len(), 6);
        assert_eq!(
            FONT_NAMES,
            [
                "manrope-400",
                "manrope-700",
                "manrope-800",
                "ibm-plex-mono-400",
                "ibm-plex-mono-500",
                "ibm-plex-mono-600",
            ]
        );
    }
}
