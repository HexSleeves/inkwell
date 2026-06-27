# Inkwell

An open, API-first Markdown publishing platform implemented as a Rust service.
Write notes in Markdown, link them with `[[wikilinks]]`, publish through a REST
API or CLI, and browse the resulting digital garden at a public HTML site.

**Documentation site:** [hexsleeves.github.io/inkwell](https://hexsleeves.github.io/inkwell/)
(MkDocs Material — auto-deployed when docs files change on `main`).
Local preview: `pip install -r requirements-docs.txt && mkdocs serve`.

---

## Features

| Area | What's shipped |
|------|----------------|
| **REST API** | Full document CRUD, publish/unpublish, slug rename with 301 redirect, optimistic concurrency (`If-Match`/`ETag`) |
| **Digital garden** | `[[wikilinks]]` + `![[embeds]]`, backlinks panel, per-note graph, whole-garden graph, growth stage (seedling/budding/evergreen) |
| **Full-text search** | `GET /search?q=…` — Postgres `search_vector` generated column; JSON or HTML response |
| **AI / RAG** | `GET or POST /ask` — pgvector semantic retrieval + Claude synthesis; `GET /documents/{slug}/related` for similar notes |
| **MCP server** | `inkwell mcp` — 5 tools (`search_notes`, `read_note`, `list_notes`, `create_note`, `update_note`) over stdio; auth via scoped token |
| **Author CLI** | `inkwell author` — `new`, `push`, `publish`, `unpublish`, `upload`; `inkwell import` for bulk Markdown import |
| **Scoped tokens** | Per-author bearer tokens (`ink_<prefix>_<secret>`); scopes: `read`, `write`, `publish`, `admin`; `inkwell author token` CLI |
| **Media** | `POST /media` + `GET /media/{id}` — image upload stored as PostgreSQL `bytea`; `inkwell author upload <file>` CLI |
| **Draft preview** | `POST /documents/{slug}/preview-tokens` mints a shareable `pvw_…` token; anonymous `GET /documents/{slug}/preview?token=…` renders the draft |
| **Archive nav** | `GET /archive` lists year/month buckets; `GET /archive/{year}/{month}` shows paginated docs; prev/next bar on each document page |
| **Site metadata** | `INKWELL_SITE_TITLE`, `INKWELL_SITE_DESCRIPTION`, `INKWELL_SITE_AUTHOR`, `INKWELL_CUSTOM_CSS_URL` for brand/SEO configuration |
| **Webmentions** | Receiving always-on (`POST /webmention`); sending opt-in (`INKWELL_WEBMENTION_SEND=true`) |
| **Rate limiting** | GCRA per-principal (validated credential or client IP), configurable, 429 + `Retry-After` |
| **Request IDs** | Every request gets `X-Request-Id`; echoed in response headers and `error.requestId` |
| **HTML site** | Index, paginated, document, tag, search, archive, RSS feed, sitemap; Botanical Soft design |
| **Ops** | Docker Compose local stack, Railway auto-deploy, single-binary path, pgvector pg17 |

---

## Stack

- Rust 2024, Tokio, Axum, SQLx
- PostgreSQL 16/17 with pgvector
- Comrak + Ammonia (Markdown → safe HTML)
- Docker Compose / Railway

---

## Environment

Copy `.env.example` to `.env` before local development. `.env` is gitignored.

### Required

| Variable | Notes |
|----------|-------|
| `DATABASE_URL` | PostgreSQL connection string |
| `INKWELL_API_KEY` | Admin shared key. Required; server refuses to start without it. Sent as `X-Api-Key` header. |

### Optional — server behavior

| Variable | Default | Notes |
|----------|---------|-------|
| `PORT` | `3000` | Bind port. Do not set on Railway. |
| `HOST` | `0.0.0.0` | Bind address. |
| `INKWELL_SITE_URL` | _(none)_ | Canonical public URL for feed, sitemap, and Open Graph links. |
| `INKWELL_SITE_TITLE` | `Inkwell` | Brand name in header, `<title>`, og:site_name, and feed title. |
| `INKWELL_SITE_DESCRIPTION` | _(none)_ | Index page `<meta name="description">` and feed subtitle. |
| `INKWELL_SITE_AUTHOR` | _(none)_ | Default Atom feed author and JSON-LD author. |
| `INKWELL_CUSTOM_CSS_URL` | _(none)_ | Extra stylesheet injected on every public HTML page. |
| `INKWELL_API_URL` | `http://HOST:PORT` | Base URL the `inkwell author` CLI targets. |
| `INKWELL_WRITE_RATE_LIMIT` | `60` | Write rate limit (req/min). `0` disables. |
| `INKWELL_TRUST_FORWARDED_HEADERS` | `false` | Trust `X-Forwarded-For` for IP keying (set `true` only behind a trusted proxy). |
| `INKWELL_WEBMENTION_SEND` | `false` | Send outbound Webmentions on publish. |
| `INKWELL_BROWSER_LOGIN` | `false` | Enable flag-gated browser session login (`/auth/*`). |

### Optional — AI / semantic layer

| Variable | Notes |
|----------|-------|
| `ANTHROPIC_API_KEY` | Claude key for `/ask` synthesis. Without it, `/ask` returns an explanatory message. |
| `VOYAGE_API_KEY` | Voyage AI key for note embeddings (semantic search, `/related`). Without it, a deterministic mock embedder is used. |

See [`docs/DEPLOYMENT.md`](docs/DEPLOYMENT.md) for the full variable list, Railway setup, Docker Compose, single-binary, and reverse proxy guides. See [`docs/SITE-CONFIGURATION.md`](docs/SITE-CONFIGURATION.md) for the site branding / metadata variables in detail.

---

## Run

```bash
cp .env.example .env   # set INKWELL_API_KEY
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all
cargo build --release --bin inkwell
export DATABASE_URL=postgres://user:pass@localhost:5433/inkwell
cargo run --bin inkwell -- db migrate
cargo run --bin inkwell -- serve
```

`cargo test --all` runs without Postgres, but DB-backed contract tests are
skipped unless `DATABASE_URL` is set. Export it (or set
`INKWELL_REQUIRE_DB_TESTS=1`) when you want the full suite.

---

## Quickstart (Docker, seeded)

```bash
cp .env.example .env   # set INKWELL_API_KEY; see docs/QUICKSTART.md for MCP scoped-token setup
docker compose up
```

One command gives you a **populated** garden: `migrate → seed → serve`, planting
a handful of interlinked sample notes as published. Open <http://localhost:3000>,
click any note, and see its backlinks panel.

For the full AI walkthrough (MCP server, `/ask`, embeddings): see
[`docs/QUICKSTART.md`](docs/QUICKSTART.md).

---

## HTTP surface

Route stability is declared in [`docs/COMPATIBILITY.md`](docs/COMPATIBILITY.md).
The full reference with request/response shapes is in [`docs/API.md`](docs/API.md);
the machine-readable contract is at [`docs/openapi.yaml`](docs/openapi.yaml).

Route groups:

| Group | Routes |
|-------|--------|
| **Health** | `GET /health` |
| **Documents** | `POST/GET /documents`, `GET/PATCH/PUT/DELETE /documents/{slug}` |
| **State** | `POST /documents/{slug}/publish`, `POST /documents/{slug}/unpublish` |
| **Linked surfaces** | `GET /documents/{slug}/backlinks`, `GET /documents/{slug}/graph`, `GET /graph` |
| **Preview tokens** | `POST/GET /documents/{slug}/preview-tokens`, `DELETE /documents/{slug}/preview-tokens/{prefix}`, `GET /documents/{slug}/preview` |
| **AI** | `GET or POST /ask`, `GET /documents/{slug}/related` |
| **Search** | `GET /search` |
| **Media** | `POST /media`, `GET /media/{id}` |
| **Admin tokens** | `GET/POST /admin/tokens`, `POST /admin/tokens/{prefix}/revoke`, `POST /admin/tokens/prune` |
| **Webmention** | `POST /webmention` |
| **Public HTML** | `GET /`, `GET /page/{page}`, `GET /{slug}`, `GET /tags`, `GET /tags/{tag}`, `GET /tags/{tag}/page/{page}`, `GET /search`, `GET /archive`, `GET /archive/{year}/{month}`, `GET /archive/{year}/{month}/page/{page}` |
| **Feeds / sitemaps** | `GET /feed.xml`, `GET /sitemap.xml`, `GET /sitemap-static.xml`, `GET /sitemaps/documents/{page}`, `GET /sitemaps/tags/{page}` |

---

## Authoring

The `inkwell author` subcommands are the first-party way to write content. They
speak the authenticated HTTP write API (never the database directly), so they
work against any deployment — local or remote.

```bash
export INKWELL_API_KEY=your-write-key        # or a scoped token
export INKWELL_API_URL=https://blog.example.com   # or use --server

# Scaffold a new Markdown file (writes ./hello-world.md)
inkwell author new "Hello World" --tag rust --tag notes

# Create or update from file (POST on first push, PUT after)
inkwell author push hello-world.md

# Publish so it appears in the public list
inkwell author publish hello-world

# Take it back to draft
inkwell author unpublish hello-world

# Upload an image and get a /media/{id} URL to embed
inkwell author upload screenshot.png
# Prints: /media/550e8400-e29b-41d4-a716-446655440000
# Embed it in Markdown: ![alt](/media/550e8400-e29b-41d4-a716-446655440000)
```

Markdown front matter:

```yaml
---
title: Hello World
slug: hello-world      # optional; auto-derived from title
status: draft          # advisory; publish via CLI command
tags:
  - rust
---
```

For scoped token setup, MCP agent authoring, and bulk import: see
[`docs/AUTHORING.md`](docs/AUTHORING.md).

---

## Scoped tokens

The shared `INKWELL_API_KEY` is the admin key. For day-to-day authoring and
AI agents, mint a scoped token:

```bash
# Mint a write+publish token for your laptop
inkwell author token create --name laptop --scopes read,write,publish
# Prints ink_<prefix>_<secret> exactly once — store it securely

# List tokens
inkwell author token list

# Revoke a token
inkwell author token revoke <prefix>
```

Use the scoped token as `INKWELL_API_KEY` for the MCP server or `inkwell author`
CLI. Admin key is only needed for token management itself. See
[`docs/AUTHORING.md`](docs/AUTHORING.md).

---

## Draft preview

Share a rendered draft before publishing:

```bash
# Mint a preview token (admin or owner with write scope)
curl -X POST https://blog.example.com/documents/my-draft/preview-tokens \
  -H "X-Api-Key: $INKWELL_API_KEY" -H "Content-Type: application/json" -d '{}'

# Returns: {"token":"pvw_<prefix>_<secret>","prefix":"...","expiresAt":null}

# Share the URL — no auth header required
https://blog.example.com/documents/my-draft/preview?token=pvw_<prefix>_<secret>
```

See [`docs/AUTHORING.md`](docs/AUTHORING.md) and [`docs/API.md`](docs/API.md) for full details.

---

## API reference

See [`docs/API.md`](docs/API.md) for the full HTTP API reference including
authentication, error shapes, request/response examples, and all endpoints.
A machine-readable OpenAPI 3.1 contract is at [`docs/openapi.yaml`](docs/openapi.yaml).

---

## Compatibility

[`docs/COMPATIBILITY.md`](docs/COMPATIBILITY.md) declares which public behaviors
are **stability commitments** before v0.2 and which are flexible. Changes to
stable surfaces require a CHANGELOG migration note and a semver bump. The
contract-test suite in `tests/` covers all stable surfaces.

---

## Deployment

See [`docs/DEPLOYMENT.md`](docs/DEPLOYMENT.md) for the full production guide:
all environment variables, Docker Compose and single-binary paths, reverse
proxy/TLS setup, migration order, and secret handling.

```bash
# Railway (auto-deploy on push to main)
# See docs/RAILWAY.md

# Docker Compose
docker compose up --build

# Single binary
cargo build --release --bin inkwell
./target/release/inkwell db migrate
./target/release/inkwell serve
```

---

## Documentation site

Browsable docs live at
[hexsleeves.github.io/inkwell](https://hexsleeves.github.io/inkwell/)
(MkDocs Material — deployed via GitHub Actions when docs files change on `main`).

To preview locally:

```bash
pip install -r requirements-docs.txt
mkdocs serve   # http://127.0.0.1:8000
```

---

## Docs index

| Document | Purpose |
|----------|---------|
| [`docs/QUICKSTART.md`](docs/QUICKSTART.md) | One-command local demo + AI walkthrough |
| [`docs/AUTHORING.md`](docs/AUTHORING.md) | Author CLI, scoped tokens, media upload, draft preview, import |
| [`docs/SITE-CONFIGURATION.md`](docs/SITE-CONFIGURATION.md) | Site branding and metadata env vars |
| [`docs/API.md`](docs/API.md) | Full HTTP API reference |
| [`docs/openapi.yaml`](docs/openapi.yaml) | OpenAPI 3.1 machine-readable contract |
| [`docs/COMPATIBILITY.md`](docs/COMPATIBILITY.md) | Stability contracts |
| [`docs/DEPLOYMENT.md`](docs/DEPLOYMENT.md) | Production deployment guide |
| [`docs/RAILWAY.md`](docs/RAILWAY.md) | Railway-specific deployment |
| [`docs/STAGING.md`](docs/STAGING.md) | Staging environment |
| [`docs/BACKUP-RESTORE.md`](docs/BACKUP-RESTORE.md) | Backup and restore runbook |
| [`docs/RELEASES.md`](docs/RELEASES.md) | Release process and versioning |
| [`docs/RELEASE-CHECKLIST.md`](docs/RELEASE-CHECKLIST.md) | Pre-release gate checklist |
| [`docs/QA-MATRIX.md`](docs/QA-MATRIX.md) | v0.2 QA matrix |
| [`docs/CLOSEOUT.md`](docs/CLOSEOUT.md) | Professionalization project closeout |
