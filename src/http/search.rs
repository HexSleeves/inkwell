use axum::Json;
use axum::extract::{Query, State};
use axum::response::{Html, IntoResponse};
use serde::Serialize;

use crate::db::documents;
use crate::domain::document::SearchOptions;
use crate::http::AppState;
use crate::views::layout::{PAGE_SIZE, derive_excerpt};
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
    Query(query): Query<SearchQuery>,
) -> impl IntoResponse {
    let raw_query = query.q.unwrap_or_default();
    let trimmed = raw_query.trim().to_string();
    let page = parse_page(query.page.as_deref());
    let wants_json = query.format.as_deref() == Some("json");
    let total = if trimmed.is_empty() {
        0
    } else {
        documents::count_search_published_documents(&state.pool, &trimmed)
            .await
            .unwrap_or_default()
    };
    let total_pages = std::cmp::max(1, (total + PAGE_SIZE - 1) / PAGE_SIZE);
    let docs = if trimmed.is_empty() {
        Vec::new()
    } else {
        documents::search_published_documents(
            &state.pool,
            &trimmed,
            SearchOptions {
                limit: Some(PAGE_SIZE as u32),
                offset: Some(((page - 1) * PAGE_SIZE) as u32),
            },
        )
        .await
        .unwrap_or_default()
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
                    excerpt: derive_excerpt(doc.body_markdown(), 160),
                    tags: doc.tags.clone(),
                    created_at: doc.created_at,
                    updated_at: doc.updated_at,
                })
                .collect(),
        };
        Json(payload).into_response()
    } else {
        Html(render_search_page(
            &raw_query,
            &docs,
            page,
            total_pages,
            state.config.site_url.as_deref(),
        ))
        .into_response()
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
