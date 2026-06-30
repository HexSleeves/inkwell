use axum::Json;
use axum::extract::{Extension, Path, State};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::Serialize;

use crate::db::documents;
use crate::db::links::{self, Backlink, Graph, GraphEdge, GraphNode, Visibility};
use crate::error::AppError;
use crate::http::AppState;
use crate::http::auth::resolve_visibility;
use crate::http::cache;
use crate::http::documents::document_not_found;
use crate::http::security_headers::CspNonce;
use crate::views::graph::render_graph_page;
use crate::views::layout::SiteMeta;

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

/// A verified inbound Webmention surfaced alongside backlinks.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct MentionEnvelope {
    source_url: String,
}

impl From<crate::db::webmentions::Mention> for MentionEnvelope {
    fn from(value: crate::db::webmentions::Mention) -> Self {
        Self {
            source_url: value.source_url,
        }
    }
}

/// The "linked from" surface as JSON: internal backlinks plus verified external
/// Webmentions, both visibility-filtered identically (never a draft-targeting
/// mention to the public).
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct BacklinksResponse {
    backlinks: Vec<BacklinkEnvelope>,
    mentions: Vec<MentionEnvelope>,
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

/// `GET /documents/{slug}/backlinks` — the "linked from" set as JSON: internal
/// backlinks plus verified inbound Webmentions.
///
/// The target is resolved under the caller's visibility (authenticated ⇒ all
/// statuses, else published-only), mirroring document reads: a target the caller
/// cannot see 404s rather than leaking its existence. Both backlinks and
/// mentions are fetched at the SAME visibility, so a public caller never sees a
/// draft source nor a mention targeting a draft (the no-draft-leak invariant,
/// enforced by the centralized visibility predicate). GET only; 405 else.
pub async fn document_backlinks(
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
        documents::get_document_by_slug_vis(&state.pool, &slug, visibility).await?
    else {
        return Err(document_not_found(&slug));
    };
    let backlinks = links::backlinks(&state.pool, document.id, visibility).await?;
    let mentions =
        crate::db::webmentions::verified_mentions(&state.pool, document.id, visibility).await?;
    let response = BacklinksResponse {
        backlinks: backlinks.into_iter().map(BacklinkEnvelope::from).collect(),
        mentions: mentions.into_iter().map(MentionEnvelope::from).collect(),
    };
    Ok((StatusCode::OK, Json(response)).into_response())
}

/// `GET /graph` — the whole garden's bounded link graph.
///
/// Content-negotiated: a browser (any `Accept` containing `text/html`) gets the
/// interactive HTML graph page; every other caller (`application/json`, `*/*`,
/// no `Accept` — i.e. curl and API clients) gets the historical JSON envelope,
/// byte-for-byte unchanged, so the documented wire contract is preserved.
///
/// Visibility follows the same rule as document reads: an authenticated caller
/// sees every note (`All`/`Owner`), an anonymous one only published notes
/// (`Public`). The query itself enforces the no-draft-leak invariant — a public
/// graph never returns a draft node nor an edge touching one — and is hard
/// bounded by the node/edge caps in [`links`]. GET only; any other method 405s.
pub async fn graph(
    State(state): State<AppState>,
    method: Method,
    headers: HeaderMap,
    Extension(csp_nonce): Extension<CspNonce>,
) -> Result<Response, AppError> {
    if method != Method::GET {
        return Err(AppError::MethodNotAllowed(vec!["GET"]));
    }
    let visibility = resolve_visibility(&headers, &state).await;
    let graph = links::garden_graph(&state.pool, visibility).await?;

    let mut response = if wants_html(&headers) {
        let site = SiteMeta::from_config(&state.config);
        let html = render_graph_page(&graph, csp_nonce.as_str(), &site);
        // The public graph is identical for everyone, so it rides the shared HTML
        // cache (with an ETag). Authenticated representations get `no-store` below.
        if matches!(visibility, Visibility::Public) {
            cache::html_response(&headers, "graph", StatusCode::OK, html)
        } else {
            (
                StatusCode::OK,
                [(
                    header::CONTENT_TYPE,
                    HeaderValue::from_static("text/html; charset=utf-8"),
                )],
                html,
            )
                .into_response()
        }
    } else {
        (StatusCode::OK, Json(GraphEnvelope::from(graph))).into_response()
    };

    // The same URL serves multiple representations (HTML vs JSON by `Accept`), so
    // a shared cache must key on `Accept` or it could replay one representation to
    // a client that asked for the other and break the JSON wire contract.
    let response_headers = response.headers_mut();
    response_headers.insert(header::VARY, HeaderValue::from_static("Accept"));
    // Any authenticated representation — HTML or JSON — can contain the caller's
    // own drafts, so it must never be stored where another visitor could see it.
    if !matches!(visibility, Visibility::Public) {
        response_headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    }
    Ok(response)
}

/// Whether the caller prefers HTML (a browser) over the default JSON.
///
/// Honors `Accept` quality values: HTML is served only when `text/html` is
/// explicitly acceptable (`q > 0`) AND at least as preferred as
/// `application/json`. A wildcard-only (`*/*`), JSON, or absent `Accept` — curl,
/// fetch defaults, and API clients — always gets JSON, preserving the documented
/// wire contract; `text/html;q=0` (listed but refused) also falls through to JSON.
fn wants_html(headers: &HeaderMap) -> bool {
    let Some(accept) = headers
        .get(header::ACCEPT)
        .and_then(|value| value.to_str().ok())
    else {
        return false;
    };
    let mut html_q: Option<f32> = None;
    let mut json_q: Option<f32> = None;
    for part in accept.split(',') {
        let mut fields = part.split(';').map(str::trim);
        let media = fields.next().unwrap_or("").to_ascii_lowercase();
        // The `q` parameter name is case-insensitive per RFC 9110, so match it
        // on the key rather than a literal `q=` prefix (`Q=0.5` is valid).
        let q = fields
            .find_map(|field| {
                let (key, value) = field.split_once('=')?;
                key.trim().eq_ignore_ascii_case("q").then(|| value.trim())
            })
            .and_then(|value| value.parse::<f32>().ok())
            .unwrap_or(1.0);
        match media.as_str() {
            "text/html" => html_q = Some(html_q.map_or(q, |existing| existing.max(q))),
            "application/json" => json_q = Some(json_q.map_or(q, |existing| existing.max(q))),
            _ => {}
        }
    }
    match html_q {
        Some(hq) if hq > 0.0 => json_q.is_none_or(|jq| hq >= jq),
        _ => false,
    }
}

/// `GET /documents/{slug}/graph` — the one-hop neighborhood graph around a note.
///
/// Same visibility rule as [`graph`]/document reads: a note the caller cannot
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
    let visibility = resolve_visibility(&headers, &state).await;
    if documents::get_document_by_slug_vis(&state.pool, &slug, visibility)
        .await?
        .is_none()
    {
        return Err(document_not_found(&slug));
    }
    let graph = links::note_neighborhood(&state.pool, &slug, visibility).await?;
    Ok((StatusCode::OK, Json(GraphEnvelope::from(graph))).into_response())
}
