# Inkwell

An open, API-first Markdown publishing platform implemented as a Rust service.

## Stack

- Rust 2024
- Tokio + Axum
- SQLx + PostgreSQL
- Comrak + Ammonia
- Cargo + Docker Compose

## Environment

- Copy `.env.example` to `.env` before local development. `.env` is gitignored.
- `DATABASE_URL` required
- `PORT` default `3000`
- `HOST` default `0.0.0.0`
- `INKWELL_API_KEY` optional but writes fail closed when unset
- `INKWELL_SITE_URL` optional, used for absolute feed/sitemap/page metadata URLs
- `INKWELL_API_URL` optional, base URL the `inkwell author` CLI targets (defaults to `http://HOST:PORT`)
- `INKWELL_WRITE_RATE_LIMIT` optional, write rate limit in requests/minute (default `60`). Applied per authenticated principal/token, or per client IP when anonymous, to mutation routes (create/update/delete/publish/unpublish, `POST /media`, `/webmention`) and `/ask`. Reads and the public HTML site are never throttled. Over-limit requests get `429` with a `Retry-After` header. Set to `0` to disable.

## Run

```bash
cp .env.example .env
# Full integration tests require DATABASE_URL in your shell or .env.
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all
cargo build --release --bin inkwell
export DATABASE_URL=postgres://user:pass@localhost:5433/inkwell
cargo run --bin inkwell -- db migrate
cargo run --bin inkwell -- serve
```

`cargo test --all` stays runnable without Postgres, but the database-backed
contract tests are skipped unless `DATABASE_URL` is set. Export
`DATABASE_URL` before local verification when you want the API/SQL/migration
path covered, or set `INKWELL_REQUIRE_DB_TESTS=1` to make missing database
configuration fail fast.

## Quickstart (Docker, seeded)

```bash
cp .env.example .env   # set INKWELL_API_KEY (and INKWELL_MCP_KEY for the AI walkthrough)
docker compose up
```

One command gives you a **populated** garden: the app runs `migrate -> seed ->
serve`, planting a handful of interlinked sample notes (as published) into an
empty garden. Open <http://localhost:3000>, click any note, and see its "Linked
from" backlinks panel. From nothing to an AI agent reading, searching, and
editing your live garden: see [`docs/QUICKSTART.md`](docs/QUICKSTART.md).

## Docker Compose

```bash
cp .env.example .env
docker compose up --build
```

Set `INKWELL_API_KEY` in your shell or `.env` before starting Compose; the app refuses to start until it is set. The app runs `inkwell db migrate`, then `inkwell seed` (populates an empty garden with the bundled sample vault, idempotent), then `inkwell serve` once Postgres is healthy.

## Railway

Railway deployment is configured in `railway.json`. Create a Railway web service
from this repo, attach Railway PostgreSQL, reference the database
`DATABASE_URL`, set `INKWELL_API_KEY` and `INKWELL_SITE_URL`, then deploy. See
[`docs/RAILWAY.md`](docs/RAILWAY.md).

## Staging

See [`docs/STAGING.md`](docs/STAGING.md) for the reproducible staging deploy: secret handling, deploy/redeploy/teardown commands, and the release smoke check.

## HTTP surface

Preserved routes:

- `GET /health`
- `POST /documents`
- `GET /documents`
- `GET /documents/:slug`
- `PATCH /documents/:slug`
- `PUT /documents/:slug`
- `DELETE /documents/:slug`
- `POST /documents/:slug/publish`
- `POST /documents/:slug/unpublish`
- `GET /`
- `GET /page/:page`
- `GET /:slug`
- `GET /tags`
- `GET /tags/:tag`
- `GET /tags/:tag/page/:page`
- `GET /search`
- `GET /feed.xml`
- `GET /sitemap.xml`

## Authoring

The `inkwell author` subcommands are the first-party way to write content. They
speak the same authenticated HTTP write API above (never the database directly),
so they work against any deployment — local or remote.

Configuration is shared with the server:

- `INKWELL_API_KEY` — sent as the `X-API-Key` header on every write. Required.
- `INKWELL_API_URL` — the server base URL (e.g. `https://blog.example.com`).
  Falls back to `http://HOST:PORT`. Override per-invocation with `--server <url>`.

Documents are plain Markdown files with a small YAML front matter block:

```md
---
title: Hello World
slug: hello-world
status: draft
tags:
  - rust
  - notes
---

# Hello World

Body Markdown lives here.
```

`title` is required. `slug` is optional and defaults to the server's
slugification of the title. `status` is advisory metadata — publishing always
happens through an explicit command, never a file write. The body is capped at
256 KiB and rejected client-side before any request is sent.

Typical workflow:

```bash
export INKWELL_API_KEY=your-write-key
export INKWELL_API_URL=https://blog.example.com   # or use --server

# Scaffold a new Markdown file (writes ./hello-world.md):
inkwell author new "Hello World" --tag rust --tag notes

# Create or update the document from the file (POST on first push, PUT after):
inkwell author push hello-world.md

# Publish it so it appears in the public list and pages:
inkwell author publish hello-world

# Take it back to draft:
inkwell author unpublish hello-world
```

`push` decides between create and update by probing the slug with a `GET`. All
commands print a one-line result and fail with a clear, non-panicking message on
`401` (bad key), `404` (missing slug), oversize bodies, or validation errors.
