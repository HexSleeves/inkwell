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
This change touches ~9 files (counted, not estimated) — `render_document_list` is the shared seam but each wrapper view takes a concrete `&[Document]`:
- `src/domain/document.rs` — add `DocumentSummary` struct
- `src/db/documents.rs` — add summary-only query variants for `list_documents`, `list_documents_by_tag`, `list_documents_by_month`, `search_documents`
- `src/http/pages.rs` — switch index/tag/archive handlers to the summary queries
- `src/http/search.rs` — switch the HTML search path to the summary query
- `src/views/layout.rs` — `render_document_list(&[Document])` → `render_document_list(&[DocumentSummary])`
- `src/views/index.rs` — `render_index_page` param `&[Document]` → `&[DocumentSummary]`
- `src/views/tags.rs` — `render_tag_page` param → `&[DocumentSummary]`
- `src/views/search.rs` — `render_search_page` param → `&[DocumentSummary]`
- `src/views/archive.rs` — `render_archive_month_page` param → `&[DocumentSummary]`

**Out of scope** (keep using full `Document`):
- `GET /documents/{slug}` — single-document detail fetch; body is needed
- `PATCH /documents/{slug}`, `PUT /documents/{slug}` — write paths; body is needed
- `src/http/api.rs` `GET /documents` JSON list (`list_documents_vis`) — returns JSON; whether to drop `body` from the JSON response is a separate API/compat decision. **Do NOT touch `list_documents_vis`** (the panel flagged this as a likely mis-target). Leave the JSON list on `Document`.
- `src/db/links.rs`, `src/db/chunks.rs` — those have their own narrow selects

## Git workflow

- Branch: `advisor/032-document-summary`
- Commit: `perf(db): add DocumentSummary type and list queries that omit body fields`

## Steps

### Step 1: Decide what DocumentSummary needs

`render_document_list` (`src/views/layout.rs:488`) is the shared list renderer. It currently calls `derive_excerpt(doc.body_markdown(), 160)` per item — and `derive_excerpt` (`layout.rs:447`) **strips markdown** (```` ``` ````, `` ` ``, `**`, `__`, `*_~`, leading `#`) and collapses whitespace. **The excerpt MUST keep going through `derive_excerpt`** or list pages will show raw markdown — a visible regression the existing `view_layout_contract` test (which only checks the `<p class="excerpt">` element exists) would NOT catch.

So `DocumentSummary` carries a **raw** body slice (not a finished excerpt), and `render_document_list` runs `derive_excerpt` over it exactly as today. Fields:
- `id: Uuid` (used by callers for links/edge queries)
- `slug: String`
- `title: String`
- `body_excerpt_source: String` — `LEFT(body_markdown, 320)` from SQL (first 320 chars; enough for a 160-char stripped excerpt). This is RAW markdown, fed to `derive_excerpt`; it is NOT the final excerpt.
- `tags: Vec<String>`
- `growth: GrowthStage`
- `status: DocumentStatus`
- `created_at: OffsetDateTime`
- `updated_at: OffsetDateTime`

(Omit `version` and `rendered_html` — list views don't use them.)

**Verify**: You have confirmed `render_document_list` uses `derive_excerpt` and the field list above.

### Step 2: Add DocumentSummary to src/domain/document.rs

Add a new struct `DocumentSummary` after the existing `Document` definition:
```rust
#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentSummary {
    pub id: uuid::Uuid,
    pub slug: String,
    pub title: String,
    pub body_excerpt_source: String,  // LEFT(body_markdown, 320) — raw, fed to derive_excerpt
    pub tags: Vec<String>,
    pub growth: GrowthStage,
    pub status: DocumentStatus,
    pub created_at: time::OffsetDateTime,
    pub updated_at: time::OffsetDateTime,
}
```

Match the `sqlx::FromRow` derive pattern used by `Document`. Match the `serde` attributes used on `Document`. Note: `Document` may not derive `sqlx::FromRow` (the repo uses positional `query_as::<Postgres, Document>`); if `FromRow` causes a column-name mismatch with the `LEFT(...) AS body_excerpt_source` alias, drop the derive and map rows manually in Step 3 (see STOP conditions).

**Verify**: `cargo check --all-targets` → exit 0

### Step 3: Add summary query variants to src/db/documents.rs

Add new functions alongside the existing ones (do NOT replace them — callers of the full query still exist):

```rust
pub async fn list_document_summaries(
    pool: &PgPool,
    options: ListOptions,
) -> Result<Vec<DocumentSummary>, sqlx::Error> {
    let mut builder = QueryBuilder::<Postgres>::new(
        "SELECT id, slug, title, LEFT(body_markdown, 320) AS body_excerpt_source, \
         status, growth, tags, created_at, updated_at FROM documents",
    );
    // same WHERE/ORDER/LIMIT/OFFSET logic as list_documents
}
```

Add `_summary` variants for the **in-scope** list functions only: `list_documents`, `list_documents_by_tag`, `list_documents_by_month`, and `search_documents`. **Do NOT add a variant for `list_documents_vis`** (that backs the JSON `GET /documents`, which stays on `Document` — out of scope).

Naming convention: append `_summary` to the existing function name.

**Verify**: `cargo check --all-targets` → exit 0

### Step 4: Switch HTML list handlers + views to summaries

1. In `src/http/pages.rs`: index, tag, and archive handlers call `list_documents`/`list_documents_by_tag`/`list_documents_by_month` → switch to the `_summary` variants.
2. In `src/http/search.rs`: the HTML path calls `search_documents` then `derive_excerpt(doc.body_markdown(), 160)` → switch to `search_documents_summary` and call `derive_excerpt(&summary.body_excerpt_source, 160)`.
3. In `src/views/layout.rs`: change `render_document_list(documents: &[Document])` to `&[DocumentSummary]`, and its per-item call from `derive_excerpt(doc.body_markdown(), 160)` to `derive_excerpt(&doc.body_excerpt_source, 160)`. Everything else in that function (title, slug, date_line, tag chips) reads fields present on `DocumentSummary`.
4. In `src/views/index.rs`, `tags.rs`, `search.rs`, `archive.rs`: change each wrapper's `&[Document]` parameter to `&[DocumentSummary]`.

The detail view (`src/views/document.rs`) keeps using `&Document` — do not change it.

**Verify**: `cargo check --all-targets` → exit 0; `DATABASE_URL=… cargo nextest run` → all pass

## Test plan

This is intended as a behaviour-preserving refactor — BUT the excerpt path is the risk. Add/keep a guard:
- Because `render_document_list` still calls `derive_excerpt` (now over `body_excerpt_source`), list excerpts stay markdown-stripped. Add one assertion to `tests/view_layout_contract.rs`: a doc whose body starts with `**bold** and \`code\`` renders a `<p class="excerpt">` whose text does NOT contain `**` or backticks (proves the strip still runs). This is the regression guard for the excerpt change.
- Existing `tests/api_contract.rs`, `tests/view_layout_contract.rs`, `tests/archive_nav_contract.rs` exercise the affected handlers; if any asserts on `body_markdown`/`rendered_html` in an HTML list response, update it.

## Done criteria

- [ ] `cargo check --all-targets` exits 0
- [ ] `cargo clippy --all-targets -- -D warnings` exits 0
- [ ] `cargo fmt --check` exits 0
- [ ] `DATABASE_URL=… cargo nextest run` exits 0 (DB-backed list tests skip without it)
- [ ] `grep -c "body_markdown, rendered_html" src/db/documents.rs` is below the baseline of **18** (the in-scope list queries no longer select both body columns; detail/write queries still do)
- [ ] Excerpt regression guard test added and passing (`view_layout_contract`): list excerpt is markdown-stripped
- [ ] `list_documents_vis` is unchanged (`git diff` shows the JSON list query untouched)
- [ ] `plans/README.md` status row updated

## STOP conditions

- A list view actually renders `body_html` (full preview). If so, that view keeps `Document`; exclude it and report.
- `sqlx::FromRow` derive fails for `DocumentSummary` (column/alias mismatch). Fall back to `sqlx::query_as::<Postgres, (Uuid, String, String, String, DocumentStatus, GrowthStage, Vec<String>, OffsetDateTime, OffsetDateTime)>` with a manual map into `DocumentSummary`, matching the positional style already used for `Document` in this file.
- The change requires modifying **more than 10** files — report scope creep and stop. (Expected set is 9, enumerated in Scope.)

## Maintenance notes

- When a new list endpoint is added (`GET /documents/recent`, etc.), always use `DocumentSummary`, not `Document`.
- If `excerpt` needs to be longer than 320 chars for some surface (e.g., an API that returns more), add an optional `excerpt_length: Option<usize>` to `ListOptions` defaulting to 320.
- The `body_markdown` and `rendered_html` fields remain on `Document` for all detail and write paths.
