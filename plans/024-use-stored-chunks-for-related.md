# Plan 024: Use stored chunks for related-note retrieval

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report. When done, update the status row for this plan in `plans/README.md`.
>
> **Drift check (run first)**: `git diff --stat fef38ad..HEAD -- src/http/ai.rs src/db/chunks.rs tests/ai_contract.rs`
> If any in-scope file changed since this plan was written, compare the
> "Current state" excerpts against the live code before proceeding; on a
> mismatch, treat it as a STOP condition.

## Status

- **Priority**: P1
- **Effort**: M
- **Risk**: MED
- **Depends on**: none
- **Category**: bug/perf
- **Planned at**: commit `fef38ad`, 2026-06-23

## Why this matters

The indexing path carefully chunks notes to at most 1,500 characters before
embedding, but `GET /documents/{slug}/related` ignores that stored chunk index
and embeds the entire document body as one provider input on every request.
With real Voyage configured, a long but valid note can make `/related` slow,
expensive, or fail even though its chunks are already stored in Postgres.

The route should use `note_chunks` as the source of truth: find neighbors by
joining the origin note's stored chunks against other notes' stored chunks, then
return the closest distinct notes.

## Current state

- The chunking contract is bounded:

```rust
// src/ai/mod.rs:31-35
/// Maximum characters per chunk before splitting. Notes are split on blank-line
/// paragraph boundaries and packed up to this size so each chunk is a coherent
/// unit small enough to embed meaningfully. A small const keeps embedding work
/// bounded per note, mirroring every other bounded surface in the garden.
pub const MAX_CHUNK_CHARS: usize = 1_500;
```

- `document_related` re-embeds the whole body instead of using stored chunks:

```rust
// src/http/ai.rs:83-100
let embeddings = state
    .embedder
    .embed(std::slice::from_ref(&document.body_markdown))
    .await
    .map_err(AppError::Internal)?;
let related = match embeddings.first() {
    Some(embedding) => {
        chunks::related_notes(
            &state.pool,
            document.id,
            embedding,
            visibility,
            RELATED_LIMIT,
        )
        .await?
    }
    None => Vec::new(),
};
```

- Stored chunk rows already carry per-note chunk embeddings:

```rust
// src/db/chunks.rs:47-66
/// Replace every chunk of `note_id` with `chunks`, atomically.
pub async fn replace_note_chunks(
    pool: &PgPool,
    note_id: Uuid,
    expected_version: i64,
    chunks: &[NewChunk],
) -> Result<bool, sqlx::Error> {
```

- Existing related-note tests are in `tests/ai_contract.rs`:

```rust
// tests/ai_contract.rs:161-200
#[tokio::test]
async fn related_returns_nearest_published_notes() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(router) = common::maybe_router_with_ai().await? else {
        return Ok(());
    };
```

## Commands you will need

| Purpose | Command | Expected on success |
|---------|---------|---------------------|
| Format | `cargo fmt --check` | exit 0 |
| Lint | `cargo clippy --all-targets --all-features -- -D warnings` | exit 0 |
| Tests | `cargo test --all` | exit 0 |
| DB focused | `DATABASE_URL=<postgres-url> cargo test --test ai_contract related_` | related tests pass |

## Scope

**In scope**:

- `src/db/chunks.rs`
- `src/http/ai.rs`
- `tests/ai_contract.rs`

**Out of scope**:

- Changing the `/documents/{slug}/related` JSON response shape
- Changing `/ask` retrieval
- Adding a reindex command
- Changing provider clients
- Changing chunk size

## Git workflow

- Branch: `advisor/024-use-stored-chunks-for-related`
- Commit message style: `fix(ai): use stored chunks for related notes`
- Do not push or open a PR unless instructed.

## Steps

### Step 1: Add a stored-chunk related query

In `src/db/chunks.rs`, add a function that does not take `query_embedding`.
Suggested signature:

```rust
pub async fn related_notes_for_note(
    pool: &PgPool,
    origin_note_id: Uuid,
    visibility: Visibility,
    limit: i64,
) -> Result<Vec<RelatedNote>, sqlx::Error>
```

The query should:

- read origin chunks from `note_chunks` where `note_id = $1`;
- join candidate chunks from other notes;
- apply the same `Visibility` status filter as current `related_notes`;
- compute the minimum cosine distance between any origin chunk and any candidate
  chunk;
- return one row per candidate note ordered by best distance, then slug;
- exclude the origin note;
- cap by `limit`.

SQL shape to adapt:

```sql
SELECT documents.slug, documents.title,
       min(candidate.embedding <=> origin.embedding) AS distance
FROM note_chunks AS origin
JOIN note_chunks AS candidate ON candidate.note_id <> origin.note_id
JOIN documents ON documents.id = candidate.note_id
WHERE origin.note_id = $1
  -- optional: AND documents.status = $2
GROUP BY documents.id, documents.slug, documents.title
ORDER BY distance ASC, documents.slug ASC
LIMIT $limit
```

Keep the existing `related_notes` function for `/ask` or tests if it is still
used. Do not delete it unless `rg related_notes` proves it has no callers after
the change.

**Verify**: `cargo test --lib db::chunks` may have no direct tests; continue.

### Step 2: Switch the route to stored chunks

In `src/http/ai.rs`, replace the body-embedding block in `document_related` with
a direct call to `chunks::related_notes_for_note(&state.pool, document.id,
visibility, RELATED_LIMIT).await?`.

Do not call `state.embedder.embed` in `document_related`. If a note has no
stored chunks, returning an empty related list is acceptable for this plan.

**Verify**: `rg -n "body_markdown\\)|embed\\(" src/http/ai.rs` should show
that `document_related` no longer embeds `document.body_markdown`.

### Step 3: Add a regression test that related does not call the embedder

In `tests/ai_contract.rs`, add a small test-only embedder that panics or returns
an error if called. Because indexing during create uses the embedder, create the
notes with `common::maybe_router_with_ai()` first, then build a second router
against the same pool/config with the failing embedder for the GET request.

If sharing the pool through existing helpers is awkward, add a helper in
`tests/common/mod.rs` only if it stays test-only and keeps the current helper
style.

Test intent:

- create two published notes and let normal mock indexing write chunks;
- call `/documents/<slug>/related` through a router whose embedder fails if
  used;
- assert `200 OK` and a non-empty related list.

**Verify**: `DATABASE_URL=<postgres-url> cargo test --test ai_contract related_` -> all related tests pass.

## Test plan

- Existing related tests:
  - `related_returns_nearest_published_notes`
  - `related_hides_drafts_from_public_callers`
  - `related_404s_for_unknown_or_draft_slug`
- New regression test proving the route does not call the embedder.
- Full gate:
  - `cargo fmt --check`
  - `cargo clippy --all-targets --all-features -- -D warnings`
  - `cargo test --all`

## Done criteria

- [ ] `document_related` never embeds the full document body.
- [ ] Related-note retrieval is computed from stored `note_chunks`.
- [ ] Public visibility still hides draft neighbors.
- [ ] Unknown or public-invisible origin slugs still return `404`.
- [ ] New test fails on the old implementation and passes on the new one.
- [ ] `cargo fmt --check` exits 0.
- [ ] `cargo clippy --all-targets --all-features -- -D warnings` exits 0.
- [ ] `cargo test --all` exits 0.

## STOP conditions

Stop and report if:

- The live related route no longer matches the excerpt above.
- Stored chunk rows are not available after create/update in DB-backed tests.
- The SQL requires changing `note_chunks` schema.
- The fix appears to require changing `/ask` retrieval or provider traits.

## Maintenance notes

This change makes `/related` depend on the semantic index being present. That is
the right dependency for this route, but reviewers should check empty-index
behavior: an unindexed note should return `200` with `related: []`, not 500.
