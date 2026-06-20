use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::Serialize;
use serde_json::Value;
use tokio::time::{Duration, timeout};

use crate::db::documents;
use crate::domain::document::{
    DEFAULT_LIMIT, Document, DocumentPatch, DocumentStatus, MAX_BODY_MARKDOWN_LENGTH, MAX_LIMIT,
    MAX_REQUEST_BODY_BYTES, MAX_TITLE_LENGTH, NewDocument, StatusFilter,
};
use crate::domain::slug::{is_valid_slug, slugify};
use crate::domain::tags::normalize_tags;
use crate::error::AppError;
use crate::http::AppState;
use crate::http::auth::is_authenticated;
use crate::http::extractors::{parse_json_body, parse_non_negative_int, require_object};
use crate::rendering::render_document_html;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DocumentEnvelope {
    id: uuid::Uuid,
    slug: String,
    title: String,
    body_markdown: String,
    rendered_html: String,
    status: DocumentStatus,
    tags: Vec<String>,
    #[serde(with = "crate::domain::document::timestamp")]
    created_at: time::OffsetDateTime,
    #[serde(with = "crate::domain::document::timestamp")]
    updated_at: time::OffsetDateTime,
}

impl From<Document> for DocumentEnvelope {
    fn from(value: Document) -> Self {
        Self {
            id: value.id,
            slug: value.slug,
            title: value.title,
            body_markdown: value.body_markdown,
            rendered_html: value.rendered_html,
            status: value.status,
            tags: value.tags,
            created_at: value.created_at,
            updated_at: value.updated_at,
        }
    }
}

#[derive(Serialize)]
struct ListResponse {
    documents: Vec<DocumentEnvelope>,
    total: i64,
    limit: u32,
    offset: u32,
}

#[derive(Default, serde::Deserialize)]
pub struct ListQuery {
    limit: Option<String>,
    offset: Option<String>,
    status: Option<String>,
}

pub async fn health(
    State(state): State<AppState>,
    method: Method,
) -> Result<(StatusCode, Json<serde_json::Value>), AppError> {
    if method != Method::GET {
        return Err(AppError::MethodNotAllowed(vec!["GET"]));
    }
    let query = timeout(
        Duration::from_millis(1000),
        sqlx::query("SELECT 1").execute(&state.pool),
    )
    .await;
    if matches!(query, Ok(Ok(_))) {
        Ok((
            StatusCode::OK,
            Json(serde_json::json!({"status": "ok", "db": "up"})),
        ))
    } else {
        Ok((
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"status": "error", "db": "down"})),
        ))
    }
}

pub async fn documents(
    State(state): State<AppState>,
    method: Method,
    headers: HeaderMap,
    query: Query<ListQuery>,
    body: Bytes,
) -> Result<Response, AppError> {
    match method {
        Method::GET => list_documents(state, headers, query.0).await,
        Method::POST => create_document(state, headers, body).await,
        _ => Err(AppError::MethodNotAllowed(vec!["GET", "POST"])),
    }
}

pub async fn document(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    method: Method,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, AppError> {
    match method {
        Method::GET => get_document(state, headers, slug).await,
        Method::PATCH | Method::PUT => update_document(state, headers, slug, body).await,
        Method::DELETE => delete_document(state, headers, slug).await,
        _ => Err(AppError::MethodNotAllowed(vec![
            "GET", "PATCH", "PUT", "DELETE",
        ])),
    }
}

pub async fn publish_document(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    method: Method,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    if method != Method::POST {
        return Err(AppError::MethodNotAllowed(vec!["POST"]));
    }
    require_api_key(&headers, state.config.api_key.as_deref())?;
    let Some(document) =
        documents::set_document_status(&state.pool, &slug, DocumentStatus::Published).await?
    else {
        return Err(AppError::NotFound(format!(
            "No document with slug \"{slug}\"."
        )));
    };
    Ok((StatusCode::OK, Json(DocumentEnvelope::from(document))).into_response())
}

pub async fn unpublish_document(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    method: Method,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    if method != Method::POST {
        return Err(AppError::MethodNotAllowed(vec!["POST"]));
    }
    require_api_key(&headers, state.config.api_key.as_deref())?;
    let Some(document) =
        documents::set_document_status(&state.pool, &slug, DocumentStatus::Draft).await?
    else {
        return Err(AppError::NotFound(format!(
            "No document with slug \"{slug}\"."
        )));
    };
    Ok((StatusCode::OK, Json(DocumentEnvelope::from(document))).into_response())
}

async fn create_document(
    state: AppState,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, AppError> {
    require_api_key(&headers, state.config.api_key.as_deref())?;
    enforce_body_limit(&body)?;
    let value = parse_json_body(body)?;
    let map = require_object(value)?;
    let title = required_string(map.get("title"), "title", MAX_TITLE_LENGTH)?;
    let body_markdown = required_string(
        map.get("bodyMarkdown"),
        "bodyMarkdown",
        MAX_BODY_MARKDOWN_LENGTH,
    )?;
    let slug = resolve_slug(map.get("slug"), &title)?;
    let tags = resolve_tags(map.get("tags"))?;
    let document = documents::create_document(
        &state.pool,
        NewDocument {
            slug,
            title,
            body_markdown: body_markdown.clone(),
            rendered_html: render_document_html(&body_markdown),
            status: None,
            tags,
        },
    )
    .await?;
    Ok((StatusCode::CREATED, Json(DocumentEnvelope::from(document))).into_response())
}

async fn list_documents(
    state: AppState,
    headers: HeaderMap,
    query: ListQuery,
) -> Result<Response, AppError> {
    let authenticated = is_authenticated(&headers, state.config.api_key.as_deref());
    let status = resolve_list_status(authenticated, query.status.as_deref())?;
    let mut limit =
        parse_non_negative_int(query.limit.as_deref(), "limit")?.unwrap_or(DEFAULT_LIMIT);
    if limit < 1 {
        return Err(AppError::BadRequest(
            "Query param \"limit\" must be at least 1.".to_string(),
        ));
    }
    if limit > MAX_LIMIT {
        limit = MAX_LIMIT;
    }
    let offset = parse_non_negative_int(query.offset.as_deref(), "offset")?.unwrap_or(0);
    let filter = StatusFilter { status };
    let documents = documents::list_documents(
        &state.pool,
        crate::domain::document::ListOptions {
            limit: Some(limit),
            offset: Some(offset),
            status: filter.status.clone(),
        },
    )
    .await?;
    let total = documents::count_documents(&state.pool, filter).await?;
    let response = ListResponse {
        documents: documents.into_iter().map(DocumentEnvelope::from).collect(),
        total,
        limit,
        offset,
    };
    Ok((StatusCode::OK, Json(response)).into_response())
}

async fn get_document(
    state: AppState,
    headers: HeaderMap,
    slug: String,
) -> Result<Response, AppError> {
    let authenticated = is_authenticated(&headers, state.config.api_key.as_deref());
    let filter = if authenticated {
        StatusFilter { status: None }
    } else {
        StatusFilter {
            status: Some(DocumentStatus::Published),
        }
    };
    let Some(document) = documents::get_document_by_slug(&state.pool, &slug, filter).await? else {
        return Err(AppError::NotFound(format!(
            "No document with slug \"{slug}\"."
        )));
    };
    Ok((StatusCode::OK, Json(DocumentEnvelope::from(document))).into_response())
}

async fn update_document(
    state: AppState,
    headers: HeaderMap,
    slug: String,
    body: Bytes,
) -> Result<Response, AppError> {
    require_api_key(&headers, state.config.api_key.as_deref())?;
    enforce_body_limit(&body)?;
    let value = parse_json_body(body)?;
    let map = require_object(value)?;
    let mut patch = DocumentPatch::default();
    if map.contains_key("title") {
        patch.title = Some(required_string(
            map.get("title"),
            "title",
            MAX_TITLE_LENGTH,
        )?);
    }
    if map.contains_key("bodyMarkdown") {
        let body_markdown = required_string(
            map.get("bodyMarkdown"),
            "bodyMarkdown",
            MAX_BODY_MARKDOWN_LENGTH,
        )?;
        patch.rendered_html = Some(render_document_html(&body_markdown));
        patch.body_markdown = Some(body_markdown);
    }
    if map.contains_key("tags") {
        patch.tags = Some(resolve_tags(map.get("tags"))?);
    }
    if patch.title.is_none() && patch.body_markdown.is_none() && patch.tags.is_none() {
        return Err(AppError::BadRequest(
            "Provide at least one of \"title\", \"bodyMarkdown\", or \"tags\" to update."
                .to_string(),
        ));
    }
    let Some(document) = documents::update_document_by_slug(&state.pool, &slug, patch).await?
    else {
        return Err(AppError::NotFound(format!(
            "No document with slug \"{slug}\"."
        )));
    };
    Ok((StatusCode::OK, Json(DocumentEnvelope::from(document))).into_response())
}

async fn delete_document(
    state: AppState,
    headers: HeaderMap,
    slug: String,
) -> Result<Response, AppError> {
    require_api_key(&headers, state.config.api_key.as_deref())?;
    if !documents::delete_document_by_slug(&state.pool, &slug).await? {
        return Err(AppError::NotFound(format!(
            "No document with slug \"{slug}\"."
        )));
    }
    Ok(StatusCode::NO_CONTENT.into_response())
}

fn required_string(
    value: Option<&Value>,
    field: &str,
    max_length: usize,
) -> Result<String, AppError> {
    let Some(Value::String(value)) = value else {
        return Err(AppError::BadRequest(format!(
            "Field \"{field}\" is required and must be a non-empty string."
        )));
    };
    if value.trim().is_empty() {
        return Err(AppError::BadRequest(format!(
            "Field \"{field}\" is required and must be a non-empty string."
        )));
    }
    if value.len() > max_length {
        return Err(AppError::BadRequest(format!(
            "Field \"{field}\" must be at most {max_length} characters."
        )));
    }
    Ok(value.clone())
}

fn resolve_slug(value: Option<&Value>, title: &str) -> Result<String, AppError> {
    match value {
        None | Some(Value::Null) => {
            let slug = slugify(title);
            if slug.is_empty() {
                return Err(AppError::BadRequest(
                    "Could not derive a slug from the title; provide an explicit \"slug\"."
                        .to_string(),
                ));
            }
            Ok(slug)
        }
        Some(Value::String(slug)) if is_valid_slug(slug) => Ok(slug.clone()),
        _ => Err(AppError::BadRequest(
            "Field \"slug\" must be lowercase alphanumerics separated by single hyphens."
                .to_string(),
        )),
    }
}

fn resolve_tags(value: Option<&Value>) -> Result<Vec<String>, AppError> {
    match value {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Array(tags)) => {
            let tags = tags
                .iter()
                .map(|value| match value {
                    Value::String(value) => Ok(value.clone()),
                    _ => Err(AppError::BadRequest(
                        "Field \"tags\" must be an array of strings.".to_string(),
                    )),
                })
                .collect::<Result<Vec<_>, _>>()?;
            normalize_tags(&tags).map_err(AppError::BadRequest)
        }
        _ => Err(AppError::BadRequest(
            "Field \"tags\" must be an array of strings.".to_string(),
        )),
    }
}

fn resolve_list_status(
    authenticated: bool,
    raw: Option<&str>,
) -> Result<Option<DocumentStatus>, AppError> {
    if !authenticated {
        return Ok(Some(DocumentStatus::Published));
    }
    match raw {
        None | Some("all") => Ok(None),
        Some("draft") => Ok(Some(DocumentStatus::Draft)),
        Some("published") => Ok(Some(DocumentStatus::Published)),
        _ => Err(AppError::BadRequest(
            "Query param \"status\" must be one of: draft, published, all.".to_string(),
        )),
    }
}

fn require_api_key(headers: &HeaderMap, configured_key: Option<&str>) -> Result<(), AppError> {
    if is_authenticated(headers, configured_key) {
        Ok(())
    } else {
        Err(AppError::Unauthorized)
    }
}

/// Reject request bodies that exceed the authoring API limit before any JSON
/// parsing or allocation-heavy work. Applied uniformly to create and update so
/// neither path can be used to force a large in-memory parse.
fn enforce_body_limit(body: &Bytes) -> Result<(), AppError> {
    if body.len() > MAX_REQUEST_BODY_BYTES {
        Err(AppError::PayloadTooLarge)
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enforce_body_limit_accepts_bodies_at_or_below_the_cap() {
        let body = Bytes::from(vec![b'a'; MAX_REQUEST_BODY_BYTES]);
        assert!(enforce_body_limit(&body).is_ok());
    }

    #[test]
    fn enforce_body_limit_rejects_oversized_bodies() {
        let body = Bytes::from(vec![b'a'; MAX_REQUEST_BODY_BYTES + 1]);
        assert!(matches!(
            enforce_body_limit(&body),
            Err(AppError::PayloadTooLarge)
        ));
    }
}
