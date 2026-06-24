//! Real embedding provider: Voyage AI.
//!
//! Raw HTTP via `reqwest` (there is no official Voyage SDK for Rust). Gated
//! behind `VOYAGE_API_KEY` — only constructed when a key is present, so CI never
//! reaches the network. The key lives only inside the built `reqwest::Client`'s
//! default headers; it is never logged.

use std::time::Duration;

use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue};
use serde::Deserialize;

use super::{EMBEDDING_DIMENSIONS, Embedder, VOYAGE_MODEL};

const VOYAGE_ENDPOINT: &str = "https://api.voyageai.com/v1/embeddings";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);

/// Voyage AI embedder. Targets [`VOYAGE_MODEL`] (1024-dim), matching the
/// `vector(1024)` column.
pub struct VoyageEmbedder {
    http: reqwest::Client,
    model: String,
}

#[derive(Deserialize)]
struct EmbeddingResponse {
    data: Vec<EmbeddingDatum>,
}

#[derive(Deserialize)]
struct EmbeddingDatum {
    embedding: Vec<f32>,
}

impl VoyageEmbedder {
    /// Build the embedder, baking the bearer token into the client's default
    /// headers so it is set once and never threaded through call sites or logs.
    pub fn new(api_key: String) -> Result<Self> {
        let mut headers = HeaderMap::new();
        let mut auth = HeaderValue::from_str(&format!("Bearer {api_key}"))
            .context("VOYAGE_API_KEY is not a valid HTTP header value")?;
        // Never let the credential surface in a header-map Debug dump.
        auth.set_sensitive(true);
        headers.insert(AUTHORIZATION, auth);
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        let http = reqwest::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .default_headers(headers)
            .build()
            .context("building Voyage HTTP client")?;
        Ok(Self {
            http,
            model: VOYAGE_MODEL.to_string(),
        })
    }
}

#[async_trait]
impl Embedder for VoyageEmbedder {
    fn provider(&self) -> &'static str {
        "voyage"
    }

    fn model(&self) -> &str {
        &self.model
    }

    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        let body = serde_json::json!({ "input": texts, "model": self.model });
        let response = self
            .http
            .post(VOYAGE_ENDPOINT)
            .json(&body)
            .send()
            .await
            .context("Voyage embeddings request failed")?;
        let status = response.status();
        if !status.is_success() {
            // Surface the status (not the key) so a misconfig is debuggable.
            let detail = response.text().await.unwrap_or_default();
            bail!("Voyage embeddings returned {status}: {detail}");
        }
        let parsed: EmbeddingResponse = response
            .json()
            .await
            .context("decoding Voyage embeddings response")?;
        let mut out = Vec::with_capacity(parsed.data.len());
        for datum in parsed.data {
            if datum.embedding.len() != EMBEDDING_DIMENSIONS {
                bail!(
                    "Voyage returned a {}-dim embedding; expected {EMBEDDING_DIMENSIONS}",
                    datum.embedding.len()
                );
            }
            out.push(datum.embedding);
        }
        if out.len() != texts.len() {
            bail!(
                "Voyage returned {} embeddings for {} inputs",
                out.len(),
                texts.len()
            );
        }
        Ok(out)
    }
}
