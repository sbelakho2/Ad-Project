//! The sales origin's assets: the one stylesheet and the self-hosted fonts.
//!
//! `GET /assets/apex.css` answers the exact bytes of
//! [`apex_ui::tokens::console_css`]; every sales page links it as its only
//! stylesheet. `GET /assets/fonts/{name}` answers the committed WOFF2 faces
//! through [`apex_ui::fonts::font_bytes`]; unknown names are a plain `404`.

use axum::extract::Path;
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};

use apex_ui::{fonts, tokens};

/// The one stylesheet's media type.
pub const CSS_CONTENT_TYPE: &str = "text/css; charset=utf-8";
/// The stylesheet cache directive (one hour; the sheet is versioned by
/// deployment, not by file name).
pub const CSS_CACHE_CONTROL: &str = "public, max-age=3600";
/// The media type of every committed face.
pub const FONT_CONTENT_TYPE: &str = "font/woff2";
/// One year, immutable: a face name identifies one frozen file forever.
pub const FONT_CACHE_CONTROL: &str = "public, max-age=31536000, immutable";

/// `GET /assets/apex.css`.
pub async fn css() -> Response {
    let mut response = (StatusCode::OK, tokens::console_css()).into_response();
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static(CSS_CONTENT_TYPE),
    );
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static(CSS_CACHE_CONTROL),
    );
    response
}

/// `GET /assets/fonts/{name}`.
pub async fn font(Path(name): Path<String>) -> Response {
    match fonts::request_bytes(&name) {
        Some(bytes) => {
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
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn the_stylesheet_is_the_exact_console_css_bytes() {
        let response = css().await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response
                .headers()
                .get(header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok()),
            Some(CSS_CONTENT_TYPE)
        );
        assert_eq!(
            response
                .headers()
                .get(header::CACHE_CONTROL)
                .and_then(|value| value.to_str().ok()),
            Some(CSS_CACHE_CONTROL)
        );
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        assert_eq!(body.as_ref(), tokens::console_css().as_bytes());
    }

    #[tokio::test]
    async fn every_committed_face_serves_woff2_bytes_with_immutable_caching() {
        for name in fonts::FONT_NAMES {
            let response = font(Path(name.to_string())).await;
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
            assert!(body.starts_with(b"wOF2"), "{name} lacks the WOFF2 magic");
        }
    }

    #[tokio::test]
    async fn unknown_font_names_are_a_plain_404() {
        for name in [
            "arial-400",
            "..",
            "%2e%2e",
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
