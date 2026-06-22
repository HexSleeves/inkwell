//! Shared HTTP client for the authenticated write API.
//!
//! [`InkwellClient`] is a thin [`reqwest`] wrapper that speaks the same write
//! API any other client would. It owns the transport concerns — the resolved
//! base URL, the `X-API-Key` header, status-code handling, the client-side
//! body cap, and the server error envelope — so callers like the `inkwell
//! author` CLI can stay focused on authoring policy.

use anyhow::{Context, Result, anyhow, bail};
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};

use crate::domain::document::MAX_BODY_MARKDOWN_LENGTH;

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
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CreatePayload<'a> {
    title: &'a str,
    slug: &'a str,
    body_markdown: &'a str,
    tags: &'a [String],
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct UpdatePayload<'a> {
    title: &'a str,
    body_markdown: &'a str,
    tags: &'a [String],
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

    async fn update(&self, slug: &str, payload: &UpdatePayload<'_>) -> Result<DocumentSummary> {
        let resp = self
            .http
            .put(self.url(&format!("/documents/{slug}")))
            .header("x-api-key", &self.api_key)
            .json(payload)
            .send()
            .await
            .with_context(|| format!("updating document {slug:?} at {}", self.base_url))?;
        match resp.status() {
            StatusCode::OK => Ok(resp.json().await.context("decoding document")?),
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
            };
            Ok((PushAction::Updated, self.update(&doc.slug, &payload).await?))
        } else {
            let payload = CreatePayload {
                title: &doc.title,
                slug: &doc.slug,
                body_markdown: &doc.body,
                tags: &doc.tags,
            };
            Ok((PushAction::Created, self.create(&payload).await?))
        }
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
