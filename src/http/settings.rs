use axum::extract::{Extension, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};

use crate::db::documents;
use crate::db::links::{self, Visibility};
use crate::domain::document::{DocumentStatus, StatusFilter};
use crate::http::AppState;
use crate::http::auth::authenticate;
use crate::http::security_headers::CspNonce;
use crate::views::layout::SiteMeta;
use crate::views::settings::{AccountPanel, GardenStats, render_settings_page};

/// `GET /settings` — the read-only "About this garden" page plus an account
/// panel. Always registered; the public section renders for everyone and the
/// account panel self-gates on `INKWELL_BROWSER_LOGIN`.
///
/// Statistics are counted at public visibility (so the numbers match what an
/// anonymous visitor can browse) and each degrades to `None` on error rather
/// than failing the page. The response is `Cache-Control: no-store` because the
/// account panel reflects the request's resolved principal — a shared cache must
/// never serve one visitor's signed-in state to another.
pub async fn settings(
    State(state): State<AppState>,
    headers: HeaderMap,
    Extension(csp_nonce): Extension<CspNonce>,
) -> Response {
    // The three counts are independent reads — run them concurrently so the page
    // waits one round-trip instead of three. Each still degrades to `None` (→ "—")
    // independently rather than failing the page.
    let (published_result, tags_result, links_result) = tokio::join!(
        documents::count_documents(
            &state.pool,
            StatusFilter {
                status: Some(DocumentStatus::Published),
            },
        ),
        documents::count_published_tags(&state.pool),
        links::count_resolved_internal_links(&state.pool, Visibility::Public),
    );
    let published = published_result
        .inspect_err(
            |error| tracing::warn!(%error, "settings: published-note count failed; showing —"),
        )
        .ok();
    let tags = tags_result
        .inspect_err(|error| tracing::warn!(%error, "settings: tag count failed; showing —"))
        .ok();
    let link_count = links_result
        .inspect_err(
            |error| tracing::warn!(%error, "settings: internal-link count failed; showing —"),
        )
        .ok();
    let stats = GardenStats {
        published,
        tags,
        links: link_count,
    };

    // The account panel is only meaningful when browser login is enabled. When
    // it is, resolve the real principal so the panel shows accurate scopes (the
    // login/media pages only check cookie presence; here we want the identity).
    let account = if !state.config.browser_login {
        AccountPanel::Disabled
    } else {
        match authenticate(&headers, &state.config, &state.pool).await {
            Some(principal) => {
                let mut scopes: Vec<String> = principal
                    .scopes
                    .iter()
                    .map(|scope| scope.as_str().to_string())
                    .collect();
                scopes.sort();
                AccountPanel::SignedIn {
                    label: principal.label,
                    scopes,
                }
            }
            None => AccountPanel::Anonymous,
        }
    };

    let site = SiteMeta::from_config(&state.config);
    let html = render_settings_page(
        &site,
        &state.config,
        &stats,
        env!("CARGO_PKG_VERSION"),
        &account,
        csp_nonce.as_str(),
    );

    (
        StatusCode::OK,
        [
            (
                header::CONTENT_TYPE,
                HeaderValue::from_static("text/html; charset=utf-8"),
            ),
            (header::CACHE_CONTROL, HeaderValue::from_static("no-store")),
        ],
        html,
    )
        .into_response()
}
