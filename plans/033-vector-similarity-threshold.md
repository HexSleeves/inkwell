# Plan 033: Add minimum similarity threshold to vector ANN search

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `plans/README.md`.
>
> **Drift check (run first)**: `git diff --stat 0819727..HEAD -- src/db/chunks.rs src/config.rs src/http/ai.rs`
> If any in-scope file changed, compare the "Current state" excerpts before proceeding.

## Status

- **Priority**: P2
- **Effort**: S
- **Risk**: LOW
- **Depends on**: none
- **Category**: perf/AI quality
- **Planned at**: commit `0819727`, 2026-06-26

## Why this matters

The vector ANN search in `src/db/chunks.rs` returns the top-K nearest chunks with no minimum similarity threshold — `ORDER BY embedding <=> $1 LIMIT $k`. When a user query has no good semantic match in the garden (e.g., asking about a topic not covered), low-quality, unrelated chunks are returned and sent to Claude as RAG context. This wastes tokens and degrades `/ask` answer quality. A minimum cosine similarity threshold filters out noise without requiring a schema change — pgvector's `<=>` operator returns distance (0 = identical, 2 = maximally different), so distance ≤ 1 - threshold is the WHERE condition.

## Current state

**The target is `search_chunks`** (`src/db/chunks.rs:374`), which the `/ask` RAG path calls (`src/http/ai.rs:228`). It is a **FLAT** query (NOT a subquery): it computes the distance in the SELECT list and references the alias only in `ORDER BY`. Each of its three visibility arms looks like (Public arm shown):
```sql
SELECT documents.slug, documents.title, note_chunks.content,
       (note_chunks.embedding <=> $1::vector) AS distance
FROM note_chunks
JOIN documents ON documents.id = note_chunks.note_id
WHERE documents.status = 'published'
  AND note_chunks.embedding_provider = $2
  AND note_chunks.embedding_model = $3
ORDER BY distance ASC, documents.slug ASC, note_chunks.chunk_index ASC
LIMIT $4
```
Note the bind numbering differs per arm: `LIMIT` is `$4` (Public/All) but `$5` (Owner, which also binds `owner_id = $2`). **PostgreSQL cannot reference a SELECT-list alias (`distance`) in `WHERE`** — so you cannot simply add `WHERE distance <= $x`. You must either wrap the whole query in an outer subquery and filter on the alias there, or repeat the `(note_chunks.embedding <=> $1::vector) <= $x` expression in the WHERE. This plan uses the **subquery-wrap** approach (Step 2) — it filters the alias once and avoids recomputing the distance.

The `related_notes` (`chunks.rs:140`) and `related_notes_for_note` (`chunks.rs:261`) functions ARE already subquery-shaped (`SELECT … FROM (SELECT … DISTINCT ON …) AS nearest ORDER BY … LIMIT …`); applying a threshold there is a one-line `WHERE distance <= $x` on the outer query. They are an OPTIONAL secondary target — see Scope.

**`src/config.rs`** — already reads many `INKWELL_*` env vars with defaults. Add `INKWELL_MIN_SIMILARITY` here.

**`src/config.rs`** — already reads many `INKWELL_*` env vars with defaults. Add `INKWELL_MIN_SIMILARITY` here.

**Convention**: Config fields added to the `Config` struct in `src/config.rs`; env var name `INKWELL_*`; default chosen to match current behaviour (0.0 = no threshold = current behaviour, or a reasonable starting value like 0.0 that operators can tune).

Note: cosine distance from pgvector `<=>` is in range [0, 2]. A similarity of 0.65 corresponds to distance ≤ 0.35. A similarity of 0.0 means distance ≤ 2.0 (no filter). Setting `INKWELL_MIN_SIMILARITY=0` disables the filter (current behaviour preserved by default).

## Commands you will need

| Purpose   | Command                                        | Expected on success |
|-----------|------------------------------------------------|---------------------|
| Typecheck | `cargo check --all-targets`                    | exit 0              |
| Tests     | `cargo nextest run --test ai_contract`         | all pass            |
| All tests | `cargo nextest run`                            | all pass            |
| Lint      | `cargo clippy --all-targets -- -D warnings`    | exit 0              |

## Scope

**In scope**:
- `src/config.rs` — add `min_similarity: f32` field (default 0.0); also add it to the test config in `tests/common/mod.rs` (the `Config { … }` literal there must list every field — add `min_similarity: 0.0,`)
- `src/db/chunks.rs` — add a `max_distance: Option<f32>` parameter to `search_chunks` (primary). OPTIONALLY also `related_notes` + `related_notes_for_note` (secondary; see below)
- `src/http/ai.rs` — pass `max_distance` from `state.config.min_similarity` to the `search_chunks` call (`ai.rs:228`)

**Out of scope**:
- Schema changes — no migration needed
- Changes to the embedding model or dimension
- `/search` endpoint — that uses FTS, not vector ANN
- The `related_notes` family is OPTIONAL. If you add `max_distance` to them, you MUST also update their caller at `src/http/ai.rs:91` (the `/documents/{slug}/related` route). If you do not want to expand scope, leave them and only thread the threshold through `search_chunks` (the `/ask` path the "Why" section justifies). State which you did.

## Git workflow

- Branch: `advisor/033-vector-similarity-threshold`
- Commit: `feat(ai): add configurable minimum similarity threshold for ANN search`

## Steps

### Step 1: Add min_similarity to Config

In `src/config.rs`, add a new field to the `Config` struct:
```rust
pub min_similarity: f32,
```

In the config loading section, parse it from the env var:
```rust
min_similarity: std::env::var("INKWELL_MIN_SIMILARITY")
    .ok()
    .and_then(|v| v.parse::<f32>().ok())
    .unwrap_or(0.0),
```

Also add to `.env.example`:
```
INKWELL_MIN_SIMILARITY=0  # Minimum cosine similarity for ANN search (0.0=disabled, 0.6-0.8=recommended)
```

**Verify**: `cargo check --all-targets` → exit 0

### Step 2: Add max_distance to `search_chunks` (wrap the flat query)

Add a `max_distance: f32` parameter to `search_chunks` (use a concrete `f32`, not `Option` — pass `2.0` for "no filter" so the bind structure is uniform; `2.0` is the max possible cosine distance, so `distance <= 2.0` matches everything). For EACH of the three visibility arms, wrap the existing flat SELECT in an outer subquery that filters on the alias and append one new bind:

```sql
SELECT slug, title, content, distance FROM (
    SELECT documents.slug, documents.title, note_chunks.content,
           (note_chunks.embedding <=> $1::vector) AS distance
    FROM note_chunks
    JOIN documents ON documents.id = note_chunks.note_id
    WHERE documents.status = 'published'
      AND note_chunks.embedding_provider = $2
      AND note_chunks.embedding_model = $3
) AS scored
WHERE distance <= $4                 -- new threshold bind
ORDER BY distance ASC, slug ASC, /* keep existing tiebreakers */
LIMIT $5                              -- shifted: was $4
```

Key points:
- The threshold bind and the LIMIT bind both shift by one per arm. The Owner arm (which binds `owner_id`) shifts accordingly. Re-number ALL `$N` carefully per arm and add the corresponding `.bind(max_distance)` / `.bind(limit)` in the right order.
- The outer `ORDER BY` can reference the `distance` alias (it is the outer query's column); the inner tiebreakers (`documents.slug`, `note_chunks.chunk_index`) must be carried out as `slug`, and you may need to also select `chunk_index` in the inner query if you want to keep that tiebreaker — simplest is to keep `ORDER BY distance ASC, slug ASC` on the outer query.
- The row tuple type `(String, String, String, f64)` is unchanged (slug, title, content, distance).

**Verify**: `cargo check --all-targets` → exit 0

### Step 3: Pass the threshold from AppState to `search_chunks`

In `src/http/ai.rs` (around line 228 where `search_chunks` is called), compute the max distance from config and pass it:
```rust
// similarity in [0,1] → cosine distance in [0,2]; 0.0 similarity ⇒ 2.0 (no filter)
let max_distance = if state.config.min_similarity > 0.0 {
    1.0 - state.config.min_similarity
} else {
    2.0
};
// ...
chunks::search_chunks(&state.pool, &embedding, visibility, limit, provider, model, max_distance).await?
```
(Match the actual current argument order of `search_chunks`; `max_distance` is the new trailing arg.)

**Verify**: `cargo check --all-targets` → exit 0

### Step 4: Run tests

**Verify**: `cargo nextest run` → all pass (especially `tests/ai_contract.rs`)

## Test plan

No new tests required for the default (`min_similarity=0.0`) — existing `ai_contract.rs` tests cover the path. Optionally add one test verifying that with a very high threshold (e.g., 0.999), `/ask` returns an empty context (the MockEmbedder produces deterministic embeddings, so you can calculate the expected distance).

## Done criteria

- [ ] `cargo check --all-targets` exits 0
- [ ] `cargo clippy --all-targets -- -D warnings` exits 0
- [ ] `cargo fmt --check` exits 0
- [ ] `DATABASE_URL=… cargo nextest run --test ai_contract` exits 0 (DB-backed; skips without `DATABASE_URL`)
- [ ] `INKWELL_MIN_SIMILARITY` documented in `.env.example` and added to the `Config` literal in `tests/common/mod.rs`
- [ ] Default value is `0.0` → passes `max_distance = 2.0` → no behavioural change when not configured
- [ ] `plans/README.md` status row updated

## STOP conditions

- After wrapping `search_chunks` in a subquery, the per-arm bind renumbering does not line up (a `cargo check` parameter-count or type error you cannot resolve in two attempts). Report the exact arm and stop.
- `sqlx` cannot bind `f32` for the distance filter. Use `f64` for the parameter and the `2.0` sentinel; if still failing, report.
- Do NOT treat "search_chunks is a flat query, not a subquery" as a stop — wrapping it in a subquery (Step 2) is the intended fix, not a blocker.

## Maintenance notes

- A recommended starting value for `INKWELL_MIN_SIMILARITY` in production is 0.65 (distance ≤ 0.35 filters out clearly unrelated chunks while keeping loose matches). Document this in the deployment guide.
- When switching embedding models (Voyage version bump), re-evaluate this threshold — similarity distributions vary across model versions.
