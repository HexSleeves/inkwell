//! Real answer-synthesis provider: Anthropic's Claude (Messages API).
//!
//! Raw HTTP via `reqwest` — Rust has no official Anthropic SDK. Gated behind
//! `ANTHROPIC_API_KEY`, so CI never reaches the network. Single-shot synthesis
//! only: one `POST /v1/messages` with the retrieved context in the system prompt
//! and the question as the sole user message. No streaming, tools, or agents.
//!
//! Per the project's claude-api guidance for `claude-opus-4-8` (the default
//! model): the request carries **no** `temperature`/`top_p`/`top_k`/`budget_tokens`
//! — those 400 on Opus 4.8/4.7 — and a `refusal` stop reason is handled
//! gracefully rather than surfaced as a hard error.

use std::time::Duration;

use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use reqwest::header::{CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue};
use serde::Deserialize;

use super::{Llm, NO_ANSWER_MARKER};

const ANTHROPIC_ENDPOINT: &str = "https://api.anthropic.com/v1/messages";
const ANTHROPIC_VERSION: &str = "2023-06-01";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);
const MAX_TOKENS: u32 = 1024;

/// System prompt: grounds the model in the retrieved context and instructs it to
/// refuse cleanly (with [`NO_ANSWER_MARKER`]) when the context does not support
/// an answer, so the no-answer path never hallucinates.
fn system_prompt() -> String {
    format!(
        "You are the librarian for a personal digital garden of published notes. \
         Answer the user's question using ONLY the provided note excerpts. \
         Cite the notes you draw from by their titles. \
         If the excerpts do not contain enough information to answer, reply \
         exactly with: \"{NO_ANSWER_MARKER}\" and nothing else. Never invent \
         notes, titles, or facts that are not in the excerpts."
    )
}

/// Claude-backed [`Llm`]. Holds a `reqwest::Client` with the auth + version
/// headers baked in (so the key is set once and never logged) and the configured
/// model id.
pub struct ClaudeLlm {
    http: reqwest::Client,
    model: String,
}

#[derive(Deserialize)]
struct MessagesResponse {
    #[serde(default)]
    content: Vec<ContentBlock>,
    #[serde(default)]
    stop_reason: Option<String>,
}

#[derive(Deserialize)]
struct ContentBlock {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    text: Option<String>,
}

impl ClaudeLlm {
    /// Build the client, baking `x-api-key` and `anthropic-version` into the
    /// default headers. The key is marked sensitive so a header-map Debug dump
    /// can never leak it.
    pub fn new(api_key: String, model: String) -> Result<Self> {
        let mut headers = HeaderMap::new();
        let mut key = HeaderValue::from_str(&api_key)
            .context("ANTHROPIC_API_KEY is not a valid HTTP header value")?;
        key.set_sensitive(true);
        headers.insert(HeaderName::from_static("x-api-key"), key);
        headers.insert(
            HeaderName::from_static("anthropic-version"),
            HeaderValue::from_static(ANTHROPIC_VERSION),
        );
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        let http = reqwest::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .default_headers(headers)
            .build()
            .context("building Anthropic HTTP client")?;
        Ok(Self { http, model })
    }
}

#[async_trait]
impl Llm for ClaudeLlm {
    async fn answer(&self, question: &str, context_blocks: &[String]) -> Result<String> {
        if context_blocks.is_empty() {
            // No retrieved context → refuse without spending a request.
            return Ok(NO_ANSWER_MARKER.to_string());
        }
        let context = context_blocks.join("\n\n---\n\n");
        let user_content = format!("Question: {question}\n\nNote excerpts:\n\n{context}");
        // No temperature/top_p/top_k/budget_tokens — they 400 on claude-opus-4-8.
        let body = serde_json::json!({
            "model": self.model,
            "max_tokens": MAX_TOKENS,
            "system": system_prompt(),
            "messages": [{ "role": "user", "content": user_content }],
        });
        let response = self
            .http
            .post(ANTHROPIC_ENDPOINT)
            .json(&body)
            .send()
            .await
            .context("Anthropic messages request failed")?;
        let status = response.status();
        if !status.is_success() {
            let detail = response.text().await.unwrap_or_default();
            bail!("Anthropic messages returned {status}: {detail}");
        }
        let parsed: MessagesResponse = response
            .json()
            .await
            .context("decoding Anthropic messages response")?;
        // Handle a safety refusal gracefully rather than 500ing.
        if parsed.stop_reason.as_deref() == Some("refusal") {
            return Ok(NO_ANSWER_MARKER.to_string());
        }
        // First text block is the answer.
        let answer = parsed
            .content
            .into_iter()
            .find(|block| block.kind == "text")
            .and_then(|block| block.text)
            .map(|text| text.trim().to_string())
            .filter(|text| !text.is_empty());
        match answer {
            Some(text) => Ok(text),
            None => Ok(NO_ANSWER_MARKER.to_string()),
        }
    }
}
