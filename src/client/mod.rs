//! Shared HTTP client for the authenticated write API.
//!
//! [`InkwellClient`] is a thin [`reqwest`] wrapper that speaks the same write
//! API any other client would. It owns the transport concerns — the resolved
//! base URL, the `X-API-Key` header, status-code handling, the client-side
//! body cap, and the server error envelope — so callers like the `inkwell
//! author` CLI can stay focused on authoring policy.

use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};

use crate::domain::document::MAX_BODY_MARKDOWN_LENGTH;

/// Time to wait for a connection to be established before giving up.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Overall time budget for a single request, so stalled connections can't
/// block CLI operations indefinitely.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);

/// A document to push to the write API: the resolved slug plus its contents.
///
/// This is the client's small owned input so the client never depends on the
/// CLI's `ParsedDocument`. Callers resolve the slug (their authoring policy)
/// and hand the client exactly what the API needs.
#[derive(Debug, Clone)]
pub struct DocumentInput {
    pub title: String,
    pub slug: String,
    pub body: String,
    pub tags: Vec<String>,
    /// Optional digital-garden maturity stage (`seedling`|`budding`|`evergreen`).
    /// `None` lets the server keep the column default / existing value.
    pub growth: Option<String>,
}

/// Outcome of a [`InkwellClient::push`]: whether the document was newly created
/// or an existing one was updated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PushAction {
    Created,
    Updated,
}

impl PushAction {
    pub fn label(self) -> &'static str {
        match self {
            PushAction::Created => "Created",
            PushAction::Updated => "Updated",
        }
    }
}

/// The subset of the server document envelope the client reports back.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentSummary {
    pub slug: String,
    pub title: String,
    pub status: String,
    pub version: i64,
}

/// A fuller view of a document for read/list/search surfaces (the MCP tools).
///
/// Includes the Markdown body and tags on top of the summary fields. The
/// `version` is what an MCP client echoes back as the optimistic-concurrency
/// token on the next update.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentDetail {
    pub slug: String,
    pub title: String,
    #[serde(rename = "bodyMarkdown")]
    pub body_markdown: String,
    pub status: String,
    pub tags: Vec<String>,
    pub version: i64,
}

/// The shape of `GET /documents` (the list envelope), narrowed to the fields
/// the MCP `list_notes`/`search_notes` tools surface. `total` lets the client
/// page through the full set rather than stopping at the server's default page.
#[derive(Debug, Clone, Deserialize)]
struct ListEnvelope {
    documents: Vec<DocumentDetail>,
    total: i64,
}

/// A freshly minted scoped token as returned by `POST /admin/tokens`. `token`
/// is the full secret, shown once.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreatedToken {
    pub token: String,
    pub prefix: String,
    pub author: String,
    pub scopes: Vec<String>,
}

/// Token metadata for `token list`. Timestamps are the server's RFC3339 strings
/// (the CLI prints them verbatim), and the secret is never present.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenInfo {
    pub prefix: String,
    pub author_name: String,
    pub scopes: Vec<String>,
    pub created_at: String,
    pub last_used_at: Option<String>,
    pub revoked_at: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct TokenListEnvelope {
    tokens: Vec<TokenInfo>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CreatePayload<'a> {
    title: &'a str,
    slug: &'a str,
    body_markdown: &'a str,
    tags: &'a [String],
    #[serde(skip_serializing_if = "Option::is_none")]
    growth: Option<&'a str>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct UpdatePayload<'a> {
    title: &'a str,
    body_markdown: &'a str,
    tags: &'a [String],
    #[serde(skip_serializing_if = "Option::is_none")]
    growth: Option<&'a str>,
}

#[derive(Deserialize)]
struct ServerError {
    error: ServerErrorBody,
}

#[derive(Deserialize)]
struct ServerErrorBody {
    message: String,
}

/// HTTP client for the authenticated write API. Holds the resolved base URL
/// (no trailing slash) and the API key sent as `X-API-Key`.
pub struct InkwellClient {
    http: reqwest::Client,
    base_url: String,
    api_key: String,
}

impl InkwellClient {
    pub fn new(base_url: impl Into<String>, api_key: impl Into<String>) -> Result<Self> {
        let base_url = base_url.into().trim_end_matches('/').to_string();
        let api_key = api_key.into();
        if api_key.trim().is_empty() {
            bail!("An API key is required. Set INKWELL_API_KEY in the environment or a .env file.");
        }
        if base_url.is_empty() {
            bail!("A server base URL is required.");
        }
        let http = reqwest::Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(REQUEST_TIMEOUT)
            .build()
            .context("building HTTP client")?;
        Ok(Self {
            http,
            base_url,
            api_key,
        })
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base_url, path)
    }

    /// Fetch a document by slug, returning `None` on a 404 so callers can
    /// distinguish "missing" from a transport or server error.
    pub async fn get(&self, slug: &str) -> Result<Option<DocumentSummary>> {
        let resp = self
            .http
            .get(self.url(&format!("/documents/{slug}")))
            .header("x-api-key", &self.api_key)
            .send()
            .await
            .with_context(|| format!("requesting document {slug:?} from {}", self.base_url))?;
        match resp.status() {
            StatusCode::OK => Ok(Some(resp.json().await.context("decoding document")?)),
            StatusCode::NOT_FOUND => Ok(None),
            status => Err(error_for(status, resp).await),
        }
    }

    async fn create(&self, payload: &CreatePayload<'_>) -> Result<DocumentSummary> {
        let resp = self
            .http
            .post(self.url("/documents"))
            .header("x-api-key", &self.api_key)
            .json(payload)
            .send()
            .await
            .with_context(|| format!("creating document at {}", self.base_url))?;
        match resp.status() {
            StatusCode::CREATED => Ok(resp.json().await.context("decoding document")?),
            status => Err(error_for(status, resp).await),
        }
    }

    /// Update a document. When `expected_version` is `Some`, the request carries
    /// an `If-Match` header so the server applies the write only if the stored
    /// version still matches; a `409 Conflict` is mapped to a clear stale-write
    /// error. The author CLI passes `None` to keep its unconditional behaviour.
    async fn update(
        &self,
        slug: &str,
        payload: &UpdatePayload<'_>,
        expected_version: Option<i64>,
    ) -> Result<DocumentSummary> {
        let mut request = self
            .http
            .put(self.url(&format!("/documents/{slug}")))
            .header("x-api-key", &self.api_key)
            .json(payload);
        if let Some(version) = expected_version {
            request = request.header("if-match", version.to_string());
        }
        let resp = request
            .send()
            .await
            .with_context(|| format!("updating document {slug:?} at {}", self.base_url))?;
        match resp.status() {
            StatusCode::OK => Ok(resp.json().await.context("decoding document")?),
            StatusCode::CONFLICT => {
                let detail = conflict_detail(resp).await;
                Err(anyhow!(
                    "Stale write: document {slug:?} changed since you read it (expected version {}).{detail} \
                     Re-read the note and retry with the current version.",
                    expected_version
                        .map(|v| v.to_string())
                        .unwrap_or_else(|| "?".to_string()),
                ))
            }
            status => Err(error_for(status, resp).await),
        }
    }

    pub async fn publish(&self, slug: &str) -> Result<DocumentSummary> {
        self.set_status(slug, "publish").await
    }

    pub async fn unpublish(&self, slug: &str) -> Result<DocumentSummary> {
        self.set_status(slug, "unpublish").await
    }

    async fn set_status(&self, slug: &str, action: &str) -> Result<DocumentSummary> {
        let resp = self
            .http
            .post(self.url(&format!("/documents/{slug}/{action}")))
            .header("x-api-key", &self.api_key)
            .send()
            .await
            .with_context(|| format!("{action}ing document {slug:?} at {}", self.base_url))?;
        match resp.status() {
            StatusCode::OK => Ok(resp.json().await.context("decoding document")?),
            status => Err(error_for(status, resp).await),
        }
    }

    /// Create or update a document. Existence is probed with a `GET`; a hit
    /// becomes a `PUT`, a miss a `POST`. The body size cap is enforced
    /// client-side before anything is sent.
    pub async fn push(&self, doc: &DocumentInput) -> Result<(PushAction, DocumentSummary)> {
        enforce_body_limit(&doc.body)?;
        if self.get(&doc.slug).await?.is_some() {
            let payload = UpdatePayload {
                title: &doc.title,
                body_markdown: &doc.body,
                tags: &doc.tags,
                growth: doc.growth.as_deref(),
            };
            Ok((
                PushAction::Updated,
                self.update(&doc.slug, &payload, None).await?,
            ))
        } else {
            let payload = CreatePayload {
                title: &doc.title,
                slug: &doc.slug,
                body_markdown: &doc.body,
                tags: &doc.tags,
                growth: doc.growth.as_deref(),
            };
            Ok((PushAction::Created, self.create(&payload).await?))
        }
    }

    // -- MCP-facing read/write surface -------------------------------------
    //
    // These return the fuller [`DocumentDetail`] so an AI agent gets the body,
    // tags, and the `version` it needs for optimistic-concurrency updates.

    /// Fetch a document by slug with its body, tags, and version. Returns
    /// `None` on a 404 so callers can report "no such note" cleanly.
    pub async fn read_note(&self, slug: &str) -> Result<Option<DocumentDetail>> {
        let resp = self
            .http
            .get(self.url(&format!("/documents/{slug}")))
            .header("x-api-key", &self.api_key)
            .send()
            .await
            .with_context(|| format!("reading document {slug:?} from {}", self.base_url))?;
        match resp.status() {
            StatusCode::OK => Ok(Some(resp.json().await.context("decoding document")?)),
            StatusCode::NOT_FOUND => Ok(None),
            status => Err(error_for(status, resp).await),
        }
    }

    /// List documents (most-recent first), as the authenticated caller sees
    /// them (drafts included). Pages through every result so `list_notes` and
    /// `search_notes` see the full garden rather than just the server's first
    /// default page.
    pub async fn list_notes(&self) -> Result<Vec<DocumentDetail>> {
        // Request the largest page the server allows to minimise round-trips,
        // then keep paging on `offset` until we've gathered `total` documents.
        const PAGE_SIZE: u32 = 100;
        let mut all = Vec::new();
        let mut offset: u32 = 0;
        loop {
            let resp = self
                .http
                // `limit`/`offset` are plain integers, so no escaping is needed.
                .get(self.url(&format!("/documents?limit={PAGE_SIZE}&offset={offset}")))
                .header("x-api-key", &self.api_key)
                .send()
                .await
                .with_context(|| format!("listing documents at {}", self.base_url))?;
            let envelope: ListEnvelope = match resp.status() {
                StatusCode::OK => resp.json().await.context("decoding document list")?,
                status => return Err(error_for(status, resp).await),
            };
            let page_len = envelope.documents.len();
            all.extend(envelope.documents);
            // Stop once we've collected everything the server reports, or when a
            // page comes back empty (defensive guard against a non-advancing loop).
            if page_len == 0 || (all.len() as i64) >= envelope.total {
                break;
            }
            offset = offset.saturating_add(PAGE_SIZE);
        }
        Ok(all)
    }

    /// Search documents by a free-text query, matching title or body. The
    /// server's `/search` page is HTML, so the MCP search rides the list
    /// endpoint and filters client-side over the same authenticated view.
    pub async fn search_notes(&self, query: &str) -> Result<Vec<DocumentDetail>> {
        let needle = query.trim().to_lowercase();
        let mut notes = self.list_notes().await?;
        if !needle.is_empty() {
            notes.retain(|note| {
                note.title.to_lowercase().contains(&needle)
                    || note.body_markdown.to_lowercase().contains(&needle)
            });
        }
        Ok(notes)
    }

    /// Create a note, returning its summary (slug, status, version).
    pub async fn create_note(&self, doc: &DocumentInput) -> Result<DocumentSummary> {
        enforce_body_limit(&doc.body)?;
        let payload = CreatePayload {
            title: &doc.title,
            slug: &doc.slug,
            body_markdown: &doc.body,
            tags: &doc.tags,
            growth: doc.growth.as_deref(),
        };
        self.create(&payload).await
    }

    /// Update a note conditionally on `expected_version`. A stale token yields a
    /// clear stale-write error (the server returns 409). Only the fields present
    /// in `patch` are changed; `None` fields keep their current server value.
    pub async fn update_note(
        &self,
        slug: &str,
        expected_version: i64,
        title: Option<&str>,
        body: Option<&str>,
        tags: Option<&[String]>,
    ) -> Result<DocumentSummary> {
        // An all-`None` patch would still issue a conditional write, bumping the
        // version for no semantic change and needlessly invalidating readers.
        if title.is_none() && body.is_none() && tags.is_none() {
            bail!("At least one of title, body, or tags must be provided to update a note.");
        }
        // Reading first lets us send a complete `UpdatePayload` (the API treats
        // each field as a full replacement), filling unspecified fields from the
        // current note rather than blanking them.
        let current = self
            .read_note(slug)
            .await?
            .ok_or_else(|| anyhow!("No note with slug {slug:?}."))?;
        if let Some(body) = body {
            enforce_body_limit(body)?;
        }
        let title = title.unwrap_or(&current.title);
        let body = body.unwrap_or(&current.body_markdown);
        let tags = tags.map(<[String]>::to_vec).unwrap_or(current.tags);
        let payload = UpdatePayload {
            title,
            body_markdown: body,
            tags: &tags,
            growth: None,
        };
        self.update(slug, &payload, Some(expected_version)).await
    }

    // -- Admin: scoped-token management ------------------------------------
    //
    // These ride the admin surface (`/admin/tokens`), which requires the
    // configured key to carry the `admin` scope — i.e. the shared
    // `INKWELL_API_KEY`. A non-admin key gets a clear 403.

    /// Mint a scoped token for `name` (created on first use) with `scopes`.
    /// Returns the full secret, shown to the operator exactly once.
    pub async fn create_token(&self, name: &str, scopes: &[String]) -> Result<CreatedToken> {
        let payload = serde_json::json!({ "name": name, "scopes": scopes });
        let resp = self
            .http
            .post(self.url("/admin/tokens"))
            .header("x-api-key", &self.api_key)
            .json(&payload)
            .send()
            .await
            .with_context(|| format!("creating token at {}", self.base_url))?;
        match resp.status() {
            StatusCode::CREATED => Ok(resp.json().await.context("decoding created token")?),
            status => Err(error_for(status, resp).await),
        }
    }

    /// List token metadata (never the secret — it is unrecoverable).
    pub async fn list_tokens(&self) -> Result<Vec<TokenInfo>> {
        let resp = self
            .http
            .get(self.url("/admin/tokens"))
            .header("x-api-key", &self.api_key)
            .send()
            .await
            .with_context(|| format!("listing tokens at {}", self.base_url))?;
        match resp.status() {
            StatusCode::OK => {
                let envelope: TokenListEnvelope =
                    resp.json().await.context("decoding token list")?;
                Ok(envelope.tokens)
            }
            status => Err(error_for(status, resp).await),
        }
    }

    /// Revoke a token by its public `prefix`. A missing/already-revoked token
    /// surfaces as a clear not-found error.
    pub async fn revoke_token(&self, prefix: &str) -> Result<()> {
        let resp = self
            .http
            .post(self.url(&format!("/admin/tokens/{prefix}/revoke")))
            .header("x-api-key", &self.api_key)
            .send()
            .await
            .with_context(|| format!("revoking token {prefix:?} at {}", self.base_url))?;
        match resp.status() {
            StatusCode::OK => Ok(()),
            status => Err(error_for(status, resp).await),
        }
    }
}

/// Extract a human-readable detail from a 409 response body, if the server sent
/// an error envelope; used to enrich the stale-write message.
async fn conflict_detail(resp: reqwest::Response) -> String {
    let raw = resp.text().await.unwrap_or_default();
    match serde_json::from_str::<ServerError>(&raw) {
        Ok(parsed) if !parsed.error.message.is_empty() => format!(" {}", parsed.error.message),
        _ => String::new(),
    }
}

/// Reject document bodies above the server's 256 KiB Markdown cap before
/// sending, so authors get an immediate, actionable error instead of a 413.
fn enforce_body_limit(body: &str) -> Result<()> {
    if body.len() > MAX_BODY_MARKDOWN_LENGTH {
        bail!(
            "Document body is {} bytes, over the {} byte (256 KiB) limit. Trim the content before pushing.",
            body.len(),
            MAX_BODY_MARKDOWN_LENGTH
        );
    }
    Ok(())
}

/// Turn a non-success HTTP response into a clear, non-panicking error message.
async fn error_for(status: StatusCode, resp: reqwest::Response) -> anyhow::Error {
    let raw = resp.text().await.unwrap_or_default();
    let server_message = serde_json::from_str::<ServerError>(&raw)
        .map(|parsed| parsed.error.message)
        .unwrap_or_else(|_| raw.trim().to_string());
    let detail = if server_message.is_empty() {
        String::new()
    } else {
        format!(" {server_message}")
    };
    match status {
        StatusCode::UNAUTHORIZED => {
            anyhow!("Unauthorized (401): the API key was rejected. Check INKWELL_API_KEY.{detail}")
        }
        StatusCode::NOT_FOUND => anyhow!("Not found (404):{detail}"),
        StatusCode::PAYLOAD_TOO_LARGE => {
            anyhow!("Payload too large (413): the document exceeds the server body limit.{detail}")
        }
        StatusCode::UNPROCESSABLE_ENTITY | StatusCode::BAD_REQUEST | StatusCode::CONFLICT => {
            anyhow!("Request rejected ({}):{detail}", status.as_u16())
        }
        other => anyhow!("Unexpected response ({}):{detail}", other.as_u16()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_client_requires_api_key() {
        assert!(InkwellClient::new("http://localhost:3000", "").is_err());
        assert!(InkwellClient::new("http://localhost:3000", "k").is_ok());
    }

    #[test]
    fn enforce_body_limit_rejects_oversize() {
        let body = "a".repeat(MAX_BODY_MARKDOWN_LENGTH + 1);
        assert!(enforce_body_limit(&body).is_err());
        let body = "a".repeat(MAX_BODY_MARKDOWN_LENGTH);
        assert!(enforce_body_limit(&body).is_ok());
    }
}
