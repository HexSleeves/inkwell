# Plan 036: Extract repeated SELECT column list to a const in documents.rs

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `plans/README.md`.
>
> **Drift check (run first)**: `git diff --stat 0819727..HEAD -- src/db/documents.rs`
> If this file changed since this plan was written, compare the "Current state"
> excerpts against the live code before proceeding.

## Status

- **Priority**: P3
- **Effort**: S
- **Risk**: LOW
- **Depends on**: none (can run after or alongside 032)
- **Category**: tech-debt
- **Planned at**: commit `0819727`, 2026-06-26

## Why this matters

The SELECT column list `id, slug, title, body_markdown, rendered_html, status, growth, tags, version, created_at, updated_at` appears 15+ times in `src/db/documents.rs`. Adding a new column to the `Document` struct requires 15 separate edits to stay in sync. Missing one edit causes a silent mismatch (SQLx maps by column name, so an extra DB column is silently ignored, but a missing column causes a runtime error). Extracting the list to a `const` means a single change updates every query.

## Current state

**`src/db/documents.rs:55`** (and 14+ other locations) — column list appears verbatim:
```
SELECT id, slug, title, body_markdown, rendered_html, status, growth, tags, version, created_at, updated_at
```

This exact column list appears ~18 times: in `SELECT` clauses, in `QueryBuilder::new("SELECT ... FROM documents")` calls, AND in 5 `INSERT/UPDATE ... RETURNING` clauses (around lines 29, 152, 217, 332, 415).

**There are NO compile-time `sqlx::query!`/`query_as!` macros in this file** — `grep -nE 'sqlx::query_as!|sqlx::query!' src/db/documents.rs` returns nothing. Every occurrence is either a runtime `sqlx::query_as::<Postgres, Document>(r#"..."#)` / `sqlx::query::<...>(...)` call or a `QueryBuilder` `.new(...)`/`.push(...)` string. ALL of them can use the runtime const via `format!`. (One column list is deliberately DIFFERENT and must NOT be touched — see "Out of scope".)

## Commands you will need

| Purpose     | Command                                               | Expected on success |
|-------------|-------------------------------------------------------|---------------------|
| Count before | `grep -c "body_markdown, rendered_html" src/db/documents.rs` | baseline count |
| Typecheck   | `cargo check --all-targets`                           | exit 0              |
| Tests       | `cargo nextest run`                                   | all pass            |
| Lint        | `cargo clippy --all-targets -- -D warnings`           | exit 0              |

## Scope

**In scope**:
- `src/db/documents.rs` — extract const and replace all manual string occurrences

**Out of scope** (do NOT replace these):
- The **INSERT column list** at `src/db/documents.rs:22` — `INSERT INTO documents (slug, title, body_markdown, rendered_html, status, growth, tags, owner_id)`. This is a DIFFERENT list (no leading `id`, includes `owner_id`, omits `version`/`created_at`/`updated_at`). A literal const replace won't match it, but do not hand-edit it to use `DOCUMENT_COLUMNS`.
- `src/domain/document.rs` — `Document` struct definition does not change

## Git workflow

- Branch: `advisor/036-document-columns-const`
- Commit: `refactor(db): extract repeated Document SELECT columns to a const`

## Steps

### Step 1: Add the const

At the top of `src/db/documents.rs`, after the imports, add:
```rust
const DOCUMENT_COLUMNS: &str =
    "id, slug, title, body_markdown, rendered_html, status, growth, tags, version, created_at, updated_at";
```

**Verify**: `cargo check --all-targets` → exit 0

### Step 2: Replace occurrences in runtime query strings

For every `sqlx::query_as::<Postgres, Document>(r#"SELECT id, slug, title, body_markdown, rendered_html, status, growth, tags, version, created_at, updated_at FROM documents..."#)` form:

Change to a `format!` or string concatenation using the const:
```rust
sqlx::query_as::<Postgres, Document>(
    &format!("SELECT {DOCUMENT_COLUMNS} FROM documents WHERE slug = $1 AND status = $2"),
)
```

For `QueryBuilder` calls like `QueryBuilder::new("SELECT id, slug, title, ... FROM documents")`:
```rust
QueryBuilder::<Postgres>::new(&format!("SELECT {DOCUMENT_COLUMNS} FROM documents"))
```

Note: `QueryBuilder::new` takes `impl Into<String>`, so `&format!(…)` as `&str` works.

**Also replace the 5 `RETURNING` clauses** (around lines 29, 152, 217, 332, 415) — these are INSERT/UPDATE statements that end with `RETURNING id, slug, title, body_markdown, rendered_html, status, growth, tags, version, created_at, updated_at`. They use the SAME list as the SELECTs, so interpolate the const there too:
```rust
&format!("... RETURNING {DOCUMENT_COLUMNS}")
```
If you only convert the SELECT/QueryBuilder forms and skip the RETURNING clauses, the done-criteria grep will still find ~6 matches and fail.

Work through the file systematically from top to bottom. Run `cargo check` after every 3–4 substitutions to catch errors early.

**Verify**: `grep -c "id, slug, title, body_markdown, rendered_html, status, growth, tags, version" src/db/documents.rs` → 1 (only the `DOCUMENT_COLUMNS` const definition line still contains the literal list).

### Step 3: Verify all existing tests still pass

**Verify**: `cargo nextest run` → all pass

## Done criteria

- [ ] `const DOCUMENT_COLUMNS: &str = "..."` exists at the top of `src/db/documents.rs`
- [ ] `grep -c "body_markdown, rendered_html, status, growth, tags, version" src/db/documents.rs` → 1 (only the const definition line; SELECTs, QueryBuilders, and RETURNING clauses all interpolate it)
- [ ] `cargo check --all-targets` exits 0
- [ ] `cargo clippy --all-targets -- -D warnings` exits 0
- [ ] `cargo fmt --check` exits 0
- [ ] `cargo nextest run` exits 0
- [ ] `plans/README.md` status row updated

## STOP conditions

- A compile-time `sqlx::query_as!`/`query!` macro turns up using the full column list (none exist at `0819727`, but if the codebase added one, its column list can't reference a runtime const — leave it and report).
- `format!` with the const produces a string that SQLx's runtime parser rejects. Report the specific query.

## Maintenance notes

- When a new column is added to the `Document` struct and the `documents` table, update `DOCUMENT_COLUMNS` once — all runtime queries pick it up automatically.
- The `DocumentSummary` query (plan 032) uses a different column set — do NOT use `DOCUMENT_COLUMNS` for those; they intentionally omit body fields.
