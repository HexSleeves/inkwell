# Plan 043: Add contract tests for input validation and backfill fan-out

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `plans/README.md`.
>
> **Drift check (run first)**: `git diff --stat 0819727..HEAD -- src/http/api.rs src/domain/document.rs src/garden.rs tests/api_contract.rs tests/links_contract.rs`
> If any in-scope file changed, compare the "Current state" excerpts before proceeding.

## Status

- **Priority**: P2
- **Effort**: M
- **Risk**: LOW
- **Depends on**: none
- **Category**: tests
- **Planned at**: commit `0819727`, 2026-06-26

## Why this matters

Two classes of behaviour lack tests:

1. **Input validation contracts** — the write API has size and value limits (`MAX_TITLE_LENGTH`, `MAX_BODY_MARKDOWN_LENGTH`, valid `GrowthStage` values, unique slug) but there are no tests asserting the response codes for violations. Without them, a refactor could silently change a 409 to a 500, or start accepting an invalid growth stage, with no failing test. These tests also *characterize* current behaviour — if any limit is unenforced, the test reveals it.
2. **Backfill fan-out** — `garden::backfill_after_change` re-renders inbound linkers when a target is created/published/renamed (so `[[link]]` stubs light up). This is complex write-path orchestration with zero direct assertion in `tests/links_contract.rs`.

## Current state

**Validation limits** — `src/domain/document.rs:5-15`:
```rust
pub const MAX_SLUG_LENGTH: usize = 200;
pub const MAX_TITLE_LENGTH: usize = 500;
pub const MAX_TAG_LENGTH: usize = 50;
pub const MAX_TAGS: usize = 20;
pub const MAX_BODY_MARKDOWN_LENGTH: usize = 262_144;
pub const MAX_REQUEST_BODY_BYTES: usize = 1_000_000;
```

**Slug collision** — handled in `src/db/documents.rs`: `map_duplicate_slug` maps the Postgres unique violation (`23505`) to a `DbError` ("A document with slug ... already exists.") which converts to `AppError::Conflict` → **409** (not 500). The create handler relies on the unique index, not a check-then-insert.

**Growth parsing** — the create handler reads the JSON field **`growth`** (NOT `growthStage`), via `resolve_growth` (`src/http/api.rs:813`). The existing test in `tests/api_contract.rs:93` sends `"growth": "evergreen"`. `resolve_growth` returns a `400 BadRequest` for a present-but-unknown `growth` string (verified), so the intended assertion below is correct **as long as the field is named `growth`**. Do NOT use `growthStage` — that field is ignored and the request would succeed with the default `seedling`, producing a false "validation gap".

**Backfill** — `src/garden.rs:327-329`:
```rust
pub async fn backfill_after_change(pool: &PgPool, note_id: Uuid, slug: &str) {
    let affected = affected_sources(pool, note_id, slug).await;
    rerender_sources(pool, &affected).await;
}
```
Effect: when note B is created/published, any note A whose body contains `[[B]]` is re-rendered so its stored `rendered_html` changes from a stub link to a resolved link.

**Test conventions**: see `tests/api_contract.rs` and `tests/links_contract.rs` — `mod common;`, `common::maybe_pool()`, `router_for(pool)`, `tower::ServiceExt::oneshot`, `SHARED_KEY = "test-secret-key"`, JSON via `serde_json::json!`.

## Commands you will need

| Purpose      | Command                                                       | Expected on success |
|--------------|-------------------------------------------------------------|---------------------|
| Typecheck    | `cargo check --all-targets`                                  | exit 0              |
| Run API tests | `DATABASE_URL=... cargo nextest run --test api_contract`    | all pass            |
| Run link tests | `DATABASE_URL=... cargo nextest run --test links_contract` | all pass            |
| All tests    | `DATABASE_URL=... cargo test --all`                         | all pass            |

DB-backed; skipped when `DATABASE_URL` is unset. If no DB is available, write the tests, confirm `cargo check --all-targets` passes, and report that runtime verification needs a DB.

## Scope

**In scope** (tests only):
- `tests/api_contract.rs` — validation tests (slug collision, oversize title/body, invalid growth, empty slug)
- `tests/links_contract.rs` — backfill fan-out test

**Out of scope**:
- All `src/` production code — if a validation test reveals missing enforcement, STOP and report; do not add validation code here

## Git workflow

- Branch: `advisor/043-validation-fanout-tests`
- Commit: `test: cover write-API input validation and backfill fan-out`

## Steps

### Step 1: Slug-collision test (expect 409)

In `tests/api_contract.rs`, add a test:
1. `POST /documents` with `{ "title": "First", "slug": "dupe", "bodyMarkdown": "x" }` (x-api-key SHARED_KEY) → assert 201
2. `POST /documents` with `{ "title": "Second", "slug": "dupe", "bodyMarkdown": "y" }` → assert `StatusCode::CONFLICT` (409)

**Verify**: `cargo check --all-targets` → exit 0

### Step 2: Oversize title and body tests

Add tests:
1. `POST /documents` with a `title` of 501+ characters (> `MAX_TITLE_LENGTH` = 500) → assert the documented rejection code (`StatusCode::BAD_REQUEST` 400, or 413). Read the handler to confirm which; assert that exact code.
2. `POST /documents` with a `bodyMarkdown` exceeding `MAX_BODY_MARKDOWN_LENGTH` (262_144) but the whole request under `MAX_REQUEST_BODY_BYTES` (1_000_000) → assert the documented rejection code.

To build a long string: `"a".repeat(501)` and `"a".repeat(300_000)`.

**Verify**: `cargo check --all-targets` → exit 0

### Step 3: Invalid growth-stage test

- Add a test: `POST /documents` with `{ "title": "G", "bodyMarkdown": "x", "growth": "cursed" }` (the field is `growth`, NOT `growthStage`).
- Assert `StatusCode::BAD_REQUEST` (verified contract: `resolve_growth` at `src/http/api.rs:813` rejects an unknown value with 400).
- Sanity guard: if you accidentally send `growthStage`, the request returns 201 because the field is ignored — that is a test bug, not a validation gap. Use `growth`.

**Verify**: `cargo check --all-targets` → exit 0

### Step 4: Empty/whitespace slug test

Add a test: `POST /documents` with `{ "title": "E", "slug": "   ", "bodyMarkdown": "x" }` → assert the documented behaviour. Two acceptable outcomes; assert whichever the handler implements and lock it in:
- 400 (rejected), OR
- 201 with a slug derived from the title (whitespace slug ignored).

Read the handler/`src/domain/slug.rs` to decide which to assert. If neither — if it stores an empty slug — STOP and report (empty slug corrupts routing).

**Verify**: `cargo check --all-targets` → exit 0

### Step 5: Backfill fan-out test

In `tests/links_contract.rs`, add a test that proves a create/publish triggers re-render of inbound linkers:
1. Create note A with body containing `[[future-note]]` → A's `rendered_html` should contain a *stub* (unresolved) link (assert it contains the stub marker — read existing links_contract tests to see how stubs are asserted, e.g. a CSS class like `wikilink-missing` or similar).
2. Create note B with slug `future-note`.
3. `GET /documents/{slug_a}` and assert A's `rendered_html` now contains a *resolved* link to `future-note` (the stub lit up via backfill).

The stub marker is `class="stub"` (`src/rendering/wikilink.rs:227,322`; asserted in `tests/links_contract.rs:237`) — a resolved link lacks that class. Model the stub/resolved assertions on the existing wikilink tests in `tests/links_contract.rs` (the one near line 237) and reuse its exact assertion strings.

**Verify**: `cargo check --all-targets` → exit 0; (with DB) `cargo nextest run --test links_contract` → all pass

### Step 6: Run the full suite

**Verify (with DB)**: `DATABASE_URL=... cargo test --all` → all pass

## Test plan

New tests:
- `api_contract.rs`: slug collision → 409; oversize title → 400/413; oversize body → 400/413; invalid growth → 400; empty slug → documented behaviour
- `links_contract.rs`: stub → resolved transition after target creation (backfill)

## Done criteria

- [ ] `cargo check --all-targets` exits 0
- [ ] ≥ 4 new validation tests in `api_contract.rs`
- [ ] 1 new backfill test in `links_contract.rs`
- [ ] With a DB: `cargo test --all` exits 0
- [ ] No `src/` files modified (`git status`)
- [ ] `plans/README.md` status row updated

## STOP conditions

- **A validation test cannot pass because the limit is not enforced** (oversize accepted, invalid growth silently defaulted, empty slug stored). Report each such gap as a finding — do NOT add enforcement code in this tests-only plan.
- The existing `links_contract.rs` tests do not show how stub vs resolved links are marked in HTML. Read `src/rendering/wikilink.rs` to find the exact markup, then assert on it.
- No DB available: compile the tests, report runtime needs a DB.

## Maintenance notes

- If any STOP condition fires (a real validation gap), spin a follow-up bug-fix plan to add the missing enforcement, then this test locks it in.
- The backfill test is the only direct assertion of the fan-out; if plan 034 (concurrent backfill) lands, this test must stay green — it proves correctness is preserved under the concurrency change.
