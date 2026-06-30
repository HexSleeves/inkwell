//! Model Context Protocol (MCP) server over stdio (card T6).
//!
//! This exposes a small, sharp set of tools so an AI agent can read, search,
//! and write notes in a **live** inkwell garden. It is a thin HTTP client: every
//! tool delegates to the shared [`InkwellClient`](crate::client::InkwellClient),
//! which talks to a running inkwell HTTP server. The MCP server never touches
//! the database directly.
//!
//! Authentication: the client is built with `INKWELL_API_KEY`, which operators
//! set to a **scoped token** (`inkwell author token create`) so MCP access is
//! granted and revoked independently of the admin shared key. Updates carry the
//! note `version` as `If-Match`, so a stale write surfaces as a clear error
//! instead of clobbering newer content. (The separate `INKWELL_MCP_KEY` was
//! retired in slice 4, ADR 0009.)

use std::sync::Arc;

use anyhow::Result;
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ContentBlock, ErrorData};
use rmcp::{ServerHandler, ServiceExt, tool, tool_handler, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;

use crate::client::{DocumentInput, InkwellClient};

/// The MCP server. Holds the shared HTTP client and the generated tool router.
#[derive(Clone)]
pub struct InkwellMcpServer {
    client: Arc<InkwellClient>,
    tool_router: ToolRouter<Self>,
}

// -- Tool argument types -----------------------------------------------------
// Each derives `JsonSchema` so the `#[tool]` macro can advertise an input
// schema to the agent, and `Deserialize` so rmcp can parse the call arguments.

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SearchNotesArgs {
    /// Free-text query matched against each note's title and Markdown body
    /// (case-insensitive substring).
    pub query: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ReadNoteArgs {
    /// The note's slug, e.g. `my-first-post`.
    pub slug: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CreateNoteArgs {
    /// Human-readable title for the note.
    pub title: String,
    /// The note body in Markdown. Wikilinks (`[[other-note]]`) are supported.
    pub body: String,
    /// Optional explicit slug (lowercase alphanumerics and single hyphens). When
    /// omitted, the server derives one from the title.
    #[serde(default)]
    pub slug: Option<String>,
    /// Optional tags to attach to the note.
    #[serde(default)]
    pub tags: Option<Vec<String>>,
    /// Optional digital-garden maturity stage: `seedling`, `budding`, or
    /// `evergreen`. When omitted, the server keeps its default (`seedling`).
    #[serde(default)]
    pub growth: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct UpdateNoteArgs {
    /// The slug of the note to update.
    pub slug: String,
    /// The version you last read (from `read_note`). The update is rejected with
    /// a stale-write error if the note changed since then, so you never clobber
    /// a newer edit. Re-read and retry with the current version on conflict.
    pub expected_version: i64,
    /// New Markdown body. Omit to leave the body unchanged.
    #[serde(default)]
    pub body: Option<String>,
    /// New title. Omit to leave the title unchanged.
    #[serde(default)]
    pub title: Option<String>,
    /// New complete tag set (replaces the existing tags). Omit to leave tags
    /// unchanged.
    #[serde(default)]
    pub tags: Option<Vec<String>>,
}

#[tool_router]
impl InkwellMcpServer {
    pub fn new(client: InkwellClient) -> Self {
        Self {
            client: Arc::new(client),
            tool_router: Self::tool_router(),
        }
    }

    /// Search notes by a free-text query over title and body. Returns the
    /// matching notes (slug, title, status, tags, version).
    #[tool(
        description = "Search notes in the garden by a free-text query (matches title and body). Returns matching notes with slug, title, status, tags, and version."
    )]
    pub async fn search_notes(
        &self,
        Parameters(args): Parameters<SearchNotesArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let notes = self
            .client
            .search_notes(&args.query)
            .await
            .map_err(to_mcp_error)?;
        ok_json(&serde_json::json!({ "notes": notes, "count": notes.len() }))
    }

    /// Read a single note by slug, returning its full content and the version
    /// to pass back to `update_note`.
    #[tool(
        description = "Read a single note by its slug. Returns title, body (Markdown), status, tags, and the version to echo back to update_note. Returns found=false when no such note exists."
    )]
    pub async fn read_note(
        &self,
        Parameters(args): Parameters<ReadNoteArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        match self
            .client
            .read_note(&args.slug)
            .await
            .map_err(to_mcp_error)?
        {
            Some(note) => ok_json(&serde_json::json!({ "found": true, "note": note })),
            None => ok_json(&serde_json::json!({ "found": false, "slug": args.slug })),
        }
    }

    /// List all notes (most recent first) as the configured key can see them.
    #[tool(
        description = "List notes in the garden, most recent first. Returns each note's slug, title, status, tags, and version."
    )]
    pub async fn list_notes(&self) -> Result<CallToolResult, ErrorData> {
        let notes = self.client.list_notes().await.map_err(to_mcp_error)?;
        ok_json(&serde_json::json!({ "notes": notes, "count": notes.len() }))
    }

    /// Create a new note. Returns the created note's slug, status, and version.
    #[tool(
        description = "Create a new note from a title and Markdown body. Optionally provide an explicit slug, tags, and a growth stage (seedling, budding, or evergreen). Returns the new note's slug, status, and version."
    )]
    pub async fn create_note(
        &self,
        Parameters(args): Parameters<CreateNoteArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let slug = match args
            .slug
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            Some(slug) => slug.to_string(),
            // Mirror the server's title-derived slug so the client can send a
            // concrete slug; the server still validates and may reject it.
            None => crate::domain::slug::slugify(&args.title),
        };
        let input = DocumentInput {
            title: args.title,
            slug,
            body: args.body,
            tags: args.tags.unwrap_or_default(),
            growth: args.growth,
        };
        let summary = self
            .client
            .create_note(&input)
            .await
            .map_err(to_mcp_error)?;
        ok_json(&serde_json::json!({
            "slug": summary.slug,
            "title": summary.title,
            "status": summary.status,
            "version": summary.version,
        }))
    }

    /// Update an existing note, conditional on `expected_version` (If-Match). A
    /// stale version is rejected so you never overwrite a newer edit.
    #[tool(
        description = "Update an existing note. Requires expected_version (from read_note) for optimistic concurrency: if the note changed since you read it, the update is rejected with a stale-write error — re-read and retry. Returns the updated note's slug, status, and new version."
    )]
    pub async fn update_note(
        &self,
        Parameters(args): Parameters<UpdateNoteArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let summary = self
            .client
            .update_note(
                &args.slug,
                args.expected_version,
                args.title.as_deref(),
                args.body.as_deref(),
                args.tags.as_deref(),
            )
            .await
            .map_err(to_mcp_error)?;
        ok_json(&serde_json::json!({
            "slug": summary.slug,
            "title": summary.title,
            "status": summary.status,
            "version": summary.version,
        }))
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for InkwellMcpServer {}

/// Serialize a value into a single structured-JSON tool result.
fn ok_json<T: serde::Serialize>(value: &T) -> Result<CallToolResult, ErrorData> {
    let content = ContentBlock::json(value)?;
    Ok(CallToolResult::success(vec![content]))
}

/// Map a client/transport error into an MCP internal error. The message is
/// already actionable (the client crafts clear errors, including stale-write).
fn to_mcp_error(error: anyhow::Error) -> ErrorData {
    ErrorData::internal_error(error.to_string(), None)
}

/// Build an [`InkwellMcpServer`] and serve it over stdio until the peer
/// disconnects. Reads the API base URL and `INKWELL_API_KEY` (a scoped token)
/// from the environment via [`AuthorConfig`](crate::config::AuthorConfig).
pub async fn run_stdio(base_url: String, api_key: String) -> Result<()> {
    let client = InkwellClient::new(base_url, api_key)?;
    let server = InkwellMcpServer::new(client);
    // `stdio()` yields `(Stdin, Stdout)`, which satisfies `IntoTransport` as an
    // `(AsyncRead, AsyncWrite)` pair.
    let running = server.serve(rmcp::transport::io::stdio()).await?;
    running.waiting().await?;
    Ok(())
}
