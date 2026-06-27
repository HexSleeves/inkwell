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

**`src/db/chunks.rs`** — ANN query pattern (all three visibility variants follow this pattern):
```rust
ORDER BY distance ASC, slug ASC
LIMIT $6
```
No `WHERE distance < threshold` clause. The `distance` column is computed as `(embedding <=> $1::vector)`.

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
- `src/config.rs` — add `min_similarity: f32` field (default 0.0)
- `src/db/chunks.rs` — add optional `max_distance` parameter to ANN query functions
- `src/http/ai.rs` — pass `max_distance` from `state.config.min_similarity`

**Out of scope**:
- Schema changes — no migration needed
- Changes to the embedding model or dimension
- `/search` endpoint — that uses FTS, not vector ANN

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

### Step 2: Add max_distance to ANN search functions in src/db/chunks.rs

In `src/db/chunks.rs`, find the `search_chunks` and `related_chunks` functions (or their visibility-aware variants). Add an optional `max_distance: Option<f32>` parameter.

In the SQL, add a `HAVING` or subquery filter. The cleanest approach since `distance` is computed in the SELECT:

For queries using a subquery pattern (the current code uses `SELECT … FROM (SELECT … ORDER BY distance) AS nearest ORDER BY distance ASC LIMIT $k`):
```sql
SELECT slug, title, distance FROM (
    SELECT ...
           (note_chunks.embedding <=> $1::vector) AS distance
    FROM note_chunks ...
) AS nearest
WHERE distance <= $max_dist   -- new: filter out low-similarity chunks
ORDER BY distance ASC, slug ASC
LIMIT $k
```

When `max_distance` is `None`, pass `2.0` (the maximum possible cosine distance, effectively no filter) to keep the same bind-parameter structure without branching.

**Verify**: `cargo check --all-targets` → exit 0

### Step 3: Pass threshold from AppState to ANN calls

In `src/http/ai.rs`, find where `search_chunks` or `related_chunks` is called. Add the threshold:
```rust
let max_distance = if state.config.min_similarity > 0.0 {
    Some(1.0 - state.config.min_similarity)  // convert similarity to distance
} else {
    None
};
// ...
chunks::search_chunks(&state.pool, ..., max_distance).await?
```

**Verify**: `cargo check --all-targets` → exit 0

### Step 4: Run tests

**Verify**: `cargo nextest run` → all pass (especially `tests/ai_contract.rs`)

## Test plan

No new tests required for the default (`min_similarity=0.0`) — existing `ai_contract.rs` tests cover the path. Optionally add one test verifying that with a very high threshold (e.g., 0.999), `/ask` returns an empty context (the MockEmbedder produces deterministic embeddings, so you can calculate the expected distance).

## Done criteria

- [ ] `cargo check --all-targets` exits 0
- [ ] `cargo clippy --all-targets -- -D warnings` exits 0
- [ ] `cargo fmt --check` exits 0
- [ ] `cargo nextest run` exits 0
- [ ] `INKWELL_MIN_SIMILARITY` documented in `.env.example`
- [ ] Default value is `0.0` (no behavioural change when not configured)
- [ ] `plans/README.md` status row updated

## STOP conditions

- The ANN query does not use a subquery structure that allows a `WHERE distance <=` clause. Report the actual query shape and stop.
- `sqlx` parameterized queries cannot bind `f32` for the distance filter. Try `f64`; if still failing, report.

## Maintenance notes

- A recommended starting value for `INKWELL_MIN_SIMILARITY` in production is 0.65 (distance ≤ 0.35 filters out clearly unrelated chunks while keeping loose matches). Document this in the deployment guide.
- When switching embedding models (Voyage version bump), re-evaluate this threshold — similarity distributions vary across model versions.
