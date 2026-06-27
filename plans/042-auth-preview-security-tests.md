# Plan 042: Add contract tests for preview expiry/revocation, HTTP If-Match, and cross-author isolation

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `plans/README.md`.
>
> **Drift check (run first)**: `git diff --stat 0819727..HEAD -- src/http/preview.rs src/http/api.rs src/db/preview_tokens.rs tests/preview_contract.rs tests/scoped_tokens_slice3b.rs`
> If any in-scope file changed, compare the "Current state" excerpts before proceeding.

## Status

- **Priority**: P1
- **Effort**: M
- **Risk**: LOW
- **Depends on**: none
- **Category**: tests
- **Planned at**: commit `0819727`, 2026-06-26

## Why this matters

Three security-critical behaviours have implementation but no (or incomplete) integration test coverage. A regression in any of them silently weakens access control with no failing test:

1. **Preview token expiry & revocation** — `tests/preview_contract.rs` covers the happy path (valid token grants draft access) but does not assert that an *expired* token returns 401, a *revoked* token returns 401, or a token for document A used on document B returns 401. These are the entire safety contract of preview links.
2. **HTTP If-Match optimistic concurrency** — the 409-on-stale-write path is implemented at `src/http/api.rs:624-647` and tested via the MCP tool, but there is no HTTP-level test that a stale `If-Match` returns 409.
3. **Cross-author draft isolation** — `tests/scoped_tokens_slice3b.rs` proves an author sees their own drafts, but there is no explicit test that author B with a valid `read` token gets **404** (not 200) when reading author A's draft by slug.

## Current state

**Preview verification logic** — `src/http/preview.rs:194-221` (all failures return 401):
```rust
// Revoked token → 401.
if row.revoked_at.is_some() { return Err(AppError::Unauthorized); }
// Expired token → 401.
if row.expires_at.is_some_and(|exp| exp < OffsetDateTime::now_utc()) {
    return Err(AppError::Unauthorized);
}
// ... document lookup ...
// The token must match THIS document.
if document.id != row.document_id { return Err(AppError::Unauthorized); }
```

**Preview test helpers** — `tests/preview_contract.rs:30-78` already provide `create_draft(router, slug)` and `mint_preview_token(router, slug) -> (token, prefix)`. Reuse them. The mint helper POSTs to `/documents/{slug}/preview-tokens` with no body. **To mint a token with a custom (or zero) expiry, check whether the mint endpoint accepts an `expiresAt`/`ttl` body field** — read `src/http/preview.rs` `preview_tokens` handler and `src/db/preview_tokens.rs` to find the exact field name before writing the expiry test.

**Revocation** — `DELETE /documents/{slug}/preview-tokens/{prefix}` revokes. The prefix is returned by `mint_preview_token`.

**If-Match handler** — `src/http/api.rs:624-647`:
```rust
let document = match parse_if_match(&headers)? {
    Some(expected_version) => {
        match documents::update_document_by_slug_if_version(&state.pool, &slug, expected_version, patch, owner).await? {
            documents::ConditionalUpdate::Updated(document) => *document,
            documents::ConditionalUpdate::NotFound => { return Err(AppError::NotFound(...)); }
            documents::ConditionalUpdate::VersionMismatch { current } => {
                return Err(AppError::Conflict(format!("Document \"{slug}\" has version {current}, not the expected {expected_version}. ...")));
            }
        }
    }
    None => { /* unconditional update */ }
};
```
The version is returned to clients via the `ETag` header on GET and the `version` field in the JSON envelope. The `If-Match` request header carries the expected version.

**Test conventions** (from `tests/preview_contract.rs` and `tests/scoped_tokens_slice3b.rs`):
- `mod common;` then `common::maybe_pool().await?` (returns `Ok(None)` → early `return Ok(())` when no DB)
- A `static DB_TEST_LOCK` Mutex + `db_guard().await` to serialize DB tests
- `const SHARED_KEY: &str = "test-secret-key";` is the admin key
- `router_for(pool)` builds the router; `tower::ServiceExt::oneshot` drives requests
- `mint_token(router, name, scopes)` in slice3b mints a scoped author token via `/admin/tokens`
- Assertions use `assert_eq!(response.status(), StatusCode::...)`

## Commands you will need

| Purpose      | Command                                                          | Expected on success |
|--------------|-----------------------------------------------------------------|---------------------|
| Run preview tests | `DATABASE_URL=... cargo nextest run --test preview_contract` | all pass        |
| Run slice3b tests | `DATABASE_URL=... cargo nextest run --test scoped_tokens_slice3b` | all pass   |
| Typecheck    | `cargo check --all-targets`                                     | exit 0              |
| All tests    | `DATABASE_URL=... cargo test --all`                            | all pass            |

These tests are DB-backed. They are skipped when `DATABASE_URL` is unset (`maybe_pool` returns `None`). To actually exercise them, a Postgres with pgvector must be available and `DATABASE_URL` set. If no DB is available in your environment, write the tests, confirm `cargo check --all-targets` passes (compiles), and report that runtime verification needs a DB.

## Scope

**In scope** (add tests only — do NOT change production code):
- `tests/preview_contract.rs` — add expiry, revocation, wrong-document tests
- `tests/scoped_tokens_slice3b.rs` — add cross-author 404 test (or a new `tests/if_match_contract.rs` for the If-Match test)
- A new file `tests/if_match_contract.rs` for the HTTP If-Match test (or add to `tests/api_contract.rs` — choose based on which has compatible helpers)

**Out of scope**:
- Any `src/` production code — if a test reveals a bug, STOP and report; do not fix it here
- Changing existing passing tests

## Git workflow

- Branch: `advisor/042-auth-preview-tests`
- Commit: `test: cover preview expiry/revocation, HTTP If-Match, cross-author isolation`

## Steps

### Step 1: Preview wrong-document test

In `tests/preview_contract.rs`, add a test:
1. `create_draft(&router, "doc-a")` and `create_draft(&router, "doc-b")`
2. `mint_preview_token(&router, "doc-a")` → `(token_a, _)`
3. `GET /documents/doc-b/preview?token={token_a}` → assert `StatusCode::UNAUTHORIZED`

**Verify**: `cargo check --all-targets` → exit 0

### Step 2: Preview revocation test

Add a test:
1. `create_draft`, `mint_preview_token` → `(token, prefix)`
2. Confirm `GET /documents/{slug}/preview?token={token}` → 200
3. `DELETE /documents/{slug}/preview-tokens/{prefix}` with `x-api-key: SHARED_KEY` → assert success (200/204 — check the handler)
4. `GET /documents/{slug}/preview?token={token}` again → assert `StatusCode::UNAUTHORIZED`

**Verify**: `cargo check --all-targets` → exit 0

### Step 3: Preview expiry test

First read `src/http/preview.rs` `preview_tokens` (mint) handler and `src/db/preview_tokens.rs` to determine how expiry is set. Two cases:
- **If the mint endpoint accepts an expiry/ttl in the request body**: mint a token with a 0-second or past expiry, then `GET .../preview?token=...` → assert 401.
- **If expiry can only be set internally**: mint a token, then directly UPDATE the `preview_tokens` row's `expires_at` to a past timestamp via `sqlx::query` on the test `pool` (the test has the pool), then GET → assert 401. Use the `prefix` to locate the row.

**Verify**: `cargo check --all-targets` → exit 0

### Step 4: Cross-author isolation test

In `tests/scoped_tokens_slice3b.rs` (which already has `mint_token` and `create_draft(router, title, body, key)`):
1. `mint_token(&router, "author-a", &["read", "write"])` → `token_a`
2. `mint_token(&router, "author-b", &["read", "write"])` → `token_b`
3. `create_draft(&router, "A secret", "body", &token_a)` → `slug_a` (a draft owned by author A)
4. `GET /documents/{slug_a}` with `x-api-key: {token_b}` (author B's read token) → assert `StatusCode::NOT_FOUND` (not 200 — B must not see A's draft)
5. Sanity: `GET /documents/{slug_a}` with `x-api-key: {token_a}` → assert `StatusCode::OK` (A sees own draft)

**Verify**: `cargo check --all-targets` → exit 0

### Step 5: HTTP If-Match stale-write test

Create `tests/if_match_contract.rs` (mirror the structure of `tests/preview_contract.rs`: `mod common;`, `DB_TEST_LOCK`, `SHARED_KEY`, `body_json`):
1. Create a document via `POST /documents` (x-api-key SHARED_KEY) → read its `version` (call it v1) from the JSON envelope
2. `PATCH /documents/{slug}` with `If-Match: {v1}` and a body change → assert 200, read new `version` v2
3. `PATCH /documents/{slug}` again with the **stale** `If-Match: {v1}` → assert `StatusCode::CONFLICT` (409)

**Verify**: `cargo check --all-targets` → exit 0; (with DB) `cargo nextest run --test if_match_contract` → all pass

### Step 6: Run the full suite

**Verify (with DB)**: `DATABASE_URL=... cargo test --all` → all pass

## Test plan

This plan IS the test plan. New tests:
- `preview_contract.rs`: wrong-document → 401, revoked → 401, expired → 401
- `scoped_tokens_slice3b.rs`: author B reading author A's draft → 404; author A reading own → 200
- `if_match_contract.rs`: stale If-Match → 409

## Done criteria

- [ ] `cargo check --all-targets` exits 0 (tests compile)
- [ ] At least 3 new `#[tokio::test]` functions in `preview_contract.rs`
- [ ] At least 1 new cross-author test in `scoped_tokens_slice3b.rs`
- [ ] New `tests/if_match_contract.rs` with the stale-If-Match test
- [ ] With a DB available: `cargo test --all` exits 0, all new tests pass
- [ ] No `src/` files modified (`git status`)
- [ ] `plans/README.md` status row updated

## STOP conditions

- **A new test fails because production behaviour is wrong** (e.g. author B CAN read author A's draft, or an expired token returns 200). This is a real security bug — STOP, do not modify production code, and report the failing test and observed behaviour. This is the most important STOP condition in this plan.
- The mint endpoint's expiry field cannot be determined from `src/http/preview.rs` / `src/db/preview_tokens.rs`. Report and use the direct DB-update approach (Step 3, second case).
- No `DATABASE_URL` available: write and compile the tests, report that runtime pass needs a DB.

## Maintenance notes

- When preview token semantics change (new expiry rules, new scopes that can mint), these tests are the guard — extend them.
- The cross-author isolation test is the canonical proof of the slice-3b invariant; if `resolve_visibility` or `owner_filter` is refactored (e.g. plan 038 or 039), this test must stay green.
