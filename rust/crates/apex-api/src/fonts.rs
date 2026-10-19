//! The console origin's self-hosted web fonts.
//!
//! The design mandates Manrope and IBM Plex Mono served from the app origin
//! itself (no third-party font CDN): the shared stylesheet points at
//! `/assets/fonts/<name>.woff2` and this route answers those requests with
//! the committed bytes from `site/static/fonts/` through
//! [`apex_ui::fonts::font_bytes`]. Unknown names are a plain `404` — like the
//! favicon assets, a font request never gets the typed HTML error page.

use axum::extract::Path;
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};

/// The media type of every committed face.
pub const FONT_CONTENT_TYPE: &str = "font/woff2";
/// One year, immutable: a face name identifies one frozen file forever.
pub const FONT_CACHE_CONTROL: &str = "public, max-age=31536000, immutable";

fn asset(bytes: &'static [u8]) -> Response {
    let mut response = (StatusCode::OK, bytes).into_response();
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static(FONT_CONTENT_TYPE),
    );
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static(FONT_CACHE_CONTROL),
    );
    response
}

/// The shared design-system stylesheet: one external document per page, so
/// the edge's `style-src 'self'` policy holds and no page embeds a style
/// block. Kept short-lived because a deploy changes it without a new URL.
pub const CSS_PATH: &str = "/assets/apex.css";

/// `GET /assets/apex.css`.
pub async fn apex_css() -> Response {
    let mut response = (StatusCode::OK, apex_ui::tokens::console_css()).into_response();
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/css; charset=utf-8"),
    );
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=300"),
    );
    response
}

/// `GET /assets/fonts/{name}`.
pub async fn font(Path(name): Path<String>) -> Response {
    match apex_ui::fonts::request_bytes(&name) {
        Some(bytes) => asset(bytes),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[tokio::test]
    async fn the_shared_stylesheet_is_an_external_css_document() {
        let response = apex_css().await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response
                .headers()
                .get(header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok()),
            Some("text/css; charset=utf-8")
        );
        let body = axum::body::to_bytes(response.into_body(), 1 << 20)
            .await
            .expect("css body");
        let css = String::from_utf8(body.to_vec()).expect("utf-8 css");
        assert!(
            css.contains("@font-face"),
            "the faces travel with the sheet"
        );
        assert!(!css.contains("<script"), "the sheet carries no markup");
    }

    #[tokio::test]
    async fn every_committed_face_serves_woff2_bytes_with_immutable_caching() {
        for name in apex_ui::fonts::FONT_NAMES {
            // The stylesheet URL carries the extension; the browser asks for
            // exactly that, so the handler must answer it.
            let response = font(Path(format!("{name}.woff2"))).await;
            assert_eq!(response.status(), StatusCode::OK, "{name}.woff2");
            let response_stem = font(Path(name.to_string())).await;
            assert_eq!(response_stem.status(), StatusCode::OK, "{name} (bare stem)");
            let response = font(Path(format!("{name}.woff2"))).await;
            assert_eq!(response.status(), StatusCode::OK, "{name}");
            assert_eq!(
                response
                    .headers()
                    .get(header::CONTENT_TYPE)
                    .and_then(|value| value.to_str().ok()),
                Some(FONT_CONTENT_TYPE),
                "{name}"
            );
            assert_eq!(
                response
                    .headers()
                    .get(header::CACHE_CONTROL)
                    .and_then(|value| value.to_str().ok()),
                Some(FONT_CACHE_CONTROL),
                "{name}"
            );
            let body = axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .expect("body");
            assert!(
                body.starts_with(b"wOF2"),
                "{name} does not start with the WOFF2 magic"
            );
            assert!(body.len() > 1_000, "{name} is implausibly small");
        }
    }

    #[tokio::test]
    async fn unknown_names_are_a_plain_404() {
        for name in [
            "arial-400",
            "manrope-500",
            "..",
            "manrope-400.ttf",
            "manrope-400.woff",
        ] {
            let response = font(Path(name.to_string())).await;
            assert_eq!(response.status(), StatusCode::NOT_FOUND, "{name}");
            let body = axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .expect("body");
            assert!(body.is_empty(), "{name} must answer without a body");
        }
    }
}
