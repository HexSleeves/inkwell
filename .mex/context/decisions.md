---
name: decisions
description: Key architectural and technical decisions with reasoning. Load when making design choices or understanding why something is built a certain way.
triggers:
  - "why do we"
  - "why is it"
  - "decision"
  - "alternative"
  - "we chose"
  - "best-effort"
  - "visibility"
  - "optimistic"
edges:
  - target: context/architecture.md
    condition: when a decision relates to system structure
  - target: context/stack.md
    condition: when a decision relates to technology choice
  - target: context/ai.md
    condition: when a decision relates to the AI/RAG layer
  - target: patterns/add-endpoint.md
    condition: when implementing a new endpoint that touches visibility or concurrency decisions
  - target: patterns/debug-request-failures.md
    condition: when a design decision (e.g., best-effort side-effects, visibility) caused a confusing failure
last_updated: 2026-06-23
---

# Decisions

## Decision Log

### Best-effort post-write side-effects (never 500 a successful write)
**Date:** 2026-01-01
**Status:** Active
**Decision:** Edge persistence, embedding indexing, and backlink re-render are all best-effort after a document write: failures warn via `tracing::warn!` but never cause the write to return 500.
**Reasoning:** A note was created/updated successfully — that is the durable fact. Stale links or missing embeddings self-heal on the next save. Failing the write for a side-effect failure would make note creation brittle against transient AI API errors.
**Alternatives considered:** Transactional side-effects (rollback write on failure) — rejected because Voyage API calls can't participate in a Postgres transaction. Queue-based async — rejected as over-engineering for a single-node publishing tool.
**Consequences:** The link graph and embedding index may lag by one save after a provider error. Callers must not assume edges or embeddings are immediately consistent.

### Visibility enum instead of boolean `published_only` flag
**Date:** 2026-01-01
**Status:** Active
**Decision:** `Visibility::Public` / `Visibility::All` controls what content any read operation can see; derived from `is_authenticated` in handlers and threaded into every DB query.
**Reasoning:** A single centralized predicate prevents draft-leak bugs — it's impossible to accidentally show a draft to an anonymous caller if every read path goes through `Visibility`. A boolean flag would be easy to forget or invert.
**Alternatives considered:** Checking `status = 'published'` inline in each handler — rejected because it's repetitive and has no structural guarantee that all paths apply it consistently.
**Consequences:** Every new read endpoint must derive `Visibility` from `is_authenticated` and pass it to the DB layer. See `src/db/links.rs` for `Visibility::status_filter()`.

### Raw SQLx queries, no ORM
**Date:** 2026-01-01
**Status:** Active
**Decision:** All database access uses raw `sqlx::query_as` / `sqlx::query` with explicit SQL strings and column lists.
**Reasoning:** SQLx gives compile-time query checking without the abstraction overhead of an ORM. The schema is stable and hand-crafted; ORM mapping would add complexity without benefit for a single-entity publishing domain.
**Alternatives considered:** Diesel — rejected for complexity of schema migrations workflow. SeaORM — rejected for same reason. Pure `tokio-postgres` — rejected because SQLx's typed query macros are a win for safety.
**Consequences:** Every query must enumerate the column list explicitly (`SELECT id, slug, title, body_markdown, rendered_html, status, growth, tags, version, created_at, updated_at FROM documents ...`). Adding a column to `Document` requires updating every SELECT query.

### Optimistic concurrency via `version` + `If-Match`
**Date:** 2026-01-01
**Status:** Active
**Decision:** Documents carry a monotonic `version` counter; PATCH/PUT accepts an `If-Match` header; the SQL UPDATE guards on `WHERE slug = $1 AND version = $2`; mismatch → 409 Conflict.
**Reasoning:** MCP agents can read and write notes concurrently. Without a concurrency guard, a slow agent with stale data would silently clobber a newer edit. The ETag/If-Match pattern is HTTP-standard and requires zero extra round-trips on the happy path.
**Alternatives considered:** Database-level locks — rejected (holds a connection during AI round-trips). Last-write-wins — rejected (silent data loss for concurrent agents).
**Consequences:** Clients must re-read on 409 and retry. The `update_document_by_slug_if_version` function and `ConditionalUpdate` enum in `src/db/documents.rs` implement this. MCP `update_note` always sends `expected_version`.

### Two separate auth tokens (`INKWELL_API_KEY` + `INKWELL_MCP_KEY`)
**Date:** 2026-01-01
**Status:** Active
**Decision:** Human authoring and MCP agent access use separate bearer tokens, both resolved to an admin `Principal` by `authenticate` for reads and `require_principal` for writes.
**Reasoning:** Allows granting/revoking MCP access independently of the human authoring credential. In production you can rotate the MCP key after an agent breach without locking out the human author.
**Alternatives considered:** Single shared key — simpler but no independent revocation. OAuth — rejected as over-engineering for a personal publishing tool.
**Consequences:** Both keys must be set in Railway env vars. `Config` holds both. `authenticate` accepts either as the bootstrap-admin principal. Superseded-in-part by scoped tokens (below): `INKWELL_MCP_KEY` is retired for a scoped token in slice 4.

### Scoped author tokens, per-author audit, admin token surface (ADR 0009, plan 023, slice 2)
**Date:** 2026-06-23
**Status:** Active
**Decision:** Request auth resolves a `Principal` (`author_id`, `label`, `scopes`) via `authenticate(headers, &Config, &PgPool)`. The shared/MCP keys map to the bootstrap-admin principal; a scoped token `ink_<prefix>_<secret>` is looked up by its public `prefix` then verified by a constant-time SHA-256 hash compare (only the hash is stored). Tokens are minted/listed/revoked over HTTP at `/admin/tokens` (admin-gated), kept on the existing `x-api-key` header, and managed by `inkwell author token …`. Writes are audited against the resolving principal. The audit insert is awaited inline (bounded, non-fatal) so the trail is durable on success.
**Reasoning:** Per-author identity + revocable tokens without sessions/OAuth. Admin-gating the token surface from day one prevents a `write` token minting an `admin` token even though document-route scope/ownership enforcement is deferred to slice 3. Reusing `x-api-key` avoids a transport break for existing clients. A security audit trail must not silently drop rows, so the slice-1 detached `tokio::spawn` insert was changed to an awaited insert.
**Alternatives considered:** Direct-DB token CLI — rejected: operators manage prod (Railway) over HTTP and have no DB access. `Authorization: Bearer` transport — rejected: needless break from the existing `x-api-key`. Storing the raw token — rejected: only the hash is ever persisted. Keeping the detached audit insert — rejected: lost rows under load/shutdown defeat the audit.
**Consequences:** New `src/domain/token.rs`, `src/db/tokens.rs`, `src/http/admin.rs`; `AppError::Forbidden` (403). Mutating handlers take `require_principal(...).await?`; reads/visibility use `authenticate(...).await` (anonymous requests short-circuit with no DB hit). Slice 3 turns on scope/ownership enforcement; slice 4 tightens `owner_id NOT NULL` and retires `INKWELL_MCP_KEY`.

### MCP server as a separate CLI process over stdio
**Date:** 2026-01-01
**Status:** Active
**Decision:** `inkwell mcp` runs as a separate process over stdio (`rmcp::transport::io::stdio()`), delegates to `InkwellClient` (HTTP), and never opens a DB connection.
**Reasoning:** The MCP server is a client of the HTTP API, not a peer. This keeps the server the single gatekeeper for auth, validation, and write ordering. The MCP process can be killed/restarted without affecting the running HTTP server.
**Alternatives considered:** Embedding MCP in the HTTP server on a `/mcp` endpoint — rejected because it would require MCP clients to speak HTTP rather than the standard stdio transport.
**Consequences:** `inkwell mcp` requires a running `inkwell serve` (or Railway deploy) to talk to. Set `INKWELL_API_URL` + `INKWELL_MCP_KEY` before running.

### MockEmbedder + MockLlm for CI/tests
**Date:** 2026-01-01
**Status:** Active
**Decision:** `MockEmbedder` (SHA-256 hash → deterministic 1024-dim vector) and `MockLlm` (deterministic canned answer) are used in tests and when API keys are absent. The real providers are only activated by keys.
**Reasoning:** CI must run without Voyage or Anthropic credentials. The mock embedder is designed to be semantically meaningful (related text is closer than unrelated) so retrieval tests pass without mocking at the query level.
**Alternatives considered:** Mocking HTTP calls (wiremock) — rejected because it tests the HTTP layer, not the RAG retrieval logic. Skipping AI tests — rejected because the retrieval surface is a core feature.
**Consequences:** All tests that exercise `/ask` or related endpoints use `build_router_with_providers` with `MockEmbedder`/`MockLlm`. The deterministic embedding hash depends on SHA-256; changing `mock_embedding` would break existing test fixtures.
