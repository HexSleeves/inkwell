# Plan 019: Bound sitemap generation before it becomes an unbounded read path

> **Executor instructions**: Follow this plan step by step. Run every verification command and confirm the expected result before moving on. If a STOP condition occurs, stop and report instead of improvising. When done, update this plan's row in `plans/README.md`.
>
> **Drift check (run first)**: `git diff --stat 8bcd1ea..HEAD -- src/http/sitemap.rs src/db/documents.rs src/http/router.rs tests testdata/contracts/sitemap.xml docs/adr/0006-content-discovery-and-seo.md`
> If any in-scope file changed, compare the excerpts below with live code before editing. On mismatch, treat it as a STOP condition.

## Status

- **Priority**: P2
- **Effort**: M
- **Risk**: MED
- **Depends on**: none
- **Category**: perf / SEO
- **Planned at**: commit `8bcd1ea`, 2026-06-19

## Why this matters

`GET /sitemap.xml` currently reads every published document with no limit and renders all document URLs plus tag URLs in one response. That is fine for a small blog, but it is an unbounded public request path and sitemap protocol has hard practical limits: large sites need multiple sitemap files or a sitemap index. This service already added pagination for public HTML pages; sitemap generation should get the same bounded treatment before content volume turns it into a memory, latency, and crawler reliability problem.

## Current state

```rust
// src/http/sitemap.rs:11-22
pub async fn sitemap(State(state): State<AppState>) -> impl IntoResponse {
    let base = normalize_site_url(state.config.site_url.as_deref());
    let documents = documents::list_documents(
        &state.pool,
        crate::domain::document::ListOptions {
            limit: None,
            offset: None,
            status: Some(crate::domain::document::DocumentStatus::Published),
        },
    )
    .await
    .unwrap_or_default();
```

```rust
// src/http/sitemap.rs:23-25
let tags = documents::list_published_tags(&state.pool)
    .await
    .unwrap_or_default();
```

`src/http/router.rs` currently exposes only one sitemap route:

```rust
// src/http/router.rs:21-23
.route("/feed.xml", get(feed::feed))
.route("/sitemap.xml", get(sitemap::sitemap))
.route("/search", get(search::search))
```

`docs/adr/0006-content-discovery-and-seo.md` says feed, sitemap, and search expose published content only, and search preserves the `ILIKE` parity path. Preserve that published-only invariant.

## Commands you will need

| Purpose | Command | Expected on success |
|---|---|---|
| Format | `cargo fmt --check` | exit 0 |
| Lint | `cargo clippy --all-targets --all-features -- -D warnings` | exit 0, no warnings |
| Tests | `cargo test --all` | exit 0 |

## Scope

**In scope**:

- `src/http/sitemap.rs`
- `src/http/router.rs` if adding sitemap part routes
- `src/db/documents.rs` only for narrow count/list helpers
- `tests/*`
- `testdata/contracts/sitemap.xml` only if existing tests require fixture updates
- `docs/adr/0006-content-discovery-and-seo.md` only for a short note if behavior changes from single sitemap to sitemap index

**Out of scope**:

- Changing document page URLs, tag URLs, or published-only sitemap visibility.
- Implementing full-text search improvements.
- Adding background sitemap generation.

## Git workflow

- Branch: `advisor/019-bound-sitemap-generation`
- Commit style: `fix(sitemap): bound sitemap generation` or similar.
- Do not push or open a PR unless instructed.

## Steps

### Step 1: Choose the bounded sitemap shape

Prefer the smallest compatible implementation:

- Keep `/sitemap.xml` as a sitemap index when total URLs exceed one page.
- Add one or more part routes, for example `/sitemap-pages-1.xml`, for document URLs.
- Keep home and `/tags`/per-tag URLs in a bounded static/discovery sitemap if they fit comfortably.

Use a conservative page size constant below the sitemap protocol maximum, such as `SITEMAP_MAX_URLS: u32 = 10_000`, so memory stays bounded and future growth has margin. If the maintainer wants to keep a single sitemap until 50,000 URLs, document that choice in a code comment and tests.

**Verify**: `cargo fmt --check` -> exit 0.

### Step 2: Add count-based routing logic

Use existing count helpers or add narrow helpers in `src/db/documents.rs`:

- count published documents
- count published tags, if tag URLs are included in generated limits

Do not fetch all documents just to decide whether to emit an index. Use counts first, then fetch a bounded page.

**Verify**: `cargo clippy --all-targets --all-features -- -D warnings` -> exit 0.

### Step 3: Add route(s) and tests

If part routes are added, update `src/http/router.rs`. Add integration or handler tests covering:

- Small site still returns a valid `/sitemap.xml` with home and document URLs.
- Large synthetic count/path uses bounded page size or emits a sitemap index.
- Published-only invariant remains true.
- Database errors are not swallowed if Plan 016 has already landed; if it has not, do not solve that here.

**Verify**: `cargo test --all` -> exit 0.

## Test plan

- Model tests after existing Axum request tests in `tests/api_contract.rs`.
- If creating enough rows for a large sitemap test is too slow, factor pure rendering/count-decision helpers and test those directly.
- Update `testdata/contracts/sitemap.xml` only if an existing fixture comparison fails and the new output is intentional.

## Done criteria

- [ ] `GET /sitemap.xml` no longer calls `list_documents` with `limit: None` for all published documents.
- [ ] Any new sitemap part route is registered in `src/http/router.rs`.
- [ ] Sitemap output remains XML with `application/xml; charset=utf-8`.
- [ ] Published-only sitemap visibility remains unchanged.
- [ ] `cargo fmt --check`, `cargo clippy --all-targets --all-features -- -D warnings`, and `cargo test --all` exit 0.

## STOP conditions

Stop and report if:

- The desired URL shape for sitemap indexes is a product/SEO decision not inferable from existing ADRs.
- A bounded implementation requires a broad DAL rewrite.
- Existing contract fixtures conflict with the chosen sitemap-index behavior and no refresh process is documented.

## Maintenance notes

Future additions such as author pages, archives, or media URLs must account for sitemap partitioning. Keep the published-only invariant from ADR 0006 unless a new ADR supersedes it.

