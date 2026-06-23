---
name: setup
description: Dev environment setup and commands for inkwell. Load when setting up the project or when environment issues arise.
triggers:
  - "setup"
  - "install"
  - "environment"
  - "getting started"
  - "how do I run"
  - "local development"
  - "docker"
  - "migrate"
  - "seed"
edges:
  - target: context/stack.md
    condition: when specific technology versions or library details are needed
  - target: context/architecture.md
    condition: when understanding how components connect during setup
  - target: patterns/database-migration.md
    condition: when running or troubleshooting migrations (especially pgvector extension)
  - target: patterns/debug-request-failures.md
    condition: when the server starts but requests fail (auth, DB connectivity)
  - target: patterns/database-migration.md
    condition: when adding or running migrations
last_updated: 2026-06-23
---

# Setup

## Prerequisites

- Rust (2024 edition) — `rustup update stable`
- PostgreSQL with **pgvector extension** — required for migration 0009 (`vector(1024)` column)
- Docker + Docker Compose (optional, for the one-command local stack)
- `DATABASE_URL` environment variable — postgres DSN, e.g. `postgres://user:pass@localhost:5432/inkwell`

## First-time Setup

1. `cp .env.example .env` — set at minimum `INKWELL_API_KEY` (writes fail closed without it)
2. `export DATABASE_URL=postgres://user:pass@localhost:5432/inkwell` (or set in `.env`)
3. `cargo run --bin inkwell -- db migrate` — runs all migrations including pgvector schema
4. `cargo run --bin inkwell -- seed` — optional, populates interlinked sample notes (idempotent)
5. `cargo run --bin inkwell -- serve` — starts HTTP server on `HOST:PORT` (default `0.0.0.0:3000`)

**Docker Compose shortcut** (fully seeded in one command):
```bash
cp .env.example .env   # set INKWELL_API_KEY (and INKWELL_MCP_KEY for MCP)
docker compose up --build
```
Compose runs migrate → seed → serve automatically once Postgres is healthy.

## Environment Variables

**Required:**
- `DATABASE_URL` — PostgreSQL connection DSN (server only; not needed by `inkwell author`)

**Required for writes to succeed:**
- `INKWELL_API_KEY` — bearer token for authoring API; absent = all writes return 401

**Optional:**
- `PORT` — HTTP listen port (default `3000`)
- `HOST` — HTTP listen host (default `0.0.0.0`)
- `INKWELL_MCP_KEY` — separate bearer token for MCP access; can be granted/revoked independently
- `INKWELL_SITE_URL` — absolute base URL for feed/sitemap/metadata (e.g. `https://myblog.com`)
- `INKWELL_API_URL` — base URL the `inkwell author` CLI targets (default: derived from HOST:PORT)
- `VOYAGE_API_KEY` — Voyage AI key for real embeddings; absent = `MockEmbedder` (index still populated)
- `ANTHROPIC_API_KEY` — Anthropic key for `/ask` synthesis; absent = endpoint returns "AI not configured"
- `INKWELL_LLM_MODEL` — Claude model for synthesis (default: `claude-opus-4-8`)
- `INKWELL_WEBMENTION_SEND` — set to `true` to enable outbound Webmention sends (default off)
- `RUST_LOG` — log level for `tracing-subscriber` (e.g. `inkwell=debug,tower_http=info`)

## Common Commands

- `cargo run --bin inkwell -- serve` — start HTTP server
- `cargo run --bin inkwell -- db migrate` — run pending migrations
- `cargo run --bin inkwell -- seed` — seed sample notes (idempotent)
- `cargo run --bin inkwell -- mcp` — start MCP server over stdio (requires running HTTP server)
- `cargo build --release --bin inkwell` — release build
- `cargo fmt --check` — format check
- `cargo clippy --all-targets --all-features -- -D warnings` — lint (CI gate)
- `cargo test --all` — run all tests; DB-backed integration tests skip if `DATABASE_URL` not set
- `INKWELL_REQUIRE_DB_TESTS=1 cargo test --all` — fail fast if `DATABASE_URL` missing

## Common Issues

**pgvector extension missing:** Migration 0009 fails with `type "vector" does not exist`. Fix: `CREATE EXTENSION IF NOT EXISTS vector;` as a superuser on the target database before running migrations.

**`INKWELL_API_KEY` not set:** Server starts but all write requests (POST/PATCH/PUT/DELETE) return 401. Set the key in `.env` or shell env.

**SQLx compile errors / `DATABASE_URL` missing at compile time:** Use `SQLX_OFFLINE=true cargo build` to use cached query metadata from `.sqlx/`. Or set `DATABASE_URL` before building.

**MCP server can't connect:** `inkwell mcp` needs a running HTTP server. Check `INKWELL_API_URL` points to it and `INKWELL_MCP_KEY` matches what the server has as `INKWELL_MCP_KEY`.

**Tests skip DB coverage:** Export `DATABASE_URL` before running, or set `INKWELL_REQUIRE_DB_TESTS=1` to surface the skip as a failure.
