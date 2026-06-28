use axum::Json;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, Method, StatusCode};
use axum::response::{IntoResponse, Response};

use crate::db::audit::AuditAction;
use crate::db::documents;
use crate::domain::author::Scope;
use crate::domain::document::DocumentStatus;
use crate::error::AppError;
use crate::garden;
use crate::http::AppState;
use crate::http::auth::{require_principal, require_scope};
use crate::http::documents::{DocumentEnvelope, document_not_found, owner_filter, record_audit};

pub async fn publish_document(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    method: Method,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    if method != Method::POST {
        return Err(AppError::MethodNotAllowed(vec!["POST"]));
    }
    let principal = require_principal(&headers, &state.config, &state.pool).await?;
    require_scope(&principal, Scope::Publish)?;
    let Some(document) = documents::set_document_status(
        &state.pool,
        &slug,
        DocumentStatus::Published,
        owner_filter(&principal),
    )
    .await?
    else {
        return Err(document_not_found(&slug));
    };
    // Now publicly resolvable: upgrade stubs pointing at this slug.
    garden::backfill_after_change(&state.pool, document.id, &document.slug).await;
    // Opt-in Webmention send (default OFF): notify external targets this note
    // links to. Fully inert unless INKWELL_WEBMENTION_SEND=true; always
    // best-effort and detached, so it never blocks or fails the publish.
    crate::http::webmention_send::maybe_send(&state, &document.slug, &document.body_markdown);
    record_audit(
        &state,
        &principal,
        AuditAction::Publish,
        Some(document.id),
        &document.slug,
    )
    .await;
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
    let principal = require_principal(&headers, &state.config, &state.pool).await?;
    require_scope(&principal, Scope::Publish)?;
    let Some(document) = documents::set_document_status(
        &state.pool,
        &slug,
        DocumentStatus::Draft,
        owner_filter(&principal),
    )
    .await?
    else {
        return Err(document_not_found(&slug));
    };
    // No longer publicly resolvable: downgrade links pointing at this slug to stubs.
    garden::backfill_after_change(&state.pool, document.id, &document.slug).await;
    record_audit(
        &state,
        &principal,
        AuditAction::Unpublish,
        Some(document.id),
        &document.slug,
    )
    .await;
    Ok((StatusCode::OK, Json(DocumentEnvelope::from(document))).into_response())
}
