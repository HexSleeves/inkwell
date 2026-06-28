//! Authoring web UI page handlers (CYP-42).
//!
//! These render the server-side shells for the browser editor. Like the login
//! page (ADR 0010) they are registered **only when `INKWELL_BROWSER_LOGIN=true`**;
//! with the flag off the routes do not exist and a request 404s (or falls through
//! to the public `/{slug}` route for `/editor`-shaped slugs, which is itself a
//! 404 unless a document owns that slug).
//!
//! # Auth posture
//! These handlers render HTML only — they do not themselves expose document
//! content. A visitor without an `inkwell_session` cookie is redirected to
//! `/login` for a friendlier flow, but that redirect is **not** the security
//! boundary: every data action the page performs goes through the existing
//! `/documents` JSON API, which authenticates and scope-checks each request
//! independently (a forged or cookie-less request to the API still 401/403s).
//! The cookie-presence check here is a UX convenience, nothing more.

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{Html, IntoResponse, Response};
use axum::extract::Extension;

use crate::http::AppState;
use crate::http::auth_session::extract_session_cookie;
use crate::http::security_headers::CspNonce;
use crate::views::editor::{render_editor_edit, render_editor_list, render_editor_new};
use crate::views::layout::SiteMeta;

/// Redirect a cookie-less visitor to `/login`. Returns `Some(redirect)` when no
/// session cookie is present, `None` when one is (so the caller renders the page).
fn redirect_if_signed_out(headers: &HeaderMap) -> Option<Response> {
    if extract_session_cookie(headers).is_some() {
        return None;
    }
    Some((StatusCode::SEE_OTHER, [(header::LOCATION, "/login")]).into_response())
}

/// `GET /editor` — the document list page.
pub async fn editor_list_page(
    State(state): State<AppState>,
    Extension(csp_nonce): Extension<CspNonce>,
    headers: HeaderMap,
) -> Response {
    if let Some(redirect) = redirect_if_signed_out(&headers) {
        return redirect;
    }
    let site = SiteMeta::from_config(&state.config);
    Html(render_editor_list(&site, Some(csp_nonce.as_str()))).into_response()
}

/// `GET /editor/new` — the create-document page.
pub async fn editor_new_page(
    State(state): State<AppState>,
    Extension(csp_nonce): Extension<CspNonce>,
    headers: HeaderMap,
) -> Response {
    if let Some(redirect) = redirect_if_signed_out(&headers) {
        return redirect;
    }
    let site = SiteMeta::from_config(&state.config);
    Html(render_editor_new(&site, Some(csp_nonce.as_str()))).into_response()
}

/// `GET /editor/{slug}` — the edit page for one document.
///
/// The handler does not look the document up: the inline script loads it from
/// `GET /documents/{slug}` so a single auth/visibility path governs access. A
/// missing or unauthorized slug surfaces as an in-page message, not a server 404.
pub async fn editor_edit_page(
    State(state): State<AppState>,
    Extension(csp_nonce): Extension<CspNonce>,
    Path(slug): Path<String>,
    headers: HeaderMap,
) -> Response {
    if let Some(redirect) = redirect_if_signed_out(&headers) {
        return redirect;
    }
    let site = SiteMeta::from_config(&state.config);
    Html(render_editor_edit(&site, Some(csp_nonce.as_str()), &slug)).into_response()
}
