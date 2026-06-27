use axum::extract::{Extension, Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{Html, IntoResponse, Response};

use crate::db::documents;
use crate::db::links::{self, Visibility};
use crate::domain::document::{DocumentStatus, ListByTagOptions, ListOptions, StatusFilter};
use crate::http::AppState;
use crate::http::cache;
use crate::http::security_headers::CspNonce;
use crate::views::document::{render_document_page, render_not_found_page};
use crate::views::index::render_index_page;
use crate::views::layout::{PAGE_SIZE, SiteMeta};
use crate::views::tags::{render_tag_index_page, render_tag_page};

pub async fn index(State(state): State<AppState>, headers: HeaderMap) -> Response {
    render_index(&state, &headers, 1).await
}

pub async fn index_page(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(page): Path<String>,
) -> Response {
    let page = parse_page_number(&page).unwrap_or(0);
    render_index(&state, &headers, page).await
}

pub async fn document_page(
    State(state): State<AppState>,
    headers: HeaderMap,
    Extension(csp_nonce): Extension<CspNonce>,
    Path(slug): Path<String>,
) -> Response {
    match documents::get_document_by_slug(
        &state.pool,
        &slug,
        StatusFilter {
            status: Some(DocumentStatus::Published),
        },
    )
    .await
    {
        Ok(Some(document)) => {
            // An unauthenticated public page is always public scope, so backlinks
            // are fetched with Visibility::Public — a draft source can never leak
            // into the panel. Degrade gracefully: a missing sidebar must not 500
            // the note, so a backlinks error renders the page with no panel.
            let backlinks = match links::backlinks(&state.pool, document.id, Visibility::Public)
                .await
            {
                Ok(backlinks) => backlinks,
                Err(error) => {
                    tracing::warn!(document_id = %document.id, %error, "backlinks query failed; rendering note without the linked-from panel");
                    Vec::new()
                }
            };
            // Resolve the wikilink targets that appear inside the backlink
            // context snippets, so the panel can render them as live links (or
            // stubs). Resolution is Public, exactly like the body and the panel
            // itself: a draft target stays an unresolved stub and never leaks.
            let snippet_links = resolve_snippet_links(&state.pool, &backlinks).await;
            let site = SiteMeta::from_config(&state.config);
            cache::html_response(
                &headers,
                "document",
                StatusCode::OK,
                render_document_page(
                    &document,
                    &backlinks,
                    &snippet_links,
                    &site,
                    csp_nonce.as_str(),
                ),
            )
        }
        Ok(None) => {
            // A retired slug 301-redirects to the document's current slug, but
            // only when that document is published — the HTML page is public
            // scope, so a draft target stays a 404 and never leaks (ADR 0011).
            match documents::resolve_alias_target(&state.pool, &slug, Visibility::Public).await {
                Ok(Some(current)) => {
                    match axum::http::HeaderValue::from_str(&format!("/{current}")) {
                        Ok(location) => (
                            StatusCode::MOVED_PERMANENTLY,
                            [(axum::http::header::LOCATION, location)],
                        )
                            .into_response(),
                        Err(_) => not_found_page(&state),
                    }
                }
                Ok(None) => not_found_page(&state),
                Err(error) => {
                    tracing::warn!(%slug, %error, "alias lookup failed; rendering 404 page");
                    not_found_page(&state)
                }
            }
        }
        Err(_) => error_page(&state),
    }
}

/// The shared 404 HTML page used by the document route (and its alias fallback).
fn not_found_page(state: &AppState) -> Response {
    (
        StatusCode::NOT_FOUND,
        Html(render_not_found_page(state.config.site_url.as_deref())),
    )
        .into_response()
}

/// Resolve the `[[wikilink]]` targets that appear in backlink context snippets
/// to the set of slugs that exist in the **public** garden, so the panel can
/// render them as live links and leave the rest as stubs. Failure degrades to
/// an empty set (every snippet link renders as a stub) rather than 500-ing the
/// page.
async fn resolve_snippet_links(
    pool: &sqlx::PgPool,
    backlinks: &[links::Backlink],
) -> std::collections::HashSet<String> {
    let slugs: Vec<String> = backlinks
        .iter()
        .filter_map(|backlink| backlink.context_snippet.as_deref())
        .flat_map(crate::rendering::wikilink::extract_wikilinks)
        .map(|reference| reference.target_slug)
        .collect();
    if slugs.is_empty() {
        return std::collections::HashSet::new();
    }
    match links::resolve_slug_ids(pool, &slugs, Visibility::Public).await {
        Ok(map) => map.into_keys().collect(),
        Err(error) => {
            tracing::warn!(%error, "snippet link resolution failed; rendering backlink snippets without live links");
            std::collections::HashSet::new()
        }
    }
}

pub async fn tags_index(State(state): State<AppState>, headers: HeaderMap) -> Response {
    match documents::list_published_tags(&state.pool).await {
        Ok(tags) => {
            let site = SiteMeta::from_config(&state.config);
            cache::html_response(
                &headers,
                "tags-index",
                StatusCode::OK,
                render_tag_index_page(&tags, &site),
            )
        }
        Err(_) => error_page(&state),
    }
}

pub async fn tag_page(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(tag): Path<String>,
) -> Response {
    render_tag_listing(&state, &headers, tag, 1).await
}

pub async fn tag_page_numbered(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((tag, page)): Path<(String, String)>,
) -> Response {
    let Some(page) = parse_page_number(&page) else {
        return (
            StatusCode::NOT_FOUND,
            Html(render_not_found_page(state.config.site_url.as_deref())),
        )
            .into_response();
    };
    render_tag_listing(&state, &headers, tag, page).await
}

async fn render_index(state: &AppState, headers: &HeaderMap, page: i64) -> Response {
    if page < 1 {
        return (
            StatusCode::NOT_FOUND,
            Html(render_not_found_page(state.config.site_url.as_deref())),
        )
            .into_response();
    }

    let total = match documents::count_documents(
        &state.pool,
        StatusFilter {
            status: Some(DocumentStatus::Published),
        },
    )
    .await
    {
        Ok(total) => total,
        Err(_) => return error_page(state),
    };

    let total_pages = std::cmp::max(1, (total + PAGE_SIZE - 1) / PAGE_SIZE);
    if page > total_pages {
        return (
            StatusCode::NOT_FOUND,
            Html(render_not_found_page(state.config.site_url.as_deref())),
        )
            .into_response();
    }

    let docs = match documents::list_documents(
        &state.pool,
        ListOptions {
            limit: Some(PAGE_SIZE as u32),
            offset: Some(((page - 1) * PAGE_SIZE) as u32),
            status: Some(DocumentStatus::Published),
        },
    )
    .await
    {
        Ok(docs) => docs,
        Err(_) => return error_page(state),
    };

    let site = SiteMeta::from_config(&state.config);
    cache::html_response(
        headers,
        "index",
        StatusCode::OK,
        render_index_page(&docs, page, total_pages, &site),
    )
}

async fn render_tag_listing(
    state: &AppState,
    headers: &HeaderMap,
    tag: String,
    page: i64,
) -> Response {
    if !is_valid_tag(&tag) || page < 1 {
        return (
            StatusCode::NOT_FOUND,
            Html(render_not_found_page(state.config.site_url.as_deref())),
        )
            .into_response();
    }

    let total = match documents::count_documents_by_tag(
        &state.pool,
        &tag,
        StatusFilter {
            status: Some(DocumentStatus::Published),
        },
    )
    .await
    {
        Ok(total) => total,
        Err(_) => return error_page(state),
    };

    if total == 0 {
        return (
            StatusCode::NOT_FOUND,
            Html(render_not_found_page(state.config.site_url.as_deref())),
        )
            .into_response();
    }

    let total_pages = std::cmp::max(1, (total + PAGE_SIZE - 1) / PAGE_SIZE);
    if page > total_pages {
        return (
            StatusCode::NOT_FOUND,
            Html(render_not_found_page(state.config.site_url.as_deref())),
        )
            .into_response();
    }

    let docs = match documents::list_documents_by_tag(
        &state.pool,
        &tag,
        ListByTagOptions {
            limit: Some(PAGE_SIZE as u32),
            offset: Some(((page - 1) * PAGE_SIZE) as u32),
            status: Some(DocumentStatus::Published),
        },
    )
    .await
    {
        Ok(docs) => docs,
        Err(_) => return error_page(state),
    };

    let site = SiteMeta::from_config(&state.config);
    cache::html_response(
        headers,
        "tag-page",
        StatusCode::OK,
        render_tag_page(&tag, &docs, page, total_pages, &site),
    )
}

fn error_page(state: &AppState) -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Html(render_not_found_page(state.config.site_url.as_deref())),
    )
        .into_response()
}

fn parse_page_number(value: &str) -> Option<i64> {
    if value.is_empty() || value.starts_with('0') || !value.chars().all(|ch| ch.is_ascii_digit()) {
        return None;
    }
    value.parse().ok()
}

fn is_valid_tag(tag: &str) -> bool {
    let bytes = tag.as_bytes();
    !bytes.is_empty()
        && bytes.first() != Some(&b'-')
        && bytes.last() != Some(&b'-')
        && !tag.contains("--")
        && tag
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}
