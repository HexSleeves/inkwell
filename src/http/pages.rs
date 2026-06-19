use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse};

use crate::db::documents;
use crate::domain::document::{DocumentStatus, ListByTagOptions, ListOptions, StatusFilter};
use crate::http::AppState;
use crate::views::document::{render_document_page, render_not_found_page};
use crate::views::index::render_index_page;
use crate::views::layout::PAGE_SIZE;
use crate::views::tags::{render_tag_index_page, render_tag_page};

pub async fn index(State(state): State<AppState>) -> impl IntoResponse {
    render_index(&state, 1).await
}

pub async fn index_page(
    State(state): State<AppState>,
    Path(page): Path<String>,
) -> impl IntoResponse {
    let page = parse_page_number(&page).unwrap_or(0);
    render_index(&state, page).await
}

pub async fn document_page(
    State(state): State<AppState>,
    Path(slug): Path<String>,
) -> impl IntoResponse {
    match documents::get_document_by_slug(
        &state.pool,
        &slug,
        StatusFilter {
            status: Some(DocumentStatus::Published),
        },
    )
    .await
    {
        Ok(Some(document)) => (
            StatusCode::OK,
            Html(render_document_page(
                &document,
                state.config.site_url.as_deref(),
            )),
        ),
        _ => (
            StatusCode::NOT_FOUND,
            Html(render_not_found_page(state.config.site_url.as_deref())),
        ),
    }
}

pub async fn tags_index(State(state): State<AppState>) -> impl IntoResponse {
    match documents::list_published_tags(&state.pool).await {
        Ok(tags) => (
            StatusCode::OK,
            Html(render_tag_index_page(
                &tags,
                state.config.site_url.as_deref(),
            )),
        ),
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Html(render_not_found_page(state.config.site_url.as_deref())),
        ),
    }
}

pub async fn tag_page(State(state): State<AppState>, Path(tag): Path<String>) -> impl IntoResponse {
    render_tag_listing(&state, tag, 1).await
}

pub async fn tag_page_numbered(
    State(state): State<AppState>,
    Path((tag, page)): Path<(String, String)>,
) -> impl IntoResponse {
    let Some(page) = parse_page_number(&page) else {
        return (
            StatusCode::NOT_FOUND,
            Html(render_not_found_page(state.config.site_url.as_deref())),
        );
    };
    render_tag_listing(&state, tag, page).await
}

async fn render_index(state: &AppState, page: i64) -> (StatusCode, Html<String>) {
    if page < 1 {
        return (
            StatusCode::NOT_FOUND,
            Html(render_not_found_page(state.config.site_url.as_deref())),
        );
    }
    let total = documents::count_documents(
        &state.pool,
        StatusFilter {
            status: Some(DocumentStatus::Published),
        },
    )
    .await
    .unwrap_or_default();
    let total_pages = std::cmp::max(1, (total + PAGE_SIZE - 1) / PAGE_SIZE);
    if page > total_pages {
        return (
            StatusCode::NOT_FOUND,
            Html(render_not_found_page(state.config.site_url.as_deref())),
        );
    }
    let docs = documents::list_documents(
        &state.pool,
        ListOptions {
            limit: Some(PAGE_SIZE as u32),
            offset: Some(((page - 1) * PAGE_SIZE) as u32),
            status: Some(DocumentStatus::Published),
        },
    )
    .await
    .unwrap_or_default();
    (
        StatusCode::OK,
        Html(render_index_page(
            &docs,
            page,
            total_pages,
            state.config.site_url.as_deref(),
        )),
    )
}

async fn render_tag_listing(
    state: &AppState,
    tag: String,
    page: i64,
) -> (StatusCode, Html<String>) {
    if !is_valid_tag(&tag) || page < 1 {
        return (
            StatusCode::NOT_FOUND,
            Html(render_not_found_page(state.config.site_url.as_deref())),
        );
    }
    let total = documents::count_documents_by_tag(
        &state.pool,
        &tag,
        StatusFilter {
            status: Some(DocumentStatus::Published),
        },
    )
    .await
    .unwrap_or_default();
    if total == 0 {
        return (
            StatusCode::NOT_FOUND,
            Html(render_not_found_page(state.config.site_url.as_deref())),
        );
    }
    let total_pages = std::cmp::max(1, (total + PAGE_SIZE - 1) / PAGE_SIZE);
    if page > total_pages {
        return (
            StatusCode::NOT_FOUND,
            Html(render_not_found_page(state.config.site_url.as_deref())),
        );
    }
    let docs = documents::list_documents_by_tag(
        &state.pool,
        &tag,
        ListByTagOptions {
            limit: Some(PAGE_SIZE as u32),
            offset: Some(((page - 1) * PAGE_SIZE) as u32),
            status: Some(DocumentStatus::Published),
        },
    )
    .await
    .unwrap_or_default();
    (
        StatusCode::OK,
        Html(render_tag_page(
            &tag,
            &docs,
            page,
            total_pages,
            state.config.site_url.as_deref(),
        )),
    )
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
