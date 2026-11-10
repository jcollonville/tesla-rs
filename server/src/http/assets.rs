// Copyright (C) 2026 Jean Collonvillé
// SPDX-License-Identifier: AGPL-3.0-only

//! Browser icons and the web app manifest, compiled into the binary.
//!
//! `assets/icon-1024.png` is the source artwork. The files served here are the
//! sizes generated from it. `apple-touch-icon.png` is flattened onto `#2a2a2c`
//! because iOS paints transparent pixels black.

use axum::Router;
use axum::http::{HeaderValue, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;

use crate::app::AppState;

const CACHE_CONTROL: &str = "public, max-age=31536000, immutable";

struct Asset {
    content_type: &'static str,
    body: &'static [u8],
}

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/favicon.ico", get(serve))
        .route("/favicon-32.png", get(serve))
        .route("/apple-touch-icon.png", get(serve))
        .route("/icon-192.png", get(serve))
        .route("/icon-512.png", get(serve))
        .route("/site.webmanifest", get(serve))
}

pub(super) fn is_static(path: &str) -> bool {
    lookup(path).is_some()
}

fn lookup(path: &str) -> Option<Asset> {
    let (content_type, body) = match path {
        "/favicon.ico" => (
            "image/x-icon",
            include_bytes!("../../assets/favicon.ico").as_slice(),
        ),
        "/favicon-32.png" => (
            "image/png",
            include_bytes!("../../assets/favicon-32.png").as_slice(),
        ),
        "/apple-touch-icon.png" => (
            "image/png",
            include_bytes!("../../assets/apple-touch-icon.png").as_slice(),
        ),
        "/icon-192.png" => (
            "image/png",
            include_bytes!("../../assets/icon-192.png").as_slice(),
        ),
        "/icon-512.png" => (
            "image/png",
            include_bytes!("../../assets/icon-512.png").as_slice(),
        ),
        "/site.webmanifest" => (
            "application/manifest+json",
            include_bytes!("../../assets/site.webmanifest").as_slice(),
        ),
        _ => return None,
    };
    Some(Asset { content_type, body })
}

async fn serve(uri: Uri) -> Response {
    let Some(asset) = lookup(uri.path()) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    (
        [
            (
                header::CONTENT_TYPE,
                HeaderValue::from_static(asset.content_type),
            ),
            (
                header::CACHE_CONTROL,
                HeaderValue::from_static(CACHE_CONTROL),
            ),
        ],
        asset.body,
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::{CACHE_CONTROL, lookup};
    use axum::body::Body;
    use axum::http::{Request, StatusCode, header};
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    #[test]
    fn embedded_icons_match_the_sizes_browsers_ask_for() {
        for (path, size, color_type) in [
            ("/favicon-32.png", 32, 6),
            ("/icon-192.png", 192, 6),
            ("/icon-512.png", 512, 6),
            ("/apple-touch-icon.png", 180, 2),
        ] {
            let bytes = lookup(path).expect(path).body;
            assert_eq!(&bytes[..8], &[137, 80, 78, 71, 13, 10, 26, 10], "{path}");
            assert_eq!(&bytes[12..16], b"IHDR", "{path}");
            let width = u32::from_be_bytes(bytes[16..20].try_into().unwrap());
            let height = u32::from_be_bytes(bytes[20..24].try_into().unwrap());
            assert_eq!(
                (width, height, bytes[24], bytes[25]),
                (size, size, 8, color_type),
                "{path}"
            );
        }

        let ico = lookup("/favicon.ico").expect("ico").body;
        assert_eq!(&ico[..4], &[0, 0, 1, 0]);
        assert_eq!(u16::from_le_bytes([ico[4], ico[5]]), 3);
        let mut widths: Vec<u8> = (0..3).map(|index| ico[6 + index * 16]).collect();
        widths.sort_unstable();
        assert_eq!(widths, [16, 32, 48]);

        let manifest = std::str::from_utf8(lookup("/site.webmanifest").expect("manifest").body)
            .expect("utf-8");
        assert!(manifest.contains("\"name\": \"Tesla Logger\""));
        assert!(manifest.contains("\"theme_color\": \"#2a2a2c\""));
        assert!(manifest.contains("\"background_color\": \"#2a2a2c\""));
        assert!(manifest.contains("\"/icon-192.png\""));
        assert!(manifest.contains("\"/icon-512.png\""));
        assert!(!manifest.contains("://"));
    }

    #[tokio::test]
    async fn icons_are_served_from_the_binary_with_a_long_cache() {
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgres://localhost/unused")
            .expect("lazy pool");
        let state = crate::app::test_state(pool).await;
        let cases: &[(&str, &str, &[u8])] = &[
            ("/favicon.ico", "image/x-icon", &[0, 0, 1, 0]),
            ("/favicon-32.png", "image/png", &[137, 80, 78, 71]),
            ("/apple-touch-icon.png", "image/png", &[137, 80, 78, 71]),
            ("/icon-192.png", "image/png", &[137, 80, 78, 71]),
            ("/icon-512.png", "image/png", &[137, 80, 78, 71]),
            ("/site.webmanifest", "application/manifest+json", b"{"),
        ];
        for (path, content_type, magic) in cases {
            let response = crate::http::router(state.clone())
                .oneshot(
                    Request::builder()
                        .uri(*path)
                        .body(Body::empty())
                        .expect("request"),
                )
                .await
                .expect("response");
            assert_eq!(response.status(), StatusCode::OK, "{path}");
            assert_eq!(
                response.headers().get(header::CONTENT_TYPE).unwrap(),
                content_type,
                "{path}"
            );
            assert_eq!(
                response.headers().get(header::CACHE_CONTROL).unwrap(),
                CACHE_CONTROL,
                "{path}"
            );
            assert_eq!(
                response
                    .headers()
                    .get(header::X_CONTENT_TYPE_OPTIONS)
                    .unwrap(),
                "nosniff",
                "{path}"
            );
            let body = response
                .into_body()
                .collect()
                .await
                .expect("body")
                .to_bytes();
            assert!(body.starts_with(magic), "{path}");
        }
    }
}
