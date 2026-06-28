---
name: agents
description: Always-loaded project anchor. Read this first. Contains project identity, non-negotiables, commands, and pointer to ROUTER.md for full context.
last_updated: 2026-06-28
---

# Inkwell

## What This Is

Inkwell is an open, API-first Markdown publishing platform (a "digital garden")
implemented as a Rust service: Axum + SQLx + PostgreSQL, with a public HTML
site, a REST API, wikilink/backlink graph, Postgres FTS + pgvector semantic
search, and an MCP server for AI agents.

## Non-Negotiables

- Raw SQL lives only in `src/db/` — never in handlers (`src/http/`) or `src/garden.rs`.
- Every read that can expose draft content must derive `Visibility` from auth
  (`resolve_visibility`) — never hardcode `status = 'published'` or skip the filter.
- Writes enforce ownership atomically in the mutating query (`owner_filter`) —
  no check-then-write; non-owner → 0 rows → 404. Admin bypasses.
- Post-write side-effects (edges, embeddings, re-render) are best-effort: log a
  `tracing::warn` on failure, never 500 a write that already succeeded.
- Never commit secrets. `INKWELL_API_KEY` and AI keys come from the environment.
- All code must pass `cargo fmt --all -- --check` and
  `cargo clippy --all-targets --all-features --locked -- -D warnings`.

## Commands

- Format: `cargo fmt --all -- --check`
- Lint: `cargo clippy --all-targets --all-features --locked -- -D warnings`
- Check: `cargo check --all-targets`
- Test (fast, no DB): `cargo nextest run --locked --profile ci-fast --lib`
- Test (selected contracts, no DB): run the contract suite listed in `.github/workflows/ci.yml`
  with the same locked `ci-fast` profile.
- Test (full): `cargo test --all --locked` — DB-backed contract tests need
  `DATABASE_URL` set and CI sets `INKWELL_REQUIRE_DB_TESTS=1`.
- Build: `cargo build --release --bin inkwell --locked`
- Docker: `docker build -t inkwell:ci .`
- Run: `cargo run --bin inkwell -- serve` (migrate first: `cargo run --bin inkwell -- db migrate`)
- Local stack: `docker compose up` (migrate → seed → serve)

## After Every Task

After meaningful work, run GROW:

- Ground: what changed in reality?
- Record: update `.mex/ROUTER.md` and relevant `.mex/context/` files
- Orient: create or update a `.mex/patterns/` runbook if this can recur
- Write: bump `last_updated` on changed scaffold files and run `mex log` when rationale matters

## Navigation

At the start of every session, read `.mex/ROUTER.md` before doing anything else.
For full project context, patterns, and task guidance — everything is there.
