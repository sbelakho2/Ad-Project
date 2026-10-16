//! The sales origin's favicon assets.
//!
//! The browser requests `/favicon.ico` from whatever origin serves the page,
//! so this service answers with the committed brand raster (the same files
//! the site ships) instead of a 404. The SVG carries the vector master for
//! clients that prefer it.

use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};

/// The committed ICO raster.
pub const FAVICON_ICO: &[u8] = include_bytes!("../../../../site/static/favicon.ico");
/// The committed vector favicon.
pub const FAVICON_SVG: &[u8] = include_bytes!("../../../../site/static/favicon.svg");

fn asset(bytes: &'static [u8], content_type: &'static str) -> Response {
    let mut response = (StatusCode::OK, bytes).into_response();
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=604800, immutable"),
    );
    response
}

/// `GET /favicon.ico`.
pub async fn ico() -> Response {
    asset(FAVICON_ICO, "image/x-icon")
}

/// `GET /favicon.svg`.
pub async fn svg() -> Response {
    asset(FAVICON_SVG, "image/svg+xml")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_committed_assets_are_the_real_files() {
        assert_eq!(&FAVICON_ICO[..4], &[0x00, 0x00, 0x01, 0x00]);
        assert!(FAVICON_ICO.len() > 1_000);
        assert!(FAVICON_SVG.starts_with(b"<svg") || FAVICON_SVG.starts_with(b"<?xml"));
        assert!(String::from_utf8_lossy(FAVICON_SVG).contains("viewBox"));
    }

    #[tokio::test]
    async fn the_routes_serve_the_assets_with_their_types() {
        let response = ico().await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response
                .headers()
                .get(header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok()),
            Some("image/x-icon")
        );
        let response = svg().await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response
                .headers()
                .get(header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok()),
            Some("image/svg+xml")
        );
    }
}
