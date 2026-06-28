use axum::Json;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::Serialize;

use crate::db::documents;
use crate::db::links::Visibility;
use crate::domain::document::SearchOptions;
use crate::http::AppState;
use crate::http::api::resolve_visibility;
use crate::http::cache;
use crate::views::layout::{PAGE_SIZE, SiteMeta, derive_excerpt};
use crate::views::search::render_search_page;

#[derive(Default, serde::Deserialize)]
pub struct SearchQuery {
    q: Option<String>,
    page: Option<String>,
    format: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SearchResult {
    slug: String,
    title: String,
    excerpt: String,
    tags: Vec<String>,
    #[serde(with = "crate::domain::document::timestamp")]
    created_at: time::OffsetDateTime,
    #[serde(with = "crate::domain::document::timestamp")]
    updated_at: time::OffsetDateTime,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SearchResponse {
    query: String,
    page: i64,
    page_size: i64,
    total: i64,
    results: Vec<SearchResult>,
}

pub async fn search(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<SearchQuery>,
) -> Response {
    let raw_query = query.q.unwrap_or_default();
    let trimmed = raw_query.trim().to_string();
    let page = parse_page(query.page.as_deref());
    let wants_json = query.format.as_deref() == Some("json");

    // Only the JSON path is owner-aware: an authenticated owner finds their own
    // drafts in search (slice 3b). The HTML page is served through
    // `cache::html_response`, which emits `Cache-Control: public` with an ETag
    // over route+body — so it MUST stay public-only, or a shared cache could
    // serve one author's draft results to another caller on the same URL.
    let visibility = if wants_json {
        resolve_visibility(&headers, &state).await
    } else {
        Visibility::Public
    };

    let total = if trimmed.is_empty() {
        0
    } else {
        match documents::count_search_documents(&state.pool, &trimmed, visibility).await {
            Ok(total) => total,
            Err(_) => return error_page(),
        }
    };
    let total_pages = std::cmp::max(1, (total + PAGE_SIZE - 1) / PAGE_SIZE);
    let docs = if trimmed.is_empty() {
        Vec::new()
    } else {
        match documents::search_documents_summary(
            &state.pool,
            &trimmed,
            visibility,
            SearchOptions {
                limit: Some(PAGE_SIZE as u32),
                offset: Some(((page - 1) * PAGE_SIZE) as u32),
            },
        )
        .await
        {
            Ok(docs) => docs,
            Err(_) => return error_page(),
        }
    };

    if wants_json {
        let payload = SearchResponse {
            query: trimmed,
            page,
            page_size: PAGE_SIZE,
            total,
            results: docs
                .iter()
                .map(|doc| SearchResult {
                    slug: doc.slug.clone(),
                    title: doc.title.clone(),
                    excerpt: derive_excerpt(&doc.body_excerpt_source, 160),
                    tags: doc.tags.clone(),
                    created_at: doc.created_at,
                    updated_at: doc.updated_at,
                })
                .collect(),
        };
        Json(payload).into_response()
    } else {
        let site = SiteMeta::from_config(&state.config);
        cache::html_response(
            &headers,
            "search-html",
            StatusCode::OK,
            render_search_page(&raw_query, &docs, page, total_pages, &site),
        )
    }
}

fn parse_page(value: Option<&str>) -> i64 {
    match value {
        Some(value) if !value.is_empty() && value.chars().all(|ch| ch.is_ascii_digit()) => {
            value.parse().ok().filter(|page| *page >= 1).unwrap_or(1)
        }
        _ => 1,
    }
}

fn error_page() -> Response {
    StatusCode::INTERNAL_SERVER_ERROR.into_response()
}
