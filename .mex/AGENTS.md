---
name: agents
description: Always-loaded project anchor. Read this first. Contains project identity, non-negotiables, commands, and pointer to ROUTER.md for full context.
last_updated: 2026-06-23
---

# Inkwell

## What This Is

An open, API-first Markdown publishing platform (digital garden) implemented as a Rust service, with a REST API, MCP server for AI agent access, Postgres full-text + vector search, and an HTML public site.

## Non-Negotiables

- Never write raw SQLx queries outside of `src/db/` — no DB access in handlers or garden
- Every read endpoint must apply `Visibility` (Public vs All) from `authenticate(...).await.is_some()` — never hardcode `status = 'published'` in a handler
- Every write endpoint must call `require_principal`, then `require_scope` (create→`write`, publish→`publish`), then enforce ownership ATOMICALLY by passing `owner_filter(&principal)` into the mutating query (`WHERE … AND ($n IS NULL OR owner_id = $n)`; admin → `None` = no constraint). A non-owner matches no row → 404 — never a separate check-then-write. `create` stamps `owner_id`; audit the action with the resolved `Principal`. Admin-only surfaces also require `principal.has(Scope::Admin)`
- Post-write side-effects (edge persist, embedding index, re-render) are always best-effort — `if let Err(e) = ... { tracing::warn!(...) }`, never 500 a write that succeeded
- Never print secrets in logs — `Config::Debug` redacts all keys; don't add `%config.api_key` to tracing spans

## Commands

- Serve: `cargo run --bin inkwell -- serve`
- Migrate: `cargo run --bin inkwell -- db migrate`
- Test: `cargo test --all` (set `DATABASE_URL` for DB-backed tests)
- Lint: `cargo clippy --all-targets --all-features -- -D warnings`
- Format: `cargo fmt --check`
- Build: `cargo build --release --bin inkwell`
- MCP: `cargo run --bin inkwell -- mcp` (requires running serve)

## Scaffold Growth

After meaningful work, run GROW:

- Ground: what changed in reality?
- Record: update `ROUTER.md` and relevant `context/` files
- Orient: create or update a `patterns/` runbook if this can recur
- Write: bump `last_updated` on changed scaffold files and run `mex log` when rationale matters

The scaffold grows from real work, not just setup. See the GROW step in `ROUTER.md` for details.

## Navigation

At the start of every session, read `ROUTER.md` before doing anything else.
For full project context, patterns, and task guidance — everything is there.
