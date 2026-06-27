---
name: architecture
description: How the major pieces of inkwell connect and flow. Load when working on system design, integrations, or understanding how components interact.
triggers:
  - "architecture"
  - "system design"
  - "how does X connect to Y"
  - "integration"
  - "flow"
  - "garden"
  - "backlinks"
  - "wikilinks"
edges:
  - target: context/stack.md
    condition: when specific technology details (Axum, SQLx, pgvector) are needed
  - target: context/decisions.md
    condition: when understanding why the architecture is structured this way
  - target: patterns/add-endpoint.md
    condition: when adding a new HTTP route or handler
  - target: context/ai.md
    condition: when working on semantic search, RAG, or embeddings
last_updated: 2026-06-26
---

# Architecture

## System Overview

Request enters via Axum router (`src/http/router.rs`) → matched to a handler in `src/http/` → handler reads auth from `Authorization: Bearer` header → authenticated writes call `require_api_key`, reads apply `Visibility` filter → handler calls `db/` functions (raw SQLx queries, no ORM) → for document writes, `garden::render_and_resolve` processes Markdown + wikilinks before the DB insert → post-write fan-out: `garden::persist_source_edges` records outbound links, `ai::index_note` chunks + embeds the body, `garden::backfill_after_change` re-renders inbound linkers → response serialized via `serde_json` as camelCase JSON.

The MCP server (`src/mcp/`) runs as a separate `inkwell mcp` CLI process over stdio, delegates all reads/writes to `InkwellClient` (HTTP), and never touches the database directly. The `inkwell author` CLI follows the same pattern.

## Key Components

- **`src/http/router.rs`** — single `build_router` fn wires all routes to handlers; `AppState` carries `Config`, `PgPool`, `Arc<dyn Embedder>`, `Option<Arc<dyn Llm>>`
- **`src/http/api.rs`** — REST CRUD for `/documents` (GET/POST) and `/documents/{slug}` (GET/PATCH/PUT/DELETE), publish/unpublish, backlinks, graph
- **`src/http/pages.rs`** — HTML rendering for public-facing pages (index, document page, tag pages)
- **`src/http/ai.rs`** — `/ask` RAG endpoint: embed question → vector search `note_chunks` → Claude synthesis
- **`src/garden.rs`** — write-path orchestration: `render_and_resolve` (wikilink extract → slug resolution → Comrak render → embed transclusion), `persist_source_edges`, `backfill_after_change` (re-render fan-out)
- **`src/db/documents.rs`** — raw SQLx queries for documents; `ConditionalUpdate` enum for If-Match optimistic concurrency
- **`src/db/links.rs`** — wikilink graph: `replace_source_edges`, `backlinks`, `garden_graph`, `Visibility` enum (`Public`/`All`)
- **`src/db/chunks.rs`** — `note_chunks` table: vector storage and ANN retrieval for semantic search
- **`src/ai/mod.rs`** — `Embedder` + `Llm` traits; `MockEmbedder` (SHA-256 hash → deterministic vector), `build_embedder`/`build_llm` factory fns; `index_note`/`chunk_text`
- **`src/mcp/mod.rs`** — MCP server via `rmcp` crate; tools: `search_notes`, `read_note`, `list_notes`, `create_note`, `update_note`; uses `If-Match` optimistic concurrency
- **`src/rendering/`** — Comrak markdown pipeline: `wikilink.rs` (extract + render `[[links]]` and `![[embeds]]`), `highlight.rs` (syntect syntax coloring), `sanitize.rs` (Ammonia HTML sanitizer)
- **`src/domain/document.rs`** — core types: `Document`, `NewDocument`, `DocumentPatch`, `DocumentStatus` (Draft/Published), `GrowthStage` (Seedling/Budding/Evergreen)
- **`src/http/rate_limit.rs`** — process-wide GCRA write rate limiter (`governor`); `from_fn` middleware throttling mutations + `/ask`, keyed by credential else client IP; `INKWELL_WRITE_RATE_LIMIT` req/min (CIL-128). Layered inside `security_headers`, outside the handlers.
- **`src/error.rs`** — `AppError` enum; all variants impl `IntoResponse` with structured JSON `{"error":{"message":"..."}}` (incl. `TooManyRequests` → 429 + `Retry-After`)

## External Dependencies

- **PostgreSQL** — sole data store; all access via raw SQLx in `src/db/`; migrations in `migrations/`; pgvector extension required (migration 0009 creates `vector(1024)` column)
- **Voyage AI** (`VOYAGE_API_KEY`) — `voyage-3` model for note embeddings; falls back to `MockEmbedder` when key absent; called once per note write to index chunks
- **Anthropic Claude** (`ANTHROPIC_API_KEY`) — `claude-opus-4-8` default for `/ask` synthesis; `None` when absent (endpoint returns "AI not configured", not 500)
- **Railway** — production deployment; auto-deploys on main push; PostgreSQL with pgvector attached; env vars (`INKWELL_API_KEY`, `INKWELL_SITE_URL`, etc.) set in Railway dashboard

## What Does NOT Exist Here

- No background job queue — all post-write fan-out (re-render, edge persist, embedding) runs inline in the request handler as best-effort; failures warn and never 500 a write that succeeded
- No session management or browser login — auth is the static admin key (`INKWELL_API_KEY`) plus scoped author tokens (`ink_<prefix>_<secret>`, ADR 0009); no OAuth, no registration. (MCP authenticates with a scoped token via `INKWELL_API_KEY`; the old `INKWELL_MCP_KEY` was retired in slice 4.)
- No external file storage — images are uploaded via `POST /media` and stored as `bytea` in the `media` table (migration 0019); `assets/` dir serves a single bundled font via `GET /assets/fonts/nunito.woff2`
- No outbound Webmention sending by default — `INKWELL_WEBMENTION_SEND=true` required to enable; receiving is always on
