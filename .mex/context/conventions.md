---
name: conventions
description: How code is written in inkwell — naming, structure, patterns, and style. Load when writing new code or reviewing existing code.
triggers:
  - "convention"
  - "pattern"
  - "naming"
  - "style"
  - "how should I"
  - "what's the right way"
  - "error handling"
edges:
  - target: context/architecture.md
    condition: when a convention depends on understanding the system structure
  - target: context/stack.md
    condition: when a convention is tied to a specific library or crate
  - target: patterns/add-endpoint.md
    condition: when adding a new HTTP handler
  - target: patterns/database-migration.md
    condition: when the convention relates to updating Document structs and SELECT queries
  - target: patterns/debug-request-failures.md
    condition: when a convention was violated and caused a bug
last_updated: 2026-06-28
---

# Conventions

## Naming

- **Files**: snake_case (`documents.rs`, `security_headers.rs`)
- **Modules**: snake_case matching file name; public items re-exported from `mod.rs` sparingly
- **Functions**: snake_case, verb-first (`create_document`, `get_document_by_slug`, `render_and_resolve`)
- **DB query functions**: in `src/db/<entity>.rs`, named `<verb>_<entity>_<qualifier>` (`list_documents_by_tag`, `update_document_by_slug_if_version`)
- **Response types**: `<Entity>Envelope` for individual items, `<Entity>Response` for list wrappers (both private to handler module)
- **Domain enums**: stored as Postgres `text` with `#[sqlx(type_name = "text", rename_all = "lowercase")]`; wire format via `#[serde(rename_all = "lowercase")]`
- **JSON fields**: `camelCase` via `#[serde(rename_all = "camelCase")]` on response structs; incoming JSON fields matched by name (`"bodyMarkdown"`, `"growthStage"`)

## Structure

- **DB layer** (`src/db/`) — only place raw SQLx queries live; one file per entity; functions take `&PgPool` as first arg; never take `&AppState`
- **Handler layer** (`src/http/`) — handlers call db functions and `garden::*`; all response serialization happens here via `Into<Response>`; no business logic beyond validation and orchestration
- **Domain layer** (`src/domain/`) — pure types (`Document`, `NewDocument`, `DocumentPatch`, enums, constants); no async, no DB, no HTTP
- **Garden module** (`src/garden.rs`) — write-path orchestration: the only place that calls `db/`, `rendering/`, and `ai/` together; all side-effects are best-effort with `tracing::warn` on failure
- **Tests**: inline unit tests in `#[cfg(test)]` blocks at the bottom of the relevant file; integration tests in `tests/`; testdata in `testdata/`

## Patterns

**Error propagation**: use `AppError` in handlers (implements `IntoResponse`), `DbError` in `src/db/documents.rs`, `anyhow::Error` everywhere else.

```rust
// Handler: return AppError
pub async fn my_handler(...) -> Result<Response, AppError> {
    let doc = documents::get_document_by_slug(&pool, &slug, filter).await?;
    // DbError auto-converts to AppError via From impl
}

// DB layer: return DbError or sqlx::Error
pub async fn create_document(pool: &PgPool, input: NewDocument) -> Result<Document, DbError> { ... }

// Garden/AI: return anyhow::Result
pub async fn render_and_resolve(pool: &PgPool, markdown: &str) -> Result<(String, Vec<ResolvedRef>), sqlx::Error> { ... }
```

**Visibility guard**: every read that could expose draft content must filter through `Visibility`:
```rust
// Correct: derive visibility from auth, pass to db query (token-aware as of
// scoped-tokens slice 2; resolves the shared key OR a live scoped token).
let visibility = if authenticate(&headers, &state.config, &state.pool).await.is_some() {
    Visibility::All
} else {
    Visibility::Public
};
let filter = StatusFilter { status: visibility.status_filter() };

// Wrong: hardcode status=Published or skip the filter
```

**Best-effort post-write fan-out**: after a successful document write, side-effects (edge persist, embedding index, backlink re-render) are best-effort — log warnings, never 500 a write that succeeded:
```rust
if let Err(error) = garden::persist_source_edges(&state.pool, document.id, &refs).await {
    tracing::warn!(document_id = %document.id, %error, "persist_source_edges failed; ...");
}
```

**Optimistic concurrency on update**: PATCH/PUT accepts an `If-Match` header carrying the expected version; on mismatch return 409 `AppError::Conflict`. `update_document_by_slug_if_version` handles the SQL-level guard. MCP `update_note` always sends `expected_version` via `If-Match`.

## Verify Checklist

Before presenting any code:
- [ ] DB access only in `src/db/`, never in handlers or `src/garden.rs`
- [ ] New handlers return `Result<Response, AppError>`, not `Result<Json<T>, StatusCode>`
- [ ] Write endpoints call `require_principal`, then `require_scope` (create→`write`, publish→`publish`), then pass `owner_filter(&principal)` into the mutating query so ownership is enforced atomically (non-owner → 0 rows → 404; admin → `None` = no constraint) — no separate check-then-write; `create` stamps `owner_id`; audit with that principal
- [ ] Read endpoints derive `Visibility` from `can_see_drafts` (requires the `read` scope; admin implies all; anonymous short-circuits without a DB hit) — not a bare `is_some()`
- [ ] Admin-only surfaces (e.g. token management) additionally require `principal.has(Scope::Admin)` → 403 otherwise
- [ ] Post-write side-effects (edges, embeddings, re-render) are best-effort: `if let Err(e) = ... { tracing::warn!(...) }`
- [ ] New response types derive `Serialize` + use `#[serde(rename_all = "camelCase")]`
- [ ] New domain enums have `#[sqlx(type_name = "text", rename_all = "lowercase")]` and `as_str()` method
- [ ] `cargo fmt --check` + `cargo clippy --all-targets --all-features -- -D warnings` pass
