//! Static site assets, embedded in the binary and served same-origin.
//!
//! The "Botanical Soft" theme uses Nunito, a rounded geometric sans. Chromium
//! ignores the `ui-rounded` generic family (it is Safari-only), so a real web
//! font is required for the rounded look to show in most browsers. The font is
//! served from our own origin (not a CDN) so it loads under the strict
//! `default-src 'self'` CSP with no extra `font-src` allowance, and so the
//! one-command `docker compose up` demo works fully offline.
//!
//! `nunito.woff2` is the latin-subset **variable** font (weight axis 200–1000),
//! so one ~38 KiB file covers every weight the theme asks for.

use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};

/// The embedded Nunito variable font (latin subset, woff2). Baked into the
/// binary with `include_bytes!` so there is no filesystem dependency at runtime.
static NUNITO_WOFF2: &[u8] = include_bytes!("../../assets/fonts/nunito.woff2");

/// `GET /assets/fonts/nunito.woff2` — serve the embedded font with a long,
/// immutable cache (the URL is content-stable; bust by renaming if it changes).
pub async fn nunito_font() -> Response {
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "font/woff2"),
            (header::CACHE_CONTROL, "public, max-age=31536000, immutable"),
        ],
        NUNITO_WOFF2,
    )
        .into_response()
}

/// `GET /assets/site.css` — serve the site stylesheet same-origin so pages need
/// no inline `<style>` and CSP can keep `style-src` locked to `'self'`.
pub async fn site_css() -> Response {
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "text/css; charset=utf-8"),
            (header::CACHE_CONTROL, "public, max-age=3600"),
        ],
        crate::views::layout::STYLES,
    )
        .into_response()
}
