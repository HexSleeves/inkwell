# Plan 032: Add DocumentSummary query — stop fetching body on list views

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `plans/README.md`.
>
> **Drift check (run first)**: `git diff --stat 0819727..HEAD -- src/db/documents.rs src/domain/document.rs src/http/pages.rs src/views/`
> If any in-scope file changed, compare the "Current state" excerpts before proceeding.

## Status

- **Priority**: P2
- **Effort**: M
- **Risk**: MED
- **Depends on**: none
- **Category**: perf
- **Planned at**: commit `0819727`, 2026-06-26

## Why this matters

Every list view (index page, tag pages, archive pages, search results) fetches the full `Document` struct including `body_markdown` and `rendered_html`. For a garden with posts averaging 5KB of HTML, a 20-doc page loads ~100KB of body columns over the DB wire that are immediately discarded — only `slug`, `title`, `excerpt`, `tags`, `growth`, `status`, `created_at`, and `updated_at` are used in list rendering.

Note: this finding was previously rejected as plan 009 ("low leverage after Rust rewrite"). That rejection was speculative — the SELECT clause evidence now confirms 15 call sites across `src/db/documents.rs` fetch the full body. The fix is a new `DocumentSummary` type + a parallel set of summary-only queries, keeping the existing `Document` queries for detail views.

## Current state

**`src/db/documents.rs:80–98`** — `list_documents` query (one of 15+ that SELECT the full body):
```rust
pub async fn list_documents(pool: &PgPool, options: ListOptions) -> Result<Vec<Document>, sqlx::Error> {
    let mut builder = QueryBuilder::<Postgres>::new(
        "SELECT id, slug, title, body_markdown, rendered_html, status, growth, tags, version, created_at, updated_at FROM documents",
    );
    // ...
}
```

**`src/domain/document.rs`** — `Document` struct contains `body_markdown: String` and `rendered_html: String`. These are large fields.

**List callers that do NOT need body** (to be confirmed by reading their usage):
- `src/http/pages.rs` — index, tag, archive pages (rendered with `src/views/`)
- `src/http/search.rs` — search results (only needs `slug`, `title`, `tags`, `created_at`, `updated_at`; derives `excerpt` from `body_markdown` — see Step 1)
- `src/http/api.rs` `GET /documents` list endpoint — if callers only get `slug`, `title`, etc., they don't need `body`

**Convention**: new domain types go in `src/domain/document.rs`. New DB query functions go in `src/db/documents.rs`. Follow the naming convention `list_documents_vis_summary` or `list_document_summaries`.

## Commands you will need

| Purpose   | Command                                        | Expected on success |
|-----------|------------------------------------------------|---------------------|
| Typecheck | `cargo check --all-targets`                    | exit 0              |
| Tests     | `cargo nextest run`                            | all pass            |
| Lint      | `cargo clippy --all-targets -- -D warnings`    | exit 0              |
| Fmt       | `cargo fmt --check`                            | exit 0              |

## Scope

**In scope**:
- `src/domain/document.rs` — add `DocumentSummary` struct
- `src/db/documents.rs` — add summary-only query variants
- `src/http/pages.rs` — switch list handlers to use summary queries
- `src/http/search.rs` — switch to summary query (if excerpt can be stored or derived differently)
- `src/views/` — update view functions to accept `&DocumentSummary` instead of `&Document` for list rendering

**Out of scope** (keep using full `Document`):
- `GET /documents/{slug}` — single-document detail fetch; body is needed
- `PATCH /documents/{slug}`, `PUT /documents/{slug}` — write paths; body is needed
- `src/http/api.rs` `GET /documents` list — this returns JSON; whether to add `body` to the JSON response or not is a separate API decision; for now leave it returning `Document` (the JSON API is separate from the HTML path)
- `src/db/links.rs`, `src/db/chunks.rs` — those have their own narrow selects

## Git workflow

- Branch: `advisor/032-document-summary`
- Commit: `perf(db): add DocumentSummary type and list queries that omit body fields`

## Steps

### Step 1: Decide what DocumentSummary needs

Read `src/http/pages.rs` and `src/views/` to determine exactly which fields list-rendering uses. Likely needed:
- `id: Uuid` (for links and edge queries)
- `slug: String`
- `title: String`
- `excerpt: Option<String>` — either stored or derived from `body_markdown`. **Important**: If `derive_excerpt` in `src/views/layout.rs` reads `doc.body_markdown()`, then `DocumentSummary` must either store the first N chars of body (pre-excerpted in SQL) or include a truncated `body_snippet` column. **Recommended**: add `excerpt: String` computed in SQL as `LEFT(body_markdown, 320)` so the DB ships only the first 320 chars instead of the full body.
- `tags: Vec<String>`
- `growth: GrowthStage`
- `status: DocumentStatus`
- `version: i64` (needed for If-Match on list? — probably not; omit if unused in list views)
- `created_at: OffsetDateTime`
- `updated_at: OffsetDateTime`

**Verify**: You have a confirmed field list.

### Step 2: Add DocumentSummary to src/domain/document.rs

Add a new struct `DocumentSummary` after the existing `Document` definition:
```rust
#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentSummary {
    pub id: uuid::Uuid,
    pub slug: String,
    pub title: String,
    pub excerpt: String,   // pre-sliced from body_markdown in SQL
    pub tags: Vec<String>,
    pub growth: GrowthStage,
    pub status: DocumentStatus,
    pub created_at: time::OffsetDateTime,
    pub updated_at: time::OffsetDateTime,
}
```

Match the `sqlx::FromRow` derive pattern used by `Document`. Match the `serde` attributes used on `Document`.

**Verify**: `cargo check --all-targets` → exit 0

### Step 3: Add summary query variants to src/db/documents.rs

Add new functions alongside the existing ones (do NOT replace them — callers of the full query still exist):

```rust
pub async fn list_document_summaries(
    pool: &PgPool,
    options: ListOptions,
) -> Result<Vec<DocumentSummary>, sqlx::Error> {
    let mut builder = QueryBuilder::<Postgres>::new(
        "SELECT id, slug, title, LEFT(body_markdown, 320) AS excerpt, \
         status, growth, tags, created_at, updated_at FROM documents",
    );
    // same WHERE/ORDER/LIMIT/OFFSET logic as list_documents
}
```

Add similar variants for `list_documents_vis` (visibility-aware), `list_documents_by_tag`, `list_documents_by_month` (archive).

Naming convention: append `_summary` to the existing function name.

**Verify**: `cargo check --all-targets` → exit 0

### Step 4: Switch HTML list handlers to summary queries

In `src/http/pages.rs`, replace calls to `list_documents(…)` (and other full-body list functions) with `list_document_summaries(…)` for the index, tag, and archive handlers.

In `src/http/search.rs`, the `derive_excerpt` call currently reads `doc.body_markdown()`. Change search to use a summary query and read `summary.excerpt` instead.

Update view function signatures in `src/views/` as needed to accept `&DocumentSummary` instead of `&Document` for list-rendering functions. The detail view (`src/views/document.rs` or equivalent) keeps using `&Document`.

**Verify**: `cargo check --all-targets` → exit 0; `cargo nextest run` → all pass

## Test plan

No new tests needed — this is a query refactor, not a behaviour change. The existing `tests/api_contract.rs`, `tests/view_layout_contract.rs`, and `tests/archive_nav_contract.rs` exercise the affected handlers.

If any test asserts on `body_markdown` or `rendered_html` in a list response, update it — those fields are intentionally removed from list queries.

## Done criteria

- [ ] `cargo check --all-targets` exits 0
- [ ] `cargo clippy --all-targets -- -D warnings` exits 0
- [ ] `cargo fmt --check` exits 0
- [ ] `cargo nextest run` exits 0
- [ ] `grep -n "body_markdown, rendered_html" src/db/documents.rs | wc -l` count is smaller than the original 15 (remaining occurrences are detail-view queries only)
- [ ] Index page, tag pages, and archive pages load correctly (verified by running `cargo nextest run --test view_layout_contract`)
- [ ] `plans/README.md` status row updated

## STOP conditions

- A list view actually renders `body_html` (e.g., it shows full document preview). If so, that view must keep using `Document`; exclude it from the switch and report.
- `sqlx::FromRow` derive fails for `DocumentSummary` due to the computed `LEFT(…) AS excerpt` column. If so, use `sqlx::query_as` with a manual row mapping.
- The change requires modifying more than 6 files — report scope creep and stop.

## Maintenance notes

- When a new list endpoint is added (`GET /documents/recent`, etc.), always use `DocumentSummary`, not `Document`.
- If `excerpt` needs to be longer than 320 chars for some surface (e.g., an API that returns more), add an optional `excerpt_length: Option<usize>` to `ListOptions` defaulting to 320.
- The `body_markdown` and `rendered_html` fields remain on `Document` for all detail and write paths.
