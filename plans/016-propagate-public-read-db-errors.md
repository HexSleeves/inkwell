# Plan 016: Propagate database errors on public read surfaces

> **Executor instructions**: Follow this plan step by step. Run every verification command and confirm the expected result before moving on. If a STOP condition occurs, stop and report instead of improvising. When done, update this plan's row in `plans/README.md`.
>
> **Drift check (run first)**: `git diff --stat 8bcd1ea..HEAD -- src/http/pages.rs src/http/feed.rs src/http/sitemap.rs src/http/search.rs src/error.rs tests`
> If any in-scope file changed, compare the excerpts below with live code before editing. On mismatch, treat it as a STOP condition.

## Status

- **Priority**: P1
- **Effort**: M
- **Risk**: MED
- **Depends on**: none
- **Category**: bug / observability
- **Planned at**: commit `8bcd1ea`, 2026-06-19

## Why this matters

Most public read handlers currently convert database failures into normal-looking responses: empty indexes/search results/feed/sitemap or a 404 document page. During a database outage, crawlers and users can receive `200 OK` empty content, and operators lose the difference between "no content" and "backend unavailable." The API path already has a clean `AppError` response model that logs database errors and emits a generic 500; the public HTML/XML handlers should use the same failure semantics.

## Current state

Files and roles:

- `src/http/pages.rs` — HTML index, document, tag index, and tag listing handlers. In scope.
- `src/http/feed.rs` — Atom feed handler. In scope.
- `src/http/sitemap.rs` — sitemap handler. In scope.
- `src/http/search.rs` — HTML/JSON search handler. In scope.
- `src/error.rs` — existing `AppError` response behavior. Read for pattern; edit only if needed for HTML/XML content-type handling.
- `tests/` — add regression coverage using the existing Axum `oneshot` style.

Current swallowed-error examples:

```rust
// src/http/pages.rs:29-48
match documents::get_document_by_slug(...).await {
    Ok(Some(document)) => (StatusCode::OK, Html(render_document_page(...))),
    _ => (
        StatusCode::NOT_FOUND,
        Html(render_not_found_page(state.config.site_url.as_deref())),
    ),
}
```

```rust
// src/http/pages.rs:92-116
let total = documents::count_documents(...).await.unwrap_or_default();
let docs = documents::list_documents(...).await.unwrap_or_default();
```

```rust
// src/http/feed.rs:13-22
let documents = documents::list_documents(...).await.unwrap_or_default();
```

```rust
// src/http/sitemap.rs:13-25
let documents = documents::list_documents(...).await.unwrap_or_default();
let tags = documents::list_published_tags(&state.pool).await.unwrap_or_default();
```

```rust
// src/http/search.rs:53-70
.await.unwrap_or_default()
```

Existing API error pattern:

```rust
// src/error.rs:63-70
Self::Db(DbError::Sqlx(error)) | Self::Database(error) => {
    tracing::error!(error = %error, "database error");
    json_error(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error.", None, None)
}
```

Repo conventions:

- Handlers use Axum extractors and return either `Result<Response, AppError>` for fallible API routes or `impl IntoResponse` for simple public routes.
- Keep user-facing error bodies generic; log details through `tracing::error!`.
- Existing verification commands are `cargo fmt --check`, `cargo clippy --all-targets --all-features -- -D warnings`, and `cargo test --all`.

## Commands you will need

| Purpose | Command | Expected on success |
|---|---|---|
| Format | `cargo fmt --check` | exit 0 |
| Lint | `cargo clippy --all-targets --all-features -- -D warnings` | exit 0, no warnings |
| Tests | `cargo test --all` | exit 0 |

## Scope

**In scope**:

- `src/http/pages.rs`
- `src/http/feed.rs`
- `src/http/sitemap.rs`
- `src/http/search.rs`
- `src/error.rs` only if a shared public error helper is needed
- `tests/*` for regression coverage

**Out of scope**:

- Changing route paths, response shapes for successful responses, or document visibility rules.
- Changing SQL queries for performance; this plan is error propagation only.
- Adding a new templated error page design beyond the existing not-found page.

## Git workflow

- Branch: `advisor/016-public-read-db-errors`
- Commit style observed in `git log`: conventional-ish prefixes, e.g. `fix(api): ...`, `feat(discovery): ...`.
- Do not push or open a PR unless instructed.

## Steps

### Step 1: Change public handlers to return errors instead of defaults

Update the public handlers so database failures become an error response:

- `document_page`: return 404 only for `Ok(None)`; return an internal error for `Err(error)`.
- `tags_index`: return an internal error for `Err(error)`, not `render_not_found_page`.
- `render_index` and `render_tag_listing`: replace both `unwrap_or_default()` calls with `?` or explicit error mapping.
- `feed`, `sitemap`, and `search`: replace `unwrap_or_default()` with propagated errors.

Preferred shape: make these handlers return `Result<impl IntoResponse, AppError>` or `Result<Response, AppError>` where Axum accepts it, and use `? `to convert `sqlx::Error` through `AppError::Database`.

**Verify**: `cargo fmt --check` -> exit 0.

### Step 2: Preserve intentional 404 behavior

Keep these cases as 404:

- A published document slug does not exist.
- An invalid or out-of-range index page.
- An invalid tag slug.
- A tag with zero published documents.
- An out-of-range tag page.

Do not convert valid empty content states into 500. A site with no published documents should still render the index, feed, and sitemap successfully.

**Verify**: `cargo clippy --all-targets --all-features -- -D warnings` -> exit 0.

### Step 3: Add regression tests

Add tests that prove a broken pool or closed pool produces 500 instead of 200/404 for at least:

- `GET /`
- `GET /feed.xml`
- `GET /sitemap.xml`
- `GET /search?q=hello&format=json`
- `GET /missing-slug`

Use the existing `tests/common/mod.rs` router helper as the structural pattern. If a test requires a pool that fails queries deterministically, create a helper that obtains a test pool and closes it with `pool.close().await` before issuing requests. If that proves unreliable, add a narrow injectable database error test around the handler boundary and STOP before broad refactoring.

**Verify**: `cargo test --all` -> exit 0, with the new regression tests running.

## Test plan

- Add a public-read error regression test file such as `tests/public_read_errors.rs`.
- Cover success-preserving behavior only if existing tests do not: empty site index/feed/sitemap should still return 200 when the DB is reachable.
- Run the full suite with `DATABASE_URL` set when available. In CI this already happens.

## Done criteria

- [ ] No `.await.unwrap_or_default()` remains in `src/http/pages.rs`, `src/http/feed.rs`, `src/http/sitemap.rs`, or `src/http/search.rs` for database calls.
- [ ] `GET /`, `/feed.xml`, `/sitemap.xml`, `/search?q=x&format=json`, and `/:slug` return 500 on DB query failure.
- [ ] Intentional not-found and empty-site cases still behave as before.
- [ ] `cargo fmt --check`, `cargo clippy --all-targets --all-features -- -D warnings`, and `cargo test --all` exit 0.
- [ ] No files outside Scope are modified, except `plans/README.md` status.

## STOP conditions

Stop and report if:

- Axum route typing forces a broad router rewrite.
- Tests cannot produce a deterministic database failure without introducing a new dependency.
- Any successful response body or route contract must change.

## Maintenance notes

Reviewers should focus on preserving 404 semantics while eliminating swallowed database errors. Future public read handlers should avoid `unwrap_or_default()` around database calls unless the fallback is explicitly a product decision and covered by a test.

