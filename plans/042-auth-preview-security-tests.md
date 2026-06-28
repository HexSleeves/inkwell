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

The repo's access-control test coverage is mostly strong already. This plan fills the **two genuine gaps** that remain, and explicitly does NOT duplicate tests that already exist.

**Already covered — do NOT re-add (verified at `0819727`):**
- Wrong-document token → 401: `preview_with_wrong_slug_returns_401` (`tests/preview_contract.rs:199`).
- Revoked token → 401: `revoked_token_returns_401` (`tests/preview_contract.rs:228`).
- Past-expiry rejected at creation → 400: `preview_token_with_past_expiry_rejected_at_creation` (`tests/preview_contract.rs:282`).
- Cross-author draft isolation (A sees own draft 200; A/B cannot see each other's draft 404): `owner_aware_read_visibility` (`tests/scoped_tokens_slice3b.rs:140`, assertions ~176-197).

**The two real gaps this plan fills:**
1. **A token that was valid but has since expired → 401 at GET.** Creation-time rejection is tested, but not a token whose `expires_at` has passed *after* minting. The mint endpoint rejects a past expiry at creation, so this must be set up by directly updating the `preview_tokens.expires_at` row to a past timestamp via the test pool, then hitting the preview GET.
2. **HTTP-level stale `If-Match` → 409.** Implemented at `src/http/api.rs:624-647` and tested via the MCP tool, but there is no HTTP-API test that a stale `If-Match` header yields 409.

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

**In scope** (add tests only — do NOT change production code, and do NOT re-add the already-existing tests listed in "Why this matters"):
- `tests/preview_contract.rs` — add ONE test: a previously-valid token whose `expires_at` is now in the past → GET returns 401.
- A new file `tests/if_match_contract.rs` — the HTTP stale-`If-Match` → 409 test.

**Out of scope**:
- Any `src/` production code — if a test reveals a bug, STOP and report; do not fix it here.
- Re-adding wrong-document / revoked / past-expiry-at-creation / cross-author tests — they already exist (see "Why this matters"). Adding duplicates wastes effort and risks name collisions.

## Git workflow

- Branch: `advisor/042-auth-preview-tests`
- Commit: `test: cover expired-at-GET preview token and HTTP stale If-Match`

## Steps

### Step 1: Expired-token-at-GET test (via direct DB update)

The mint endpoint rejects a past expiry at creation (already tested), so set up an *already-minted, then expired* token by updating the row. In `tests/preview_contract.rs`, add:
1. `create_draft(&router, "exp-after")` and `mint_preview_token(&router, "exp-after")` → `(token, prefix)`.
2. Confirm it works first: `GET /documents/exp-after/preview?token={token}` → 200.
3. Expire it directly via the test pool. The test already holds `pool` (use `common::router_for(pool)` but keep a clone of `pool` first, OR re-acquire via `common::maybe_pool`). Run:
   ```rust
   sqlx::query("UPDATE preview_tokens SET expires_at = now() - interval '1 hour' WHERE prefix = $1")
       .bind(&prefix)
       .execute(&pool)
       .await?;
   ```
   Confirm the actual column names against `migrations/0022_create_preview_tokens.sql` and `src/db/preview_tokens.rs` (the prefix column may be named `prefix` or `token_prefix` — verify before binding).
4. `GET /documents/exp-after/preview?token={token}` again → assert `StatusCode::UNAUTHORIZED`.

To hold the pool for the UPDATE: structure the test like the others but bind `let pool = common::maybe_pool().await?` then `let router = common::router_for(pool.clone());` so you can run the UPDATE on `&pool` after minting.

**Verify**: `cargo check --all-targets` → exit 0

### Step 2: HTTP stale-If-Match test

Create `tests/if_match_contract.rs` (mirror `tests/preview_contract.rs`: `mod common;`, `DB_TEST_LOCK`, `db_guard`, `SHARED_KEY`, `body_json`, the `maybe_pool`/early-return pattern):
1. `POST /documents` (x-api-key SHARED_KEY) → read `version` v1 from the JSON envelope.
2. `PATCH /documents/{slug}` with header `If-Match: {v1}` and a body change → assert 200; read new `version` v2.
3. `PATCH /documents/{slug}` again with the **stale** `If-Match: {v1}` → assert `StatusCode::CONFLICT` (409).

**Verify**: `cargo check --all-targets` → exit 0; (with DB) `cargo nextest run --test if_match_contract` → all pass

### Step 3: Run the affected suites

**Verify (with DB)**: `DATABASE_URL=... cargo nextest run --test preview_contract --test if_match_contract` → all pass

## Test plan

Two new tests only:
- `preview_contract.rs`: previously-valid token, `expires_at` set to the past via DB → GET → 401.
- `if_match_contract.rs`: stale `If-Match` → 409.

(Wrong-document, revoked, past-expiry-at-creation, and cross-author isolation are already covered — do not duplicate.)

## Done criteria

- [ ] `cargo check --all-targets` exits 0 (tests compile)
- [ ] Exactly ONE new `#[tokio::test]` in `preview_contract.rs` (expired-at-GET); no duplicates of existing tests
- [ ] New `tests/if_match_contract.rs` with the stale-If-Match test
- [ ] With a DB available: `cargo nextest run --test preview_contract --test if_match_contract` passes
- [ ] No `src/` files modified (`git status`)
- [ ] `plans/README.md` status row updated

## STOP conditions

- **A new test fails because production behaviour is wrong** (e.g. an expired token returns 200, or a stale If-Match returns 200 instead of 409). Real bug — STOP, do not modify production code, report the failing test and observed behaviour.
- The `preview_tokens` expiry/prefix column names differ from the UPDATE in Step 1. Read `migrations/0022_create_preview_tokens.sql` and use the actual names.
- No `DATABASE_URL` available: write and compile the tests, report that runtime pass needs a DB.

## Maintenance notes

- When preview token semantics change (new expiry rules, new scopes that can mint), extend `tests/preview_contract.rs`.
- The cross-author isolation invariant is guarded by `owner_aware_read_visibility` in `tests/scoped_tokens_slice3b.rs`; if `resolve_visibility`/`owner_filter` is refactored (plan 038 or 039), keep that test green.
