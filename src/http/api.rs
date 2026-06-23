use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::Serialize;
use serde_json::Value;
use tokio::time::{Duration, timeout};

use crate::db::documents;
use crate::db::links::{self, Backlink, Graph, GraphEdge, GraphNode, Visibility};
use crate::domain::document::{
    DEFAULT_LIMIT, Document, DocumentPatch, DocumentStatus, GrowthStage, MAX_BODY_MARKDOWN_LENGTH,
    MAX_LIMIT, MAX_REQUEST_BODY_BYTES, MAX_TITLE_LENGTH, NewDocument, StatusFilter,
};
use crate::domain::slug::{is_valid_slug, slugify};
use crate::domain::tags::normalize_tags;
use crate::error::AppError;
use crate::garden;
use crate::http::AppState;
use crate::http::auth::is_authenticated;
use crate::http::extractors::{parse_json_body, parse_non_negative_int, require_object};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DocumentEnvelope {
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
struct BacklinkEnvelope {
    slug: String,
    title: String,
    snippet: Option<String>,
}

impl From<Backlink> for BacklinkEnvelope {
    fn from(value: Backlink) -> Self {
        Self {
            slug: value.source_slug,
            title: value.source_title,
            snippet: value.context_snippet,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct GraphNodeEnvelope {
    slug: String,
    title: String,
}

impl From<GraphNode> for GraphNodeEnvelope {
    fn from(value: GraphNode) -> Self {
        Self {
            slug: value.slug,
            title: value.title,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct GraphEdgeEnvelope {
    source_slug: String,
    target_slug: String,
}

impl From<GraphEdge> for GraphEdgeEnvelope {
    fn from(value: GraphEdge) -> Self {
        Self {
            source_slug: value.source_slug,
            target_slug: value.target_slug,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct GraphEnvelope {
    nodes: Vec<GraphNodeEnvelope>,
    edges: Vec<GraphEdgeEnvelope>,
}

impl From<Graph> for GraphEnvelope {
    fn from(value: Graph) -> Self {
        Self {
            nodes: value
                .nodes
                .into_iter()
                .map(GraphNodeEnvelope::from)
                .collect(),
            edges: value
                .edges
                .into_iter()
                .map(GraphEdgeEnvelope::from)
                .collect(),
        }
    }
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

/// `GET /documents/{slug}/backlinks` — the "linked from" set as JSON.
///
/// The target is resolved under the caller's visibility (authenticated ⇒ all
/// statuses, else published-only), mirroring [`get_document`]: a target the
/// caller cannot see 404s rather than leaking its existence. Backlinks are then
/// fetched at the SAME visibility, so a public caller never sees a draft source
/// (the no-draft-leak invariant). GET only; any other method is 405.
pub async fn document_backlinks(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    method: Method,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    if method != Method::GET {
        return Err(AppError::MethodNotAllowed(vec!["GET"]));
    }
    let visibility = if is_authenticated(
        &headers,
        state.config.api_key.as_deref(),
        state.config.mcp_key.as_deref(),
    ) {
        Visibility::All
    } else {
        Visibility::Public
    };
    let filter = StatusFilter {
        status: visibility.status_filter(),
    };
    let Some(document) = documents::get_document_by_slug(&state.pool, &slug, filter).await? else {
        return Err(AppError::NotFound(format!(
            "No document with slug \"{slug}\"."
        )));
    };
    let backlinks = links::backlinks(&state.pool, document.id, visibility).await?;
    let envelopes: Vec<BacklinkEnvelope> =
        backlinks.into_iter().map(BacklinkEnvelope::from).collect();
    Ok((StatusCode::OK, Json(envelopes)).into_response())
}

/// `GET /graph` — the whole garden's bounded link graph as JSON.
///
/// Visibility follows the same rule as [`get_document`]: an authenticated
/// caller sees every note (`All`), an anonymous one only published notes
/// (`Public`). The query itself enforces the no-draft-leak invariant — a public
/// graph never returns a draft node nor an edge touching one — and is hard
/// bounded by the node/edge caps in [`links`]. GET only; any other method 405s.
pub async fn graph(
    State(state): State<AppState>,
    method: Method,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    if method != Method::GET {
        return Err(AppError::MethodNotAllowed(vec!["GET"]));
    }
    let visibility = request_visibility(&headers, &state.config);
    let graph = links::garden_graph(&state.pool, visibility).await?;
    Ok((StatusCode::OK, Json(GraphEnvelope::from(graph))).into_response())
}

/// `GET /documents/{slug}/graph` — the one-hop neighborhood graph around a note.
///
/// Same visibility rule as [`graph`]/[`get_document`]: a note the caller cannot
/// see 404s rather than leaking its existence, and the neighborhood is fetched
/// at the SAME visibility so a public caller never sees a draft neighbor or an
/// edge touching one. Bounded and depth-capped in [`links`]. GET only.
pub async fn document_graph(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    method: Method,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    if method != Method::GET {
        return Err(AppError::MethodNotAllowed(vec!["GET"]));
    }
    let visibility = request_visibility(&headers, &state.config);
    let filter = StatusFilter {
        status: visibility.status_filter(),
    };
    if documents::get_document_by_slug(&state.pool, &slug, filter)
        .await?
        .is_none()
    {
        return Err(AppError::NotFound(format!(
            "No document with slug \"{slug}\"."
        )));
    }
    let graph = links::note_neighborhood(&state.pool, &slug, visibility).await?;
    Ok((StatusCode::OK, Json(GraphEnvelope::from(graph))).into_response())
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
    require_api_key(&headers, &state.config)?;
    let Some(document) =
        documents::set_document_status(&state.pool, &slug, DocumentStatus::Published).await?
    else {
        return Err(AppError::NotFound(format!(
            "No document with slug \"{slug}\"."
        )));
    };
    // Now publicly resolvable: upgrade stubs pointing at this slug.
    garden::backfill_after_change(&state.pool, document.id, &document.slug).await;
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
    require_api_key(&headers, &state.config)?;
    let Some(document) =
        documents::set_document_status(&state.pool, &slug, DocumentStatus::Draft).await?
    else {
        return Err(AppError::NotFound(format!(
            "No document with slug \"{slug}\"."
        )));
    };
    // No longer publicly resolvable: downgrade links pointing at this slug to stubs.
    garden::backfill_after_change(&state.pool, document.id, &document.slug).await;
    Ok((StatusCode::OK, Json(DocumentEnvelope::from(document))).into_response())
}

async fn create_document(
    state: AppState,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, AppError> {
    require_api_key(&headers, &state.config)?;
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
    // Light up any existing stubs that pointed at this new slug.
    garden::backfill_after_change(&state.pool, document.id, &document.slug).await;
    Ok((StatusCode::CREATED, Json(DocumentEnvelope::from(document))).into_response())
}

async fn list_documents(
    state: AppState,
    headers: HeaderMap,
    query: ListQuery,
) -> Result<Response, AppError> {
    let authenticated = is_authenticated(
        &headers,
        state.config.api_key.as_deref(),
        state.config.mcp_key.as_deref(),
    );
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
    let authenticated = is_authenticated(
        &headers,
        state.config.api_key.as_deref(),
        state.config.mcp_key.as_deref(),
    );
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
    require_api_key(&headers, &state.config)?;
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
    if patch.title.is_none()
        && patch.body_markdown.is_none()
        && patch.tags.is_none()
        && patch.growth.is_none()
    {
        return Err(AppError::BadRequest(
            "Provide at least one of \"title\", \"bodyMarkdown\", \"tags\", or \"growth\" to update."
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
            )
            .await?
            {
                documents::ConditionalUpdate::Updated(document) => *document,
                documents::ConditionalUpdate::NotFound => {
                    return Err(AppError::NotFound(format!(
                        "No document with slug \"{slug}\"."
                    )));
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
                documents::update_document_by_slug(&state.pool, &slug, patch).await?
            else {
                return Err(AppError::NotFound(format!(
                    "No document with slug \"{slug}\"."
                )));
            };
            document
        }
    };
    // Body changed → its outbound edges changed; replace them. Best-effort for
    // the same reason as create: the update already succeeded and the note
    // renders correctly; stale edges self-heal on the next save.
    if let Some(refs) = body_refs
        && let Err(error) = garden::persist_source_edges(&state.pool, document.id, &refs).await
    {
        tracing::warn!(document_id = %document.id, %error, "persist_source_edges failed; edges may be stale until next save");
    }
    Ok((StatusCode::OK, Json(DocumentEnvelope::from(document))).into_response())
}

async fn delete_document(
    state: AppState,
    headers: HeaderMap,
    slug: String,
) -> Result<Response, AppError> {
    require_api_key(&headers, &state.config)?;
    // Resolve the note first so we can capture which sources link to it BEFORE
    // the row (and its inbound edges' target_note_id) are gone.
    let Some(document) =
        documents::get_document_by_slug(&state.pool, &slug, StatusFilter::default()).await?
    else {
        return Err(AppError::NotFound(format!(
            "No document with slug \"{slug}\"."
        )));
    };
    let affected = garden::affected_sources(&state.pool, document.id, &document.slug).await;
    if !documents::delete_document_by_slug(&state.pool, &slug).await? {
        return Err(AppError::NotFound(format!(
            "No document with slug \"{slug}\"."
        )));
    }
    // Inbound edges now dangle; re-render those sources so they fall back to stubs.
    garden::rerender_sources(&state.pool, &affected).await;
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

/// Map the request's credentials to a read [`Visibility`]: an authenticated
/// caller sees everything (`All`), an anonymous one only published content
/// (`Public`). The single place read-scope is derived for the graph surfaces,
/// mirroring the inline rule in [`document_backlinks`]/[`get_document`].
fn request_visibility(headers: &HeaderMap, config: &crate::config::Config) -> Visibility {
    if is_authenticated(
        headers,
        config.api_key.as_deref(),
        config.mcp_key.as_deref(),
    ) {
        Visibility::All
    } else {
        Visibility::Public
    }
}

/// Require a write credential: the request must carry either the configured
/// authoring key or the MCP key. Used by every mutating endpoint.
fn require_api_key(headers: &HeaderMap, config: &crate::config::Config) -> Result<(), AppError> {
    if is_authenticated(
        headers,
        config.api_key.as_deref(),
        config.mcp_key.as_deref(),
    ) {
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
