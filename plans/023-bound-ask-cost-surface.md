# Plan 023: Bound the public ask cost surface

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report. When done, update the status row for this plan in `plans/README.md`.
>
> **Drift check (run first)**: `git diff --stat fef38ad..HEAD -- src/http/ai.rs src/http/router.rs tests/ai_contract.rs`
> If any in-scope file changed since this plan was written, compare the
> "Current state" excerpts against the live code before proceeding; on a
> mismatch, treat it as a STOP condition.

## Status

- **Priority**: P1
- **Effort**: S
- **Risk**: LOW
- **Depends on**: none
- **Category**: security/perf
- **Planned at**: commit `fef38ad`, 2026-06-23

## Why this matters

`GET|POST /ask` is public and, when AI keys are configured, one request can call
both Voyage and Anthropic. The handler accepts arbitrary query length up to
framework/request limits and has no endpoint-specific guard before provider
work starts. A cheap first slice should cap the question size and add regression
tests so accidental long-query cost or latency cannot grow unnoticed.

This is not a full rate-limiting plan. It is the smallest deterministic guard
that belongs in the application code before any provider call is made.

## Current state

- `src/http/router.rs` wires `/ask` as a public route:

```rust
// src/http/router.rs:42-45
Router::new()
    .route("/health", any(api::health))
    .route("/ask", any(ai::ask))
    .route("/documents", any(api::documents))
```

- `src/http/ai.rs` accepts GET query text or POST JSON, trims it, and only
  rejects empty strings:

```rust
// src/http/ai.rs:153-172
let raw_query = match method {
    Method::GET => query.0.q.unwrap_or_default(),
    Method::POST => {
        if body.is_empty() {
            query.0.q.unwrap_or_default()
        } else {
            let parsed: AskBody = serde_json::from_slice(&body).map_err(|_| {
                AppError::BadRequest("Request body must be JSON with a \"q\" field.".into())
            })?;
            parsed.q.or(query.0.q).unwrap_or_default()
        }
    }
    _ => return Err(AppError::MethodNotAllowed(vec!["GET", "POST"])),
};
let trimmed = raw_query.trim().to_string();
if trimmed.is_empty() {
    return Err(AppError::BadRequest(
        "Query param \"q\" is required and must be non-empty.".into(),
    ));
}
```

- Provider work happens after that validation:

```rust
// src/http/ai.rs:195-198
let answer = llm
    .answer(&trimmed, &context_blocks)
    .await
    .map_err(AppError::Internal)?;
```

```rust
// src/http/ai.rs:225-229
let embeddings = state
    .embedder
    .embed(&[query.to_string()])
    .await
    .map_err(AppError::Internal)?;
```

- Existing tests for `/ask` live in `tests/ai_contract.rs` and use
  `common::maybe_router_with_ai()` for deterministic no-network provider
  coverage. Match that style.

## Commands you will need

| Purpose | Command | Expected on success |
|---------|---------|---------------------|
| Format | `cargo fmt --check` | exit 0 |
| Lint | `cargo clippy --all-targets --all-features -- -D warnings` | exit 0 |
| Tests | `cargo test --all` | exit 0 |
| DB tests | `INKWELL_REQUIRE_DB_TESTS=1 DATABASE_URL=<postgres-url> cargo test --all` | exit 0 when a pgvector Postgres is available |

## Scope

**In scope**:

- `src/http/ai.rs`
- `tests/ai_contract.rs`
- `src/http/router.rs` only if you add an endpoint-specific body limit layer

**Out of scope**:

- Global rate limiting
- Authentication changes for `/ask`
- Provider client changes in `src/ai/claude.rs` or `src/ai/voyage.rs`
- Any response-shape change except returning the existing JSON error envelope
  for oversized questions

## Git workflow

- Branch: `advisor/023-bound-ask-cost-surface`
- Commit message style: match the repo's conventional history, e.g.
  `fix(ai): bound public ask query size`
- Do not push or open a PR unless instructed.

## Steps

### Step 1: Add a question length guard

In `src/http/ai.rs`, add a small constant near `ASK_TOP_K`, for example:

```rust
const MAX_ASK_QUERY_CHARS: usize = 1_000;
```

Add a helper that trims the raw query and validates by character count, not
byte count, so non-ASCII questions are handled correctly:

```rust
fn validate_ask_query(raw_query: String) -> Result<String, AppError> {
    let trimmed = raw_query.trim().to_string();
    if trimmed.is_empty() {
        return Err(AppError::BadRequest(
            "Query param \"q\" is required and must be non-empty.".into(),
        ));
    }
    if trimmed.chars().count() > MAX_ASK_QUERY_CHARS {
        return Err(AppError::BadRequest(format!(
            "Query param \"q\" must be at most {MAX_ASK_QUERY_CHARS} characters."
        )));
    }
    Ok(trimmed)
}
```

Replace the inline empty-query block with this helper before `state.llm` or
`retrieve_context` can run.

**Verify**: `cargo test --lib http::ai` may have no tests; continue to Step 2.

### Step 2: Add DB-backed contract tests

In `tests/ai_contract.rs`, add two tests near `ask_empty_query_is_a_bad_request`:

- `ask_rejects_overlong_get_query_before_provider_work`
- `ask_rejects_overlong_post_query_before_provider_work`

Use `common::maybe_router_with_ai().await?` like the existing tests. Build an
overlong string with `"a".repeat(MAX + 1)`. Because `MAX_ASK_QUERY_CHARS` is not
public, either duplicate the boundary in the test with a comment pointing to
`src/http/ai.rs`, or expose a `pub(crate)` helper only if that does not leak API
surface outside the crate.

Expected assertions:

- status is `400 BAD_REQUEST`;
- response body contains the JSON error envelope;
- no note creation is needed for these tests because validation must happen
  before retrieval.

**Verify**: `DATABASE_URL=<postgres-url> cargo test --test ai_contract ask_rejects_overlong` -> both new tests pass.

### Step 3: Consider route body limiting only if the tests expose a gap

Axum's `Bytes` extractor has a default body limit, but this route should not
need to buffer a large JSON body just to reject an overlong `q`. If you add a
route-specific `DefaultBodyLimit`, keep it narrow to `/ask` and do not change
write API limits.

Only touch `src/http/router.rs` if you can keep the change local and tests
still pass. If this becomes awkward because of router layering, STOP and leave
the query validation in place; do not refactor the router.

**Verify**: `cargo clippy --all-targets --all-features -- -D warnings` -> exit 0.

## Test plan

- New integration tests in `tests/ai_contract.rs` for overlong GET and POST
  questions.
- Existing tests to rerun:
  - `DATABASE_URL=<postgres-url> cargo test --test ai_contract`
  - `cargo test --all`
- Full gate:
  - `cargo fmt --check`
  - `cargo clippy --all-targets --all-features -- -D warnings`

## Done criteria

- [ ] `/ask?q=<overlong>` returns `400` before provider work.
- [ ] `POST /ask` with an overlong JSON `q` returns `400`.
- [ ] Empty-query behavior remains `400`.
- [ ] Normal `/ask` tests still pass.
- [ ] `cargo fmt --check` exits 0.
- [ ] `cargo clippy --all-targets --all-features -- -D warnings` exits 0.
- [ ] `cargo test --all` exits 0.
- [ ] No files outside the in-scope list are modified, except `plans/README.md`
  status if you update it.

## STOP conditions

Stop and report if:

- The live `/ask` handler no longer matches the excerpts above.
- The validation requires changing provider traits or response envelopes.
- Route-specific body limiting requires a broad router refactor.
- DB-backed tests cannot run because no pgvector Postgres is available; report
  the exact command and environment gap instead of weakening the tests.

## Maintenance notes

This plan does not solve abuse control by itself. A later production plan can
add rate limiting, per-key quotas, or authentication policy for `/ask`. Reviewers
should make sure this plan's guard runs before both `retrieve_context` and
`llm.answer`.
