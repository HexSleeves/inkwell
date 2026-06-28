use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::Serialize;
use serde_json::Value;
use tokio::time::{Duration, timeout};

use crate::db::audit::{self, AuditAction};
use crate::db::documents;
use crate::db::links::Visibility;
use crate::domain::author::{Principal, Scope};
use crate::domain::document::{
    DEFAULT_LIMIT, Document, DocumentPatch, DocumentStatus, GrowthStage, MAX_BODY_MARKDOWN_LENGTH,
    MAX_LIMIT, MAX_REQUEST_BODY_BYTES, MAX_TITLE_LENGTH, NewDocument, StatusFilter,
};
use crate::domain::slug::{is_valid_slug, slugify};
use crate::domain::tags::normalize_tags;
use crate::error::AppError;
use crate::garden;
use crate::http::AppState;
use crate::http::auth::{authenticate, require_principal, require_scope, resolve_visibility};
use crate::http::extractors::{parse_json_body, parse_non_negative_int, require_object};

/// The canonical 404 for a document addressed by slug. Centralizes the message
/// so every handler returns an identical not-found error.
pub(crate) fn document_not_found(slug: &str) -> AppError {
    AppError::NotFound(format!("No document with slug \"{slug}\"."))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DocumentEnvelope {
    id: uuid::Uuid,
    slug: String,
    title: String,
    body_markdown: String,
    rendered_html: String,
    status: DocumentStatus,
    /// Digital-garden maturity stage (seedling | budding | evergreen).
    growth: GrowthStage,
    tags: Vec<String>,
    /// Monotonic revision counter. MCP clients echo this back as the
    /// `If-Match` header on an update to detect stale writes (409).
    version: i64,
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
            growth: value.growth,
            tags: value.tags,
            version: value.version,
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

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AuditEntryEnvelope {
    action: String,
    actor_label: String,
    #[serde(with = "crate::domain::document::timestamp")]
    at: time::OffsetDateTime,
}

impl From<audit::AuditEntry> for AuditEntryEnvelope {
    fn from(value: audit::AuditEntry) -> Self {
        Self {
            action: value.action,
            actor_label: value.actor_label,
            at: value.at,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct HistoryResponse {
    slug: String,
    history: Vec<AuditEntryEnvelope>,
}

#[derive(Default, serde::Deserialize)]
pub struct ListQuery {
    limit: Option<String>,
    offset: Option<String>,
    status: Option<String>,
}

#[derive(Default, serde::Deserialize)]
pub struct HistoryQuery {
    limit: Option<String>,
    offset: Option<String>,
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

/// `GET /documents/{slug}/history` — append-only write-audit events for one
/// document, newest first.
///
/// Unlike ordinary document reads, this surface is admin-or-owner only because
/// it exposes actor labels. An unrelated author cannot read history for another
/// author's published document even though they may read the document itself.
pub async fn document_history(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    method: Method,
    headers: HeaderMap,
    query: Query<HistoryQuery>,
) -> Result<Response, AppError> {
    if method != Method::GET {
        return Err(AppError::MethodNotAllowed(vec!["GET"]));
    }

    let Some(principal) = authenticate(&headers, &state.config, &state.pool).await else {
        return Err(document_not_found(&slug));
    };
    let owner = if principal.has(Scope::Admin) {
        None
    } else if principal.has(Scope::Read) {
        Some(principal.author_id.unwrap_or_else(uuid::Uuid::nil))
    } else {
        return Err(document_not_found(&slug));
    };

    let Some(document_id) = audit::resolve_history_document_id(&state.pool, &slug, owner).await?
    else {
        return Err(document_not_found(&slug));
    };

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
    let history = audit::list_audit_for_document(
        &state.pool,
        document_id,
        i64::from(limit),
        i64::from(offset),
    )
    .await?;
    let response = HistoryResponse {
        slug,
        history: history.into_iter().map(AuditEntryEnvelope::from).collect(),
    };
    Ok((StatusCode::OK, Json(response)).into_response())
}

async fn create_document(
    state: AppState,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, AppError> {
    let principal = require_principal(&headers, &state.config, &state.pool).await?;
    // Creating a note requires the `write` scope. Ownership is stamped from the
    // principal below; there is no existing owner to check on a create.
    require_scope(&principal, Scope::Write)?;
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
    let growth = resolve_growth(map.get("growth"))?;
    let (rendered_html, refs) = garden::render_and_resolve(&state.pool, &body_markdown).await?;
    let document = documents::create_document(
        &state.pool,
        NewDocument {
            slug,
            title,
            body_markdown,
            rendered_html,
            status: None,
            growth,
            tags,
            owner_id: principal.author_id,
        },
    )
    .await?;
    // Persist outbound edges after insert (the source id exists only now).
    // Best-effort: the document is created and already renders its own links
    // correctly; a failure here only delays backlink/fan-out metadata, which
    // rebuilds on the next save. Don't 500 a create that succeeded.
    if let Err(error) = garden::persist_source_edges(&state.pool, document.id, &refs).await {
        tracing::warn!(document_id = %document.id, %error, "persist_source_edges failed; edges rebuild on next save");
    }
    // Best-effort embedding index (mirrors the edge-persist pattern): chunk the
    // body, embed via the configured provider, upsert into note_chunks. With no
    // Voyage key the deterministic MockEmbedder is used, so chunks are still
    // indexed (just not with real semantic vectors). A failure only warns — it
    // never 500s a create that succeeded; the index rebuilds on the next save.
    if let Err(error) = crate::ai::index_note(
        &state.pool,
        state.embedder.as_ref(),
        document.id,
        document.version,
        &document.body_markdown,
    )
    .await
    {
        tracing::warn!(document_id = %document.id, %error, "index_note failed; embeddings rebuild on next save");
    }
    // Light up any existing stubs that pointed at this new slug.
    garden::backfill_after_change(&state.pool, document.id, &document.slug).await;
    record_audit(
        &state,
        &principal,
        AuditAction::Create,
        Some(document.id),
        &document.slug,
    )
    .await;
    Ok((StatusCode::CREATED, Json(DocumentEnvelope::from(document))).into_response())
}

async fn list_documents(
    state: AppState,
    headers: HeaderMap,
    query: ListQuery,
) -> Result<Response, AppError> {
    let visibility = resolve_visibility(&headers, &state).await;
    let extra_status = resolve_list_extra_status(visibility, query.status.as_deref())?;
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
    let docs =
        documents::list_documents_vis(&state.pool, visibility, extra_status, limit, offset).await?;
    let total = documents::count_documents_vis(&state.pool, visibility, extra_status).await?;
    let response = ListResponse {
        documents: docs.into_iter().map(DocumentEnvelope::from).collect(),
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
    let visibility = resolve_visibility(&headers, &state).await;
    let Some(document) =
        documents::get_document_by_slug_vis(&state.pool, &slug, visibility).await?
    else {
        // The slug may be a retired one — 301 to the document's current slug, but
        // only when that document is visible to the caller (a draft target the
        // caller can't see resolves to None and stays a 404, no existence leak).
        if let Some(current) =
            documents::resolve_alias_target(&state.pool, &slug, visibility).await?
            && let Ok(location) = HeaderValue::from_str(&format!("/documents/{current}"))
        {
            return Ok((
                StatusCode::MOVED_PERMANENTLY,
                [(axum::http::header::LOCATION, location)],
            )
                .into_response());
        }
        return Err(document_not_found(&slug));
    };
    // Advertise the current version as an ETag so clients can echo it back as
    // `If-Match` on a conditional update. RFC 7232 requires the value to be a
    // quoted string; `parse_if_match` tolerates both the quoted and bare forms.
    let etag = HeaderValue::from_str(&format!("\"{}\"", document.version))
        .unwrap_or_else(|_| HeaderValue::from_static("\"0\""));
    Ok((
        StatusCode::OK,
        [(axum::http::header::ETAG, etag)],
        Json(DocumentEnvelope::from(document)),
    )
        .into_response())
}

/// Parse an `If-Match` header into an expected version, if present.
///
/// The value is the bare version integer (matching the `ETag` we emit on GET).
/// A malformed value is a client error rather than a silent unconditional write,
/// so a stale or corrupt header can never sneak past the concurrency guard.
fn parse_if_match(headers: &HeaderMap) -> Result<Option<i64>, AppError> {
    let Some(value) = headers.get(axum::http::header::IF_MATCH) else {
        return Ok(None);
    };
    let raw = value
        .to_str()
        .map_err(|_| AppError::BadRequest("Header \"If-Match\" must be a valid integer.".into()))?
        .trim()
        // Tolerate a quoted ETag form (`"3"`) as well as the bare integer.
        .trim_matches('"');
    let version = raw.parse::<i64>().map_err(|_| {
        AppError::BadRequest("Header \"If-Match\" must be a valid integer version.".into())
    })?;
    Ok(Some(version))
}

async fn update_document(
    state: AppState,
    headers: HeaderMap,
    slug: String,
    body: Bytes,
) -> Result<Response, AppError> {
    let principal = require_principal(&headers, &state.config, &state.pool).await?;
    require_scope(&principal, Scope::Write)?;
    let owner = owner_filter(&principal);
    enforce_body_limit(&body)?;
    let value = parse_json_body(body)?;
    let map = require_object(value)?;
    let mut patch = DocumentPatch::default();
    let mut body_refs = None;
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
        let (rendered_html, refs) = garden::render_and_resolve(&state.pool, &body_markdown).await?;
        patch.rendered_html = Some(rendered_html);
        patch.body_markdown = Some(body_markdown);
        body_refs = Some(refs);
    }
    if map.contains_key("tags") {
        patch.tags = Some(resolve_tags(map.get("tags"))?);
    }
    if map.contains_key("growth") {
        patch.growth = resolve_growth(map.get("growth"))?;
    }
    if map.contains_key("slug") {
        // A rename (ADR 0011). Validate format up front so a bad slug is a 400
        // before any DB work; the db layer records the old slug as a 301 alias.
        match map.get("slug") {
            Some(Value::String(slug)) if is_valid_slug(slug) => {
                patch.new_slug = Some(slug.clone());
            }
            _ => {
                return Err(AppError::BadRequest(
                    "Field \"slug\" must be lowercase alphanumerics separated by single hyphens."
                        .to_string(),
                ));
            }
        }
    }
    if patch.title.is_none()
        && patch.body_markdown.is_none()
        && patch.tags.is_none()
        && patch.growth.is_none()
        && patch.new_slug.is_none()
    {
        return Err(AppError::BadRequest(
            "Provide at least one of \"title\", \"bodyMarkdown\", \"tags\", \"growth\", or \"slug\" to update."
                .to_string(),
        ));
    }
    // Optimistic concurrency: when the request carries `If-Match`, the update is
    // version-checked and a stale write surfaces as 409. Without `If-Match` the
    // unconditional path is preserved so the author CLI keeps working as before.
    let document = match parse_if_match(&headers)? {
        Some(expected_version) => {
            match documents::update_document_by_slug_if_version(
                &state.pool,
                &slug,
                expected_version,
                patch,
                owner,
            )
            .await?
            {
                documents::ConditionalUpdate::Updated(document) => *document,
                documents::ConditionalUpdate::NotFound => {
                    return Err(document_not_found(&slug));
                }
                documents::ConditionalUpdate::VersionMismatch { current } => {
                    return Err(AppError::Conflict(format!(
                        "Document \"{slug}\" has version {current}, not the expected {expected_version}. \
                         Re-read the note and retry with the current version."
                    )));
                }
            }
        }
        None => {
            let Some(document) =
                documents::update_document_by_slug(&state.pool, &slug, patch, owner).await?
            else {
                return Err(document_not_found(&slug));
            };
            document
        }
    };
    // Body changed → its outbound edges changed; replace them. Version-guarded
    // (against this update's version) so a slower OLDER concurrent update can't
    // overwrite a newer revision's edges — the same staleness guard index_note
    // uses for embeddings. Best-effort for the same reason as create: the update
    // already succeeded and the note renders correctly; stale edges self-heal on
    // the next save. A failure only warns and never 500s the write.
    if let Some(refs) = body_refs
        && let Err(error) = garden::persist_source_edges_if_version(
            &state.pool,
            document.id,
            document.version,
            &refs,
        )
        .await
    {
        tracing::warn!(document_id = %document.id, %error, "persist_source_edges failed; edges may be stale until next save");
    }
    // Reindex on EVERY successful update, not only body patches: a metadata-only
    // update still bumps `version`, which can cause a concurrent body update's
    // version-guarded index_note to be skipped and leave note_chunks permanently
    // stale relative to the current body. Reindexing here (against this update's
    // version) guarantees a later correction. index_note re-derives chunks from
    // `document.body_markdown`, so a tags/growth-only change re-embeds the
    // unchanged body — cheap insurance against a desynced semantic index. The
    // version guard inside index_note still drops this write if a newer revision
    // has since landed. Best-effort: a failure only warns and never 500s.
    if let Err(error) = crate::ai::index_note(
        &state.pool,
        state.embedder.as_ref(),
        document.id,
        document.version,
        &document.body_markdown,
    )
    .await
    {
        tracing::warn!(document_id = %document.id, %error, "index_note failed; embeddings may be stale until next save");
    }
    record_audit(
        &state,
        &principal,
        AuditAction::Update,
        Some(document.id),
        &document.slug,
    )
    .await;
    Ok((StatusCode::OK, Json(DocumentEnvelope::from(document))).into_response())
}

async fn delete_document(
    state: AppState,
    headers: HeaderMap,
    slug: String,
) -> Result<Response, AppError> {
    let principal = require_principal(&headers, &state.config, &state.pool).await?;
    require_scope(&principal, Scope::Write)?;
    let owner = owner_filter(&principal);
    // Resolve the note first so we can capture which sources link to it BEFORE
    // the row (and its inbound edges' target_note_id) are gone.
    let Some(document) =
        documents::get_document_by_slug(&state.pool, &slug, StatusFilter::default()).await?
    else {
        return Err(document_not_found(&slug));
    };
    let affected = garden::affected_sources(&state.pool, document.id, &document.slug).await;
    // Ownership is enforced atomically by the owner-scoped delete: a non-owner
    // (or a slug that vanished) deletes nothing → 404, no TOCTOU window.
    if !documents::delete_document_by_slug(&state.pool, &slug, owner).await? {
        return Err(document_not_found(&slug));
    }
    // Inbound edges now dangle; re-render those sources so they fall back to stubs.
    garden::rerender_sources(&state.pool, &affected).await;
    record_audit(
        &state,
        &principal,
        AuditAction::Delete,
        Some(document.id),
        &slug,
    )
    .await;
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

/// Parse the optional `growth` field into a [`GrowthStage`]. Absent/null leaves
/// it unset so the column default (create) or stored value (update) stands; a
/// present-but-unknown token is a client error rather than a silent default.
fn resolve_growth(value: Option<&Value>) -> Result<Option<GrowthStage>, AppError> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(raw)) => GrowthStage::parse(raw).map(Some).ok_or_else(|| {
            AppError::BadRequest(
                "Field \"growth\" must be one of: seedling, budding, evergreen.".to_string(),
            )
        }),
        _ => Err(AppError::BadRequest(
            "Field \"growth\" must be one of: seedling, budding, evergreen.".to_string(),
        )),
    }
}

/// Parse the optional `?status` query param into an ADDITIONAL status filter
/// on top of the visibility predicate. The param is validated uniformly for
/// every visibility — an unknown value (e.g. a `drfat` typo) is always a `400`,
/// never silently ignored:
///   - absent / `"all"` → no extra restriction (visibility already applies)
///   - `"draft"` → `Some(Draft)`; for `Public` this ANDs with the published-only
///     predicate into an empty result (anonymous callers can't see drafts)
///   - `"published"` → `Some(Published)`; redundant under `Public`, so dropped
fn resolve_list_extra_status(
    visibility: Visibility,
    raw: Option<&str>,
) -> Result<Option<DocumentStatus>, AppError> {
    let extra_status = match raw {
        None | Some("all") => None,
        Some("draft") => Some(DocumentStatus::Draft),
        Some("published") => Some(DocumentStatus::Published),
        _ => {
            return Err(AppError::BadRequest(
                "Query param \"status\" must be one of: draft, published, all.".to_string(),
            ));
        }
    };

    match (visibility, extra_status) {
        // Public is already restricted to published by the visibility predicate,
        // so an explicit `published` adds nothing.
        (Visibility::Public, Some(DocumentStatus::Published)) => Ok(None),
        _ => Ok(extra_status),
    }
}

/// The ownership constraint passed to a mutating DB query (ADR 0009 slice 3).
/// `None` for an admin (the shared key) — no owner constraint. For a non-admin,
/// the principal's author id, so the `UPDATE`/`DELETE` only matches a row that
/// principal owns and authorization is enforced ATOMICALLY in the write itself —
/// no separate check-then-write step, so no TOCTOU window where a slug is
/// deleted and recreated between an ownership check and the mutation. A non-owner
/// (or missing slug) matches no row → the handler's normal 404.
///
/// `author_id` is always `Some` for a real principal; the `nil` fallback fails
/// closed (matches no note) rather than degrading to "no constraint".
pub(crate) fn owner_filter(principal: &Principal) -> Option<uuid::Uuid> {
    if principal.has(Scope::Admin) {
        None
    } else {
        Some(principal.author_id.unwrap_or_else(uuid::Uuid::nil))
    }
}

/// Record a successful mutating action in the write-audit trail (ADR 0009, plan
/// 023). Awaited inline (bounded by [`AUDIT_INSERT_TIMEOUT`]) so the row is
/// durable before the handler responds — an audit trail that silently dropped
/// rows under load or graceful shutdown would defeat its purpose. The insert is
/// still *non-fatal*: a DB error or timeout only logs a `warn!` and the
/// successful write's response is returned regardless (a failed audit must never
/// turn a 201/200 into a 500).
///
/// As of slice 2 the write is attributed to the resolved [`Principal`]: the
/// owning author's id and label for a scoped token, or the bootstrap admin with
/// `actor_label = "shared-key"`/`"mcp-key"` for a static key.
pub(crate) async fn record_audit(
    state: &AppState,
    principal: &Principal,
    action: AuditAction,
    document_id: Option<uuid::Uuid>,
    slug: &str,
) {
    const AUDIT_INSERT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);
    let insert = audit::record_write(
        &state.pool,
        principal.author_id,
        &principal.label,
        action,
        document_id,
        Some(slug),
    );
    match tokio::time::timeout(AUDIT_INSERT_TIMEOUT, insert).await {
        Ok(Ok(())) => {}
        Ok(Err(error)) => {
            tracing::warn!(action = action.as_str(), slug, %error, "write_audit insert failed; audit row dropped");
        }
        Err(_elapsed) => {
            tracing::warn!(
                action = action.as_str(),
                slug,
                timeout_secs = AUDIT_INSERT_TIMEOUT.as_secs(),
                "write_audit insert timed out; audit row dropped"
            );
        }
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
