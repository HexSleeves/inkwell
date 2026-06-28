# Plan 038: Deduplicate Visibility SQL predicate across db/ modules

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `plans/README.md`.
>
> **Drift check (run first)**: `git diff --stat 0819727..HEAD -- src/db/documents.rs src/db/links.rs src/db/chunks.rs`
> If any in-scope file changed, compare the "Current state" excerpts before proceeding.

## Status

- **Priority**: P2
- **Effort**: M
- **Risk**: MED
- **Depends on**: none
- **Category**: tech-debt
- **Planned at**: commit `0819727`, 2026-06-26

## Why this matters

The `Visibility` enum (`Public` / `Owner(Uuid)` / `All`, defined at `src/db/links.rs:41-49`) drives a `match` block that produces a different SQL `WHERE` fragment across many sites in `src/db/documents.rs`, `src/db/links.rs`, and `src/db/chunks.rs`. Every site repeats the same three-arm match. Adding a new visibility mode requires updating each independent SQL branch, and missing one creates a security gap.

**Scope reality (read before planning):** the sites split into two kinds, and only one kind is cleanly dedupable:
- **`QueryBuilder`-based sites** (in `src/db/documents.rs`) build the WHERE incrementally and CAN use a `push_where` helper. These are the in-scope target.
- **Raw `sqlx::query_as`/`query_scalar` sites with hand-numbered positional params** (ALL of `src/db/chunks.rs`, plus a couple in `documents.rs` and `links.rs`) bind `$1..$N` by position, and the numbering SHIFTS between arms (e.g. in `chunks.rs` the provider bind is `$3` for Public/All but `$4` for Owner). A `QueryBuilder` helper cannot be applied to these without first rewriting them as `QueryBuilder` queries — out of scope here. This plan therefore targets the `QueryBuilder` sites only and explicitly leaves the raw-positional sites as a documented follow-up.

## Current state

**`src/db/links.rs:135–162`** — typical visibility match pattern (repeated many times):
```rust
let found: Vec<String> = match visibility {
    Visibility::Public => {
        sqlx::query_scalar::<Postgres, String>(
            "SELECT slug FROM documents WHERE slug = ANY($1) AND status = 'published'",
        )
        .bind(slugs).fetch_all(pool).await?
    }
    Visibility::Owner(owner_id) => {
        sqlx::query_scalar::<Postgres, String>(
            "SELECT slug FROM documents WHERE slug = ANY($1) \
             AND (status = 'published' OR owner_id = $2)",
        )
        .bind(slugs).bind(owner_id).fetch_all(pool).await?
    }
    Visibility::All => {
        sqlx::query_scalar::<Postgres, String>(
            "SELECT slug FROM documents WHERE slug = ANY($1)",
        )
        .bind(slugs).fetch_all(pool).await?
    }
};
```

The pattern: three separate query paths, each with an inline SQL string. For `QueryBuilder`-based queries the same three-way split builds a `WHERE` clause fragment.

**Where `Visibility` is defined**: `src/db/links.rs` (where the enum lives — confirm with `grep -n "enum Visibility" src/db/links.rs`).

## Commands you will need

| Purpose     | Command                                        | Expected on success |
|-------------|------------------------------------------------|---------------------|
| Count before | `grep -c "Visibility::Public\|Visibility::Owner\|Visibility::All" src/db/documents.rs` | baseline |
| Typecheck   | `cargo check --all-targets`                    | exit 0              |
| Tests       | `cargo nextest run`                            | all pass            |
| Lint        | `cargo clippy --all-targets -- -D warnings`    | exit 0              |

## Scope

**In scope**:
- `src/db/links.rs` — add the `push_where` helper method to `Visibility`
- `src/db/documents.rs` — use the helper in the 4 `QueryBuilder`-based queries (the WHERE-fragment three-arm matches at approximately lines 585, 622, 706, 744)

**Out of scope** (raw positional-param queries — the helper does not apply):
- `src/db/chunks.rs` — ALL visibility matches here are raw `sqlx::query_as` with hand-numbered `$1..$N` binds whose numbering shifts between arms; there are ZERO `QueryBuilder` blocks (`grep "QueryBuilder" src/db/chunks.rs` returns nothing). Converting them to `QueryBuilder` first is a larger, separate effort — leave them and note as a follow-up.
- The two raw `query_as`/`query_scalar` sites in `documents.rs` (the slug-alias lookup ~line 377 and `get_document_by_slug_vis` ~line 652) — also positional-param, not QueryBuilder. Leave them.
- The `resolve_slug_ids`/`resolve_existing_slugs` matches in `links.rs` — positional-param. Leave them.
- `src/http/` callers; `src/domain/` — no change.

## Git workflow

- Branch: `advisor/038-visibility-predicate-dedup`
- Commit: `refactor(db): add Visibility::push_where helper, dedup documents.rs predicates`

## Steps

### Step 1: Add a `push_where` helper to Visibility

`Visibility::Owner` holds a `Uuid` (not `i64`). The helper pushes the visibility predicate onto a `QueryBuilder`, binding the `Uuid` via `push_bind` (which handles positional numbering automatically — no `$N` bookkeeping). In `src/db/links.rs`:

```rust
impl Visibility {
    /// Push a visibility WHERE predicate onto a QueryBuilder.
    /// Call right after the builder has pushed `WHERE ` or `... AND `.
    /// Emits UNQUALIFIED column names (`status`, `owner_id`); only use at call
    /// sites where `documents` is the sole table providing those columns.
    pub fn push_where(&self, qb: &mut QueryBuilder<'_, Postgres>) {
        match self {
            Visibility::Public => { qb.push("status = 'published'"); }
            Visibility::Owner(id) => {
                qb.push("(status = 'published' OR owner_id = ").push_bind(*id).push(")");
            }
            Visibility::All => { qb.push("TRUE"); }
        }
    }
}
```

(Do NOT add a `status_sql_fragment(&self) -> (&str, Option<i64>)` variant — `Owner` is a `Uuid`, and the sentinel-placeholder approach is brittle. `push_where` is the only helper this plan adds.)

Add a `#[cfg(test)]` unit test exercising each arm if a `QueryBuilder` can be cheaply constructed in a test; otherwise rely on the integration tests in Step 4.

**Verify**: `cargo check --all-targets` → exit 0

### Step 2: Replace the three-way matches in the documents.rs QueryBuilder queries

In `src/db/documents.rs` only, find the `QueryBuilder` queries whose visibility handling is a three-arm `match` differing only in the WHERE predicate (≈ lines 585, 622, 706, 744). Replace each with a single `QueryBuilder` block using `visibility.push_where(&mut builder)`:
```rust
let mut builder = QueryBuilder::<Postgres>::new("SELECT ... FROM documents WHERE ");
visibility.push_where(&mut builder);
builder.push(" AND ...other conditions...");
// rest of query
```

Do NOT touch `chunks.rs` or the raw positional-param sites (see Out of scope) — they are not `QueryBuilder`-based and `push_where` cannot apply. Run `cargo check` after each conversion.

**Verify after each conversion**: `cargo check --all-targets` → exit 0

### Step 3: Leave non-reducible matches as-is

Some queries may have visibility-dependent SQL that is more than just the WHERE predicate (e.g., different JOIN conditions or entirely different query shapes). Do NOT force these into `push_where` — leave them as three-arm matches and add a comment: `// Visibility variants differ in structure, not just the predicate; intentional match`.

**Verify**: You have not changed any query whose correctness depends on the variant-specific SQL structure.

### Step 4: Run all tests

**Verify**: `cargo nextest run` → all pass

## Test plan

No new tests — this is a refactor. Existing integration tests across `tests/scoped_tokens_slice3b.rs`, `tests/links_contract.rs`, `tests/ai_contract.rs` exercise visibility filtering and will catch regressions.

## Done criteria

- [ ] `Visibility::push_where` defined in `src/db/links.rs` (binds the `Uuid` owner via `push_bind`)
- [ ] `grep -c "Visibility::Public =>" src/db/documents.rs` reduced from 6 (baseline) to ≤ 3 (the 4 QueryBuilder sites converted; the 2 raw positional-param sites remain by design)
- [ ] `src/db/chunks.rs` is unchanged (out of scope) — `git diff --stat` shows no chunks.rs change
- [ ] `cargo check --all-targets` exits 0
- [ ] `cargo clippy --all-targets -- -D warnings` exits 0
- [ ] `cargo fmt --check` exits 0
- [ ] `DATABASE_URL=… cargo nextest run` exits 0 (visibility tests are DB-backed; set `DATABASE_URL` or they skip — see `tests/common/mod.rs`)
- [ ] `plans/README.md` status row updated

## STOP conditions

- A visibility match cannot be reduced because the three arms use entirely different SQL (not just different WHERE predicates). Document each such match with a comment and stop for that one; proceed with the reducible ones.
- `QueryBuilder` does not support the push_bind pattern for i64 as expected. Report the specific error.

## Maintenance notes

- When adding a new `Visibility` variant in the future, add it to `push_where` first — the compiler will then point at every remaining three-arm match that needs updating.
- The `push_where` helper is the single source of truth for visibility SQL semantics. Its unit tests (add at least one per variant in `#[cfg(test)]` at the bottom of `links.rs`) verify it in isolation.
