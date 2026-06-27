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

The `Visibility` enum (`Public` / `Owner(i64)` / `All`) drives a `match` block that produces a different SQL `WHERE` fragment across 16+ sites in `src/db/documents.rs`, `src/db/links.rs`, and `src/db/chunks.rs`. Every site repeats the same three-arm match. Adding a new visibility mode (e.g., team-level access) requires updating 16+ independent SQL branches with no single point of change, and missing one site creates a security gap.

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
- `src/db/links.rs` — add helper method to `Visibility` enum
- `src/db/documents.rs` — use the helper in `QueryBuilder`-based queries
- `src/db/chunks.rs` — use the helper in ANN queries

**Out of scope**:
- `src/http/` — callers of db functions; no change
- `src/domain/` — the `Visibility` type is in `db/links.rs` per current code
- Queries that use the three-way match and cannot be restructured (e.g., those that must use entirely different SQL per variant) — leave those as-is; document them as intentional exceptions

## Git workflow

- Branch: `advisor/038-visibility-predicate-dedup`
- Commit: `refactor(db): add Visibility::status_sql_fragment helper, deduplicate predicates`

## Steps

### Step 1: Add a SQL fragment helper to Visibility

In `src/db/links.rs`, add a method to the `Visibility` enum that returns the SQL `WHERE` fragment and the optional owner bind value:

```rust
impl Visibility {
    /// Returns the SQL WHERE fragment for filtering by visibility, and
    /// the optional owner_id to bind when the variant is Owner(id).
    ///
    /// Callers append this to their WHERE clause and bind the optional
    /// owner_id as the next positional parameter.
    pub fn status_sql_fragment(&self) -> (&'static str, Option<i64>) {
        match self {
            Visibility::Public => ("status = 'published'", None),
            Visibility::Owner(id) => ("(status = 'published' OR owner_id = $__owner__)", Some(*id)),
            Visibility::All => ("TRUE", None),
        }
    }
}
```

NOTE: The `$__owner__` placeholder is a sentinel — callers replace it with the actual positional parameter number (e.g., `$2`, `$3`). Alternatively, use `QueryBuilder` push_bind API which handles positional numbering automatically. See Step 2 for the preferred approach.

**Preferred approach** (avoids positional parameter complexity): For `QueryBuilder`-based queries, add a helper that pushes the predicate directly:

```rust
impl Visibility {
    /// Push a visibility WHERE predicate onto a QueryBuilder.
    /// Call after `WHERE ` or `AND ` has been pushed.
    pub fn push_where<'qb>(&self, qb: &mut QueryBuilder<'qb, Postgres>) {
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

This approach integrates naturally with `QueryBuilder` and handles positional parameters automatically.

**Verify**: `cargo check --all-targets` → exit 0

### Step 2: Replace three-way matches in QueryBuilder queries

In `src/db/documents.rs` and `src/db/chunks.rs`, find every `match visibility { Visibility::Public => { … QueryBuilder … }, Visibility::Owner => { … }, Visibility::All => { … } }` block where the three arms differ only in the WHERE predicate.

Replace with a single `QueryBuilder` block using `visibility.push_where(&mut builder)`:
```rust
let mut builder = QueryBuilder::<Postgres>::new("SELECT ... FROM documents WHERE ");
visibility.push_where(&mut builder);
builder.push(" AND ...other conditions...");
// rest of query
```

Work file by file. Run `cargo check` after each file.

**Verify after each file**: `cargo check --all-targets` → exit 0

### Step 3: Leave non-reducible matches as-is

Some queries may have visibility-dependent SQL that is more than just the WHERE predicate (e.g., different JOIN conditions or entirely different query shapes). Do NOT force these into `push_where` — leave them as three-arm matches and add a comment: `// Visibility variants differ in structure, not just the predicate; intentional match`.

**Verify**: You have not changed any query whose correctness depends on the variant-specific SQL structure.

### Step 4: Run all tests

**Verify**: `cargo nextest run` → all pass

## Test plan

No new tests — this is a refactor. Existing integration tests across `tests/scoped_tokens_slice3b.rs`, `tests/links_contract.rs`, `tests/ai_contract.rs` exercise visibility filtering and will catch regressions.

## Done criteria

- [ ] `Visibility::push_where` (or equivalent helper) defined in `src/db/links.rs`
- [ ] `grep -c "Visibility::Public =>" src/db/documents.rs` count significantly reduced (goal: ≤ 3 remaining, for genuinely structure-varying matches)
- [ ] `cargo check --all-targets` exits 0
- [ ] `cargo clippy --all-targets -- -D warnings` exits 0
- [ ] `cargo fmt --check` exits 0
- [ ] `cargo nextest run` exits 0
- [ ] `plans/README.md` status row updated

## STOP conditions

- A visibility match cannot be reduced because the three arms use entirely different SQL (not just different WHERE predicates). Document each such match with a comment and stop for that one; proceed with the reducible ones.
- `QueryBuilder` does not support the push_bind pattern for i64 as expected. Report the specific error.

## Maintenance notes

- When adding a new `Visibility` variant in the future, add it to `push_where` first — the compiler will then point at every remaining three-arm match that needs updating.
- The `push_where` helper is the single source of truth for visibility SQL semantics. Its unit tests (add at least one per variant in `#[cfg(test)]` at the bottom of `links.rs`) verify it in isolation.
