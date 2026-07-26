# Changelog

All notable changes to Inkwell are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versioning follows
[Semantic Versioning](https://semver.org/) as described in
[`docs/RELEASES.md`](docs/RELEASES.md).

Full prose notes per release live in `docs/RELEASE-NOTES-<version>.md`.

## [Unreleased]

_Nothing yet._

## [0.2.0] — 2026-07-25

The v0.2 slice: **scoped author tokens**, a **browser authoring UI**, **media
upload and hosting**, **first-class observability**, and **tested backup +
restore**. Full notes:
[`docs/RELEASE-NOTES-v0.2.0.md`](docs/RELEASE-NOTES-v0.2.0.md).

### Added

#### Scoped API tokens (CYP-41, ADR 0009)

- Per-author, individually revocable bearer tokens (`ink_<prefix>_<secret>`).
  Only a SHA-256 digest is stored; the secret is shown once at creation.
- Scopes `read` / `write` / `publish` / `admin`, enforced on every mutating
  route. A `write` token cannot mint or revoke tokens.
- Ownership model: `documents.owner_id` is `NOT NULL`; non-admin principals only
  act on documents they own. Owner-scoped draft read visibility.
- Revocation takes effect on the next request — the auth path rejects `revoked`
  rows before any scope check.
- Token management API (`GET/POST /admin/tokens`,
  `POST /admin/tokens/{prefix}/revoke`, `POST /admin/tokens/prune`) and the
  `inkwell author token {create,list,revoke}` CLI.
- Write audit trail: create/update/delete/publish/unpublish emit rows attributed
  to the acting principal, so residual shared-key use is distinguishable.

#### Authoring web UI (CYP-42, ADR 0010)

- Server-rendered browser authoring surface behind `INKWELL_BROWSER_LOGIN=true`
  (default off): `GET /login`, `GET /editor`, `GET /editor/new`,
  `GET /editor/{slug}`, plus `POST /auth/login` and `POST /auth/logout`.
- Session-cookie auth for browser requests (`sessions` table, migration 0020),
  mapped onto the same scoped-token principal and scope checks as the API — the
  UI has no privileged back door.
- Media picker/upload page at `GET /media/new` (same flag); insert an uploaded
  image into the editor without leaving the browser.
- With the flag off, every route above falls through to the public `/{slug}`
  handler — no auth or authoring surface is exposed.

#### Media upload and hosting (CYP-45, ADR 0013)

- `POST /media`, `GET /media/{id}`, `DELETE /media/{id}` — content-addressed
  image upload and serving.
- Pluggable storage behind `INKWELL_MEDIA_BACKEND`: `local` filesystem (default,
  rooted at `INKWELL_MEDIA_DIR`) or `postgres` (`media_blobs` table).
- Magic-byte content sniffing verifies the declared type; the configurable size
  cap is `INKWELL_MEDIA_MAX_BYTES` (default 5 MiB, ceiling 256 MiB).
- Immutable cache headers plus `ETag` on served media; `304` on revalidation.
- `inkwell author upload <file>` prints the `/media/{id}` URL to embed.

#### Observability (CYP-46, ADR 0012)

- Structured request logs (JSON by default, `INKWELL_LOG_FORMAT=pretty` for
  local dev), filtered with `INKWELL_LOG` (falls back to `RUST_LOG`).
- Every request carries `X-Request-Id`, echoed in response headers, log lines,
  and `error.requestId`.
- Prometheus metrics at `GET /metrics`, off unless
  `INKWELL_METRICS_ENABLED=true`; optional bearer auth via
  `INKWELL_METRICS_TOKEN`. Secrets are redacted from logs and config dumps.
- Split health probes: `GET /healthz` (liveness, no DB) and `GET /readyz`
  (readiness, DB-aware). `GET /health` remains an alias of `/readyz`.
- Guide: [`docs/OBSERVABILITY.md`](docs/OBSERVABILITY.md).

#### Also in this release

- **Digital garden** — `[[wikilinks]]`/`![[embeds]]` with persisted edges,
  backlinks panel, per-note and whole-garden graphs, growth stages.
- **Reader pages** (always on) — `GET /notes` filterable index, `GET /graph`
  interactive whole-garden graph, `GET /settings` about-this-garden.
- **AI / RAG** — `GET or POST /ask` (pgvector retrieval + Claude synthesis) and
  `GET /documents/{slug}/related`, with embedding-provider provenance and a
  configurable similarity floor (`INKWELL_MIN_SIMILARITY`).
- **Full-text search** — Postgres `search_vector` generated column.
- **MCP server** — `inkwell mcp` exposes five tools over stdio.
- **Draft preview links** — `POST /documents/{slug}/preview-tokens` mints a
  shareable `pvw_…` token; anonymous `GET /documents/{slug}/preview?token=…`.
- **Slug rename with 301 alias redirect** (ADR 0011).
- **Archive navigation** — `GET /archive`, `GET /archive/{year}/{month}`, and
  prev/next bars on document pages.
- **Configurable site metadata** — `INKWELL_SITE_TITLE`,
  `INKWELL_SITE_DESCRIPTION`, `INKWELL_SITE_AUTHOR`, `INKWELL_CUSTOM_CSS_URL`.
- **Webmentions** — receiving always on; sending opt-in via
  `INKWELL_WEBMENTION_SEND`, with SSRF defenses.
- **Backup / restore** — `inkwell backup` writes one bundle (database + media +
  manifest); `inkwell restore` loads it, refusing a non-empty target without
  `--overwrite` (ADR 0014, [`docs/BACKUP-RESTORE.md`](docs/BACKUP-RESTORE.md)).
- **Obsidian vault import** — `inkwell import`.
- **Write rate limiting** — GCRA per principal or client IP, `429` +
  `Retry-After`, configurable via `INKWELL_WRITE_RATE_LIMIT`.
- **Docs site** — MkDocs Material, HTTP API reference, and an OpenAPI 3.1
  contract at [`docs/openapi.yaml`](docs/openapi.yaml).

### Changed

- `GET /health` is now an alias of `/readyz`; orchestrators should prefer the
  explicit `/healthz` (liveness) and `/readyz` (readiness) probes.
- Default log output is JSON. Set `INKWELL_LOG_FORMAT=pretty` to restore the
  human-readable formatter.
- Media rows written before migration 0025 keep their inline `media.data` bytes
  and are still served from it; no blob rewrite is required to upgrade.
- Markdown rendering and sanitization run on a blocking thread pool, so large
  documents no longer stall the async runtime.
- Site CSS is served from `GET /assets/site.css` instead of inline `<style>`, so
  the CSP no longer needs `style-src 'unsafe-inline'`.

### Removed

- **`INKWELL_MCP_KEY`** — the standalone MCP credential is gone. The MCP server
  now authenticates with a scoped token supplied via `INKWELL_API_KEY`.

### Deprecated

- Using the shared `INKWELL_API_KEY` as an everyday authoring credential. It is
  **not removed** and is still required at boot (fail-closed), but it is now
  positioned as a bootstrap / break-glass admin key. Mint scoped tokens for
  routine authoring, automation, and agent access.

### Migration notes (v0.1 → v0.2)

**Existing deployments keep working without configuration changes**, with one
exception (`INKWELL_MCP_KEY`, below).

1. **Apply migrations.** `inkwell db migrate` runs `0005`–`0025`. The bootstrap
   admin author is seeded automatically and every existing document is
   backfilled to own it (`0015`–`0017`), so `documents.owner_id NOT NULL` is
   satisfied without manual work. Take a backup first — see
   [`docs/BACKUP-RESTORE.md`](docs/BACKUP-RESTORE.md).
2. **Your current `INKWELL_API_KEY` still authenticates.** When it matches, the
   request acts as a synthetic `bootstrap-admin` with implicit `admin` scope that
   bypasses ownership checks. No downtime, no client changes required.
3. **Breaking — MCP clients.** If you set `INKWELL_MCP_KEY`, that variable is no
   longer read. Mint a scoped token and pass it as `INKWELL_API_KEY` to
   `inkwell mcp`:
   ```bash
   inkwell author token create --name mcp --scopes read,write
   ```
4. **Move clients onto scoped tokens.** Mint per-client tokens and reserve the
   shared key for provisioning and emergency recovery. Shared-key use is
   auditable (logged as `bootstrap-admin`) so you can track what is left.
5. **Media persistence.** The default `local` backend writes to
   `INKWELL_MEDIA_DIR` (`./data/media`). This directory **must be persistent** —
   the bundled Compose stack mounts the `inkwell-media` volume at
   `/app/data/media`. Set `INKWELL_MEDIA_BACKEND=postgres` if you would rather
   keep blobs in the database.
6. **Optional — enable metrics.** `/metrics` stays unregistered unless
   `INKWELL_METRICS_ENABLED=true`. When you enable it on a reachable host, also
   set `INKWELL_METRICS_TOKEN`.
7. **Optional — enable the authoring UI.** `INKWELL_BROWSER_LOGIN=true` turns on
   `/login`, `/editor`, and `/media/new`. Left off, those routes 404 and the
   deployment behaves exactly as v0.1 did.

## [0.1.0] — 2026-06-20

First tagged release: API-first Markdown publishing over Axum/SQLx/Postgres —
document CRUD with publish/unpublish, a shared-key-guarded write API, the
`inkwell author` CLI, server-rendered public pages, tag browse, search, RSS and
sitemaps, and a Docker Compose recipe. Full notes:
[`docs/RELEASE-NOTES-v0.1.0.md`](docs/RELEASE-NOTES-v0.1.0.md).

[Unreleased]: https://github.com/HexSleeves/inkwell/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/HexSleeves/inkwell/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/HexSleeves/inkwell/releases/tag/v0.1.0
