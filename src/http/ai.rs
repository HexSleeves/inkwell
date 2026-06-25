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
use crate::error::AppError;
use crate::http::AppState;
use crate::http::api::resolve_visibility;

/// How many related notes a `/documents/{slug}/related` response returns.
const RELATED_LIMIT: i64 = 5;

/// How many chunks `/ask` retrieves to ground the answer.
const ASK_TOP_K: i64 = 6;

/// Max characters accepted for an `/ask` question. `/ask` is public and one
/// request can drive both Voyage (embedding) and Anthropic (synthesis), so this
/// is a deterministic, endpoint-level guard against accidental long-query cost or
/// latency BEFORE any provider work runs. Counted in characters, not bytes, so
/// non-ASCII questions are treated fairly. Not a rate limiter — that's separate.
const MAX_ASK_QUERY_CHARS: usize = 1_000;

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
    let visibility = resolve_visibility(&headers, &state).await;
    let Some(document) =
        crate::db::documents::get_document_by_slug_vis(&state.pool, &slug, visibility).await?
    else {
        return Err(AppError::NotFound(format!(
            "No document with slug \"{slug}\"."
        )));
    };

    // Use the stored chunk index to find nearest neighbors. No re-embedding is
    // needed: note_chunks already holds per-note embeddings written by the
    // indexer on every create/update. An origin note with no stored chunks
    // that match the active provider/model (e.g. unindexed or stored under a
    // different provider) returns an empty list rather than 500ing.
    let related = chunks::related_notes_for_note(
        &state.pool,
        document.id,
        visibility,
        RELATED_LIMIT,
        state.embedder.provider(),
        state.embedder.model(),
    )
    .await?;

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
    // Validate length BEFORE any provider work (embedding/synthesis) can run.
    let trimmed = validate_ask_query(raw_query)?;

    // Not configured → a clear, non-500 response. The site still works.
    let Some(llm) = state.llm.clone() else {
        let response = AskResponse {
            query: trimmed,
            answer: "AI features are not configured on this site.".to_string(),
            citations: Vec::new(),
        };
        return Ok((StatusCode::OK, Json(response)).into_response());
    };

    let visibility = resolve_visibility(&headers, &state).await;

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
    // A provider error on the query embedding (e.g. a Voyage rate limit) is not
    // fatal: fall through to full-text retrieval so /ask still answers, grounded,
    // instead of 500ing. An empty vector simply skips the vector path below.
    let embeddings = match state.embedder.embed(&[query.to_string()]).await {
        Ok(embeddings) => embeddings,
        Err(error) => {
            tracing::warn!(%error, "ask: query embedding failed (provider error/rate limit); falling back to full-text retrieval");
            Vec::new()
        }
    };
    if let Some(embedding) = embeddings.first() {
        let hits = chunks::search_chunks(
            &state.pool,
            embedding,
            visibility,
            ASK_TOP_K,
            state.embedder.provider(),
            state.embedder.model(),
        )
        .await?;
        if !hits.is_empty() {
            return Ok(hits);
        }
    }
    // FTS fallback: pull the top documents by full-text rank and use their
    // bodies as context. Only ever runs when vector search returned nothing
    // (empty or unindexed garden). Visibility-filtered to match the vector path,
    // so an authenticated owner still sees draft/unlisted context here.
    let docs = crate::db::documents::search_documents(
        &state.pool,
        query,
        visibility,
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

/// Trim and validate an `/ask` question: non-empty and at most
/// [`MAX_ASK_QUERY_CHARS`] characters. Runs before any provider call, so an
/// oversized question is a cheap `400` rather than wasted embedding/synthesis.
fn validate_ask_query(raw_query: String) -> Result<String, AppError> {
    let trimmed = raw_query.trim().to_string();
    if trimmed.is_empty() {
        return Err(AppError::BadRequest(
            "Query param \"q\" is required and must be non-empty.".into(),
        ));
    }
    if trimmed.chars().count() > MAX_ASK_QUERY_CHARS {
        return Err(AppError::BadRequest(format!(
            "Query param \"q\" must be at most {MAX_ASK_QUERY_CHARS} characters."
        )));
    }
    Ok(trimmed)
}
