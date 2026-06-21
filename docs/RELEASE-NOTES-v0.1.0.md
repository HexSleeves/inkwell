# Inkwell v0.1.0

First tagged release of Inkwell — an open, API-first Markdown publishing
platform implemented as a Rust (Tokio/Axum/SQLx/Postgres) service. Authors write
Markdown, the API persists it, and Inkwell renders clean, fast public web pages.
Self-hostable via the bundled Docker Compose recipe.

## Shipped surface

### Authoring & content API

JSON write API guarded by a shared `INKWELL_API_KEY` (constant-time compare,
fail-closed when unset):

- `POST /documents`, `GET /documents`, `GET /documents/:slug`
- `PATCH /documents/:slug`, `PUT /documents/:slug`, `DELETE /documents/:slug`
- `POST /documents/:slug/publish`, `POST /documents/:slug/unpublish`
- `inkwell author` CLI (`new` / `push` / `publish`) drives the API from local
  Markdown files (ADR 0008, Option A — subcommands on the `inkwell` binary).

### Public web surface

- `GET /` + `GET /page/:page` — paginated index of published documents
- `GET /:slug` — rendered document page (Comrak render, Ammonia sanitize)
- `GET /tags`, `GET /tags/:tag`, `GET /tags/:tag/page/:page` — tag browse
- `GET /search` — content search
- `GET /health` — DB-aware health (`{"status":"ok","db":"up"}`)

### SEO & discovery

- `GET /feed.xml` (Atom) and `GET /sitemap.xml`, both emitting absolute URLs
  derived from `INKWELL_SITE_URL`.
- Sitemap generation is bounded to protect the server under large corpora.

### Caching

- Conditional-GET (ETag / `If-None-Match`) on the public read routes so
  unchanged pages, feed, and sitemap return `304`.

### Security hardening

- Fail-closed auth: every write route returns `401` when no API key is
  configured or the header does not match (`src/http/auth.rs`).
- HTML security headers including a nonce-based Content-Security-Policy.
- Request body size cap with a unified oversize error.
- Secret redaction in the Config Debug output.
- UTF-8-safe excerpt truncation on char boundaries (no panics).

### Operations

- `docker compose up --build` brings up app + Postgres 17; the app runs
  `inkwell db migrate && inkwell serve` on start.
- Reproducible staging runbook in `docs/STAGING.md` (deploy / verify / redeploy
  / teardown + secret handling).
- CI requires the DB-backed contract tests to run (`INKWELL_REQUIRE_DB_TESTS=1`).

## Validation

Release checklist (`docs/RELEASE-CHECKLIST.md`) run on this build:

- `cargo fmt --check` — pass
- `cargo clippy --all-targets --all-features -- -D warnings` — pass
- `cargo test --all` against a real Postgres (`INKWELL_REQUIRE_DB_TESTS=1`) —
  43 tests passed, 0 skipped
- `cargo build --release --bin inkwell` — pass
- `docker compose up --build` staging smoke — `/health` ok, create → publish →
  public render, `feed.xml`/`sitemap.xml` carry absolute `INKWELL_SITE_URL`
  URLs, unauthenticated write rejected with `401`

## Known gaps

These are intentional v0.1 scope cuts with follow-on work already sequenced:

- **No browser authoring UI yet.** Authoring is API + the `inkwell author` CLI
  (ADR 0008). A web authoring experience is the main product gap.
- **Single shared API key.** All writers share one `INKWELL_API_KEY`. Scoped,
  per-author tokens with an audit trail are deferred to v0.2 (ADR 0009,
  Option B — sequenced after the CLI MVP).
- **No media/image upload** — planned after scoped tokens in the v0.2 roadmap.
