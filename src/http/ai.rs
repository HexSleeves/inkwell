//! HTTP surfaces for the semantic layer (card T10, P3):
//!   - `GET /documents/{slug}/related` — nearest published notes by embedding.
//!   - `GET|POST /ask` — retrieve relevant chunks and synthesize an answer.
//!
//! Both reuse the centralized [`Visibility`] predicate so retrieval and
//! citations never expose a draft to a public caller (the no-draft-leak
//! invariant). Retrieval uses the configured embedder (real Voyage or the
//! deterministic mock). `/ask` synthesizes with the configured LLM and, when no
//! `ANTHROPIC_API_KEY` is set, returns a clear "AI features not configured"
//! response instead of 500ing.

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::Serialize;

use crate::ai::NO_ANSWER_MARKER;
use crate::db::chunks;
use crate::db::links::Visibility;
use crate::domain::document::StatusFilter;
use crate::error::AppError;
use crate::http::AppState;
use crate::http::auth::is_authenticated;

/// How many related notes a `/documents/{slug}/related` response returns.
const RELATED_LIMIT: i64 = 5;

/// How many chunks `/ask` retrieves to ground the answer.
const ASK_TOP_K: i64 = 6;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RelatedNoteEnvelope {
    slug: String,
    title: String,
    /// Cosine distance of the closest chunk (lower = more similar).
    distance: f64,
}

impl From<chunks::RelatedNote> for RelatedNoteEnvelope {
    fn from(value: chunks::RelatedNote) -> Self {
        Self {
            slug: value.slug,
            title: value.title,
            distance: value.distance,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RelatedResponse {
    slug: String,
    related: Vec<RelatedNoteEnvelope>,
}

/// `GET /documents/{slug}/related` — the nearest notes to this one by embedding
/// cosine distance, visibility-filtered. A note the caller cannot see 404s
/// (mirrors `get_document`); the neighbor set is fetched at the SAME visibility,
/// so a public caller never sees a draft. GET only.
pub async fn document_related(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    method: Method,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    if method != Method::GET {
        return Err(AppError::MethodNotAllowed(vec!["GET"]));
    }
    let visibility = request_visibility(&headers, &state);
    let filter = StatusFilter {
        status: visibility.status_filter(),
    };
    let Some(document) =
        crate::db::documents::get_document_by_slug(&state.pool, &slug, filter).await?
    else {
        return Err(AppError::NotFound(format!(
            "No document with slug \"{slug}\"."
        )));
    };

    // Embed the note's body and find its nearest neighbors. The mock embedder
    // never errors; a real-provider error is mapped to a 500 like any other
    // backend failure (this is a read, not a best-effort write).
    let embeddings = state
        .embedder
        .embed(std::slice::from_ref(&document.body_markdown))
        .await
        .map_err(AppError::Internal)?;
    let related = match embeddings.first() {
        Some(embedding) => {
            chunks::related_notes(
                &state.pool,
                document.id,
                embedding,
                visibility,
                RELATED_LIMIT,
            )
            .await?
        }
        None => Vec::new(),
    };

    let response = RelatedResponse {
        slug: document.slug,
        related: related.into_iter().map(RelatedNoteEnvelope::from).collect(),
    };
    Ok((StatusCode::OK, Json(response)).into_response())
}

#[derive(Default, serde::Deserialize)]
pub struct AskQuery {
    q: Option<String>,
}

#[derive(serde::Deserialize)]
struct AskBody {
    #[serde(default)]
    q: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Citation {
    slug: String,
    title: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AskResponse {
    query: String,
    answer: String,
    citations: Vec<Citation>,
}

/// `GET|POST /ask?q=...` — retrieve the top-k relevant published chunks (vector
/// search, falling back to FTS when no embeddings are stored) and synthesize an
/// answer with the configured LLM.
///
/// Retrieval is visibility-filtered, so a public answer is never grounded in a
/// draft and never cites one. When `ANTHROPIC_API_KEY` is unset the endpoint
/// returns a clear "AI features not configured" message (HTTP 200) rather than
/// 500ing. An empty query is a 400.
pub async fn ask(
    State(state): State<AppState>,
    method: Method,
    headers: HeaderMap,
    query: Query<AskQuery>,
    body: axum::body::Bytes,
) -> Result<Response, AppError> {
    let raw_query = match method {
        Method::GET => query.0.q.unwrap_or_default(),
        Method::POST => {
            if body.is_empty() {
                query.0.q.unwrap_or_default()
            } else {
                let parsed: AskBody = serde_json::from_slice(&body).map_err(|_| {
                    AppError::BadRequest("Request body must be JSON with a \"q\" field.".into())
                })?;
                parsed.q.or(query.0.q).unwrap_or_default()
            }
        }
        _ => return Err(AppError::MethodNotAllowed(vec!["GET", "POST"])),
    };
    let trimmed = raw_query.trim().to_string();
    if trimmed.is_empty() {
        return Err(AppError::BadRequest(
            "Query param \"q\" is required and must be non-empty.".into(),
        ));
    }

    // Not configured → a clear, non-500 response. The site still works.
    let Some(llm) = state.llm.clone() else {
        let response = AskResponse {
            query: trimmed,
            answer: "AI features are not configured on this site.".to_string(),
            citations: Vec::new(),
        };
        return Ok((StatusCode::OK, Json(response)).into_response());
    };

    let visibility = request_visibility(&headers, &state);

    // Vector retrieval over the question embedding; fall back to FTS when the
    // garden has no embeddings yet (e.g. a fresh import that hasn't been
    // indexed) so the answer is still grounded.
    let retrieved = retrieve_context(&state, &trimmed, visibility).await?;
    let context_blocks: Vec<String> = retrieved
        .iter()
        .map(|c| format!("Note \"{}\" ({}):\n{}", c.title, c.slug, c.content))
        .collect();

    let answer = llm
        .answer(&trimmed, &context_blocks)
        .await
        .map_err(AppError::Internal)?;

    // Cite only the notes that actually backed the answer. If the model refused
    // (no-answer marker), drop citations so a clean refusal never appears to be
    // sourced from notes it didn't use.
    let citations = if answer.trim() == NO_ANSWER_MARKER {
        Vec::new()
    } else {
        dedup_citations(&retrieved)
    };

    let response = AskResponse {
        query: trimmed,
        answer,
        citations,
    };
    Ok((StatusCode::OK, Json(response)).into_response())
}

/// Retrieve grounding chunks for `query` at `visibility`: vector search first,
/// FTS fallback when no chunk rows match (so a not-yet-indexed garden still
/// answers). Both paths are visibility-filtered.
async fn retrieve_context(
    state: &AppState,
    query: &str,
    visibility: Visibility,
) -> Result<Vec<chunks::RetrievedChunk>, AppError> {
    let embeddings = state
        .embedder
        .embed(&[query.to_string()])
        .await
        .map_err(AppError::Internal)?;
    if let Some(embedding) = embeddings.first() {
        let hits = chunks::search_chunks(&state.pool, embedding, visibility, ASK_TOP_K).await?;
        if !hits.is_empty() {
            return Ok(hits);
        }
    }
    // FTS fallback: pull the top published documents by full-text rank and use
    // their bodies as context. Only ever runs when vector search returned
    // nothing (empty or unindexed garden).
    let docs = crate::db::documents::search_published_documents(
        &state.pool,
        query,
        crate::domain::document::SearchOptions {
            limit: Some(ASK_TOP_K as u32),
            offset: Some(0),
        },
    )
    .await?;
    Ok(docs
        .into_iter()
        .map(|doc| chunks::RetrievedChunk {
            slug: doc.slug,
            title: doc.title,
            content: doc.body_markdown,
            distance: 0.0,
        })
        .collect())
}

/// Collapse retrieved chunks to one citation per source note, preserving the
/// retrieval order (most relevant first).
fn dedup_citations(retrieved: &[chunks::RetrievedChunk]) -> Vec<Citation> {
    let mut seen = std::collections::HashSet::new();
    let mut citations = Vec::new();
    for chunk in retrieved {
        if seen.insert(chunk.slug.clone()) {
            citations.push(Citation {
                slug: chunk.slug.clone(),
                title: chunk.title.clone(),
            });
        }
    }
    citations
}

/// Map the request's credentials to a read [`Visibility`] — the same rule every
/// content-exposing surface uses (authenticated ⇒ `All`, anonymous ⇒ `Public`).
fn request_visibility(headers: &HeaderMap, state: &AppState) -> Visibility {
    if is_authenticated(
        headers,
        state.config.api_key.as_deref(),
        state.config.mcp_key.as_deref(),
    ) {
        Visibility::All
    } else {
        Visibility::Public
    }
}
