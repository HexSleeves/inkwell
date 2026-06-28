# Plan 037: Extract raw SQLx queries from garden.rs to src/db/

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `plans/README.md`.
>
> **Drift check (run first)**: `git diff --stat 0819727..HEAD -- src/garden.rs src/db/documents.rs src/db/links.rs`
> If any in-scope file changed, compare the "Current state" excerpts before proceeding.

## Status

- **Priority**: P2
- **Effort**: M
- **Risk**: LOW
- **Depends on**: none
- **Category**: tech-debt / architecture
- **Planned at**: commit `0819727`, 2026-06-26

## Why this matters

The architecture rule (documented in `context/conventions.md` verify checklist): "DB access only in `src/db/`, never in handlers or `src/garden.rs`." However, `src/garden.rs` contains 4 raw `sqlx::query_as::<Postgres, …>` calls — a documented exception that was left in place. This exception:
1. Makes `garden.rs` functions hard to unit-test (they require a live DB even for pure rendering logic)
2. Sets a precedent for future violations
3. Means "what queries touch the `documents` table" cannot be answered by reading `src/db/documents.rs` alone

The fix is mechanical: extract the 4 inline queries to functions in `src/db/documents.rs` (or `src/db/links.rs`), then call those functions from `garden.rs`.

## Current state

**`src/garden.rs:198–232`** — 3 of the 4 inline queries live in the private helper `fetch_embed_target`, which looks up an embed target **by slug with visibility branching** (NOT by id). The three arms differ only in the `WHERE`:
```rust
async fn fetch_embed_target(
    pool: &PgPool, slug: &str, visibility: Visibility,
) -> Result<Option<(DocumentStatus, String)>, sqlx::Error> {
    match visibility {
        Visibility::Public => sqlx::query_as::<Postgres, (DocumentStatus, String)>(
            "SELECT status, body_markdown FROM documents WHERE slug = $1 AND status = 'published'",
        ).bind(slug).fetch_optional(pool).await,
        Visibility::Owner(owner_id) => sqlx::query_as::<Postgres, (DocumentStatus, String)>(
            "SELECT status, body_markdown FROM documents WHERE slug = $1 AND (status = 'published' OR owner_id = $2)",
        ).bind(slug).bind(owner_id).fetch_optional(pool).await,
        Visibility::All => sqlx::query_as::<Postgres, (DocumentStatus, String)>(
            "SELECT status, body_markdown FROM documents WHERE slug = $1",
        ).bind(slug).fetch_optional(pool).await,
    }
}
```

**`src/garden.rs:310`** — the 4th inline query, in `rerender_one`, IS id-based:
```rust
let Some((_slug, body_markdown)) = sqlx::query_as::<Postgres, (String, String)>(
    "SELECT slug, body_markdown FROM documents WHERE id = $1",
)
.bind(id)
.fetch_optional(pool)
.await?
```

These queries return narrow projections for the rendering pipeline, not the full `Document` struct.

**Convention for new DB functions** (from `context/conventions.md`):
```
DB query functions: in src/db/<entity>.rs, named <verb>_<entity>_<qualifier>
Examples in the doc: list_documents_by_tag, update_document_by_slug_if_version
```

Functions take `&PgPool` as first arg, never `&AppState`.

## Commands you will need

| Purpose     | Command                                        | Expected on success |
|-------------|------------------------------------------------|---------------------|
| Verify before | `grep -n "sqlx::query_as" src/garden.rs`    | 4 matches           |
| Typecheck   | `cargo check --all-targets`                    | exit 0              |
| Tests       | `cargo nextest run`                            | all pass            |
| Lint        | `cargo clippy --all-targets -- -D warnings`    | exit 0              |

## Scope

**In scope**:
- `src/db/documents.rs` — add new narrow-projection query functions
- `src/garden.rs` — replace inline `sqlx::query_as` calls with calls to the new functions; remove `use sqlx::{PgPool, Postgres}` if no longer needed for direct queries

**Out of scope**:
- `src/http/api.rs`, `src/http/pages.rs` — handlers already correctly use `src/db/` functions; no change
- The `sqlx` import in `garden.rs` for type use (e.g., `sqlx::Error`) — keep if still needed

## Git workflow

- Branch: `advisor/037-garden-extract-queries`
- Commit: `refactor(garden): extract inline DB queries to src/db/documents.rs`

## Steps

### Step 1: Read all 4 query sites in garden.rs

Read `src/garden.rs` in full around lines 200–330. For each of the 4 `sqlx::query_as` calls, record:
- Exact SQL string
- Bind parameters
- Return type (e.g., `(DocumentStatus, String)` or `(String, String)`)
- Purpose (what the query is for)

**Verify**: You have all 4 queries documented.

### Step 2: Add DB functions for the two distinct queries

There are only **two** distinct query shapes (the 3 arms of `fetch_embed_target` are one slug+visibility lookup; line 310 is one id lookup). Add two functions to `src/db/documents.rs`:

1. `get_document_body_by_id(pool: &PgPool, id: Uuid) -> Result<Option<(String, String)>, sqlx::Error>` — returns `(slug, body_markdown)` for a document by ID (replaces the line-310 query in `rerender_one`).

2. `get_embed_target_by_slug(pool: &PgPool, slug: &str, visibility: Visibility) -> Result<Option<(DocumentStatus, String)>, sqlx::Error>` — lift the **entire** `fetch_embed_target` body (all three visibility arms) verbatim into this function. It is a slug + visibility lookup returning `(status, body_markdown)`; do NOT rename it to anything `_rerender`-flavored — it fetches an embed target, not a rerender candidate. `garden.rs::fetch_embed_target` then becomes a thin wrapper (or is deleted and callers call the db fn directly). Note this needs `use crate::db::links::Visibility;` already imported in garden.

Follow the existing function signatures in `src/db/documents.rs`:
```rust
pub async fn get_document_body_by_id(
    pool: &PgPool,
    id: Uuid,
) -> Result<Option<(String, String)>, sqlx::Error> {
    sqlx::query_as::<Postgres, (String, String)>(
        "SELECT slug, body_markdown FROM documents WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
}
```

**Verify**: `cargo check --all-targets` → exit 0

### Step 3: Replace inline calls in garden.rs

For each of the 4 inline query sites in `src/garden.rs`, replace the `sqlx::query_as` block with a call to the new function:

```rust
// Before (garden.rs:310):
let Some((_slug, body_markdown)) = sqlx::query_as::<Postgres, (String, String)>(
    "SELECT slug, body_markdown FROM documents WHERE id = $1",
)
.bind(id)
.fetch_optional(pool)
.await?
else { return Ok(()); };

// After:
let Some((_slug, body_markdown)) = documents::get_document_body_by_id(pool, id).await?
else { return Ok(()); };
```

After all 4 are replaced, check if the `use sqlx::{Postgres}` import in `garden.rs` is still needed. If not, remove it.

**Verify**: `cargo check --all-targets` → exit 0; `grep -n "sqlx::query_as" src/garden.rs` returns zero matches

### Step 4: Update architecture comment in context/conventions.md

In `.mex/context/conventions.md`, the verify checklist item says:
```
- [ ] DB access only in `src/db/`, never in handlers or `src/garden.rs` (except `sqlx::query` in garden's internal helpers)
```

Remove the parenthetical exception:
```
- [ ] DB access only in `src/db/`, never in handlers or `src/garden.rs`
```

Update `last_updated` in the frontmatter.

**Verify**: The exception text is removed.

### Step 5: Run all tests

**Verify**: `cargo nextest run` → all pass

## Test plan

No new tests needed — the extracted functions are tested indirectly through the existing `tests/links_contract.rs` and `tests/api_contract.rs` tests that exercise the write path (create, update, publish) which triggers `garden.rs`.

## Done criteria

- [ ] `grep -n "sqlx::query_as" src/garden.rs` → zero matches
- [ ] New DB functions in `src/db/documents.rs` for each former inline query
- [ ] `cargo check --all-targets` exits 0
- [ ] `cargo clippy --all-targets -- -D warnings` exits 0
- [ ] `cargo fmt --check` exits 0
- [ ] `cargo nextest run` exits 0
- [ ] `context/conventions.md` no longer has the garden.rs exception in the verify checklist
- [ ] `plans/README.md` status row updated

## STOP conditions

- A garden.rs query has no clean DB-layer equivalent (e.g., it does a join not available from existing table helpers). Report the query and stop.
- Removing the `use sqlx::Postgres` import breaks a type reference in garden.rs that is NOT a query (e.g., used in a type annotation). Keep the import if needed; report.

## Maintenance notes

- Going forward, `garden.rs` should import only from `crate::db::*`, `crate::rendering::*`, and `crate::ai::*` — never from `sqlx` directly.
- The verify checklist in `context/conventions.md` now enforces this with no exceptions.
