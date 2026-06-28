---
name: stack
description: Technology stack, library choices, and reasoning. Load when working with specific technologies or making decisions about libraries and tools.
triggers:
  - "library"
  - "package"
  - "dependency"
  - "which tool"
  - "technology"
  - "crate"
  - "cargo"
edges:
  - target: context/decisions.md
    condition: when the reasoning behind a tech choice is needed
  - target: context/conventions.md
    condition: when understanding how to use a technology in this codebase
  - target: context/ai.md
    condition: when working with Voyage AI or Anthropic API
last_updated: 2026-06-28
---

# Stack

## Core Technologies

- **Rust 2024 edition** — primary language; `cargo build --release --bin inkwell`
- **Tokio** — async runtime with `rt-multi-thread`, `macros`, `net`, `signal`, `time` features
- **Axum 0.8** — HTTP framework; `json` feature; routes in `src/http/router.rs`
- **SQLx 0.8** — async PostgreSQL driver; compile-time query checking; `runtime-tokio-rustls`, `postgres`, `uuid`, `time`, `migrate` features
- **PostgreSQL** — sole database; pgvector extension required for `vector(1024)` column (migration 0009)

## Key Libraries

- **`comrak`** (not pulldown-cmark) — Markdown → HTML; used in `src/rendering/markdown.rs`; Comrak chosen for CommonMark compliance + GitHub-flavored Markdown support
- **`ammonia`** (not manual sanitization) — HTML sanitizer applied after Comrak render in `src/rendering/sanitize.rs`; prevents XSS from user-supplied Markdown
- **highlight.js** (client-side) — syntax highlighting via CSS class names emitted by `src/rendering/highlight.rs`; no server-side rendering
- **`rmcp`** — MCP (Model Context Protocol) server framework; `server`, `transport-io`, `macros` features; used in `src/mcp/mod.rs` with `#[tool_router]` / `#[tool]` macros
- **`schemars`** — JSON Schema generation for MCP tool argument types; paired with `#[derive(JsonSchema)]`
- **`thiserror`** — error type derivation; `DbError` in `src/db/documents.rs`, `AppError` in `src/error.rs`
- **`anyhow`** — error propagation in non-handler code (garden, AI layer, client, CLI)
- **`clap`** with `derive` feature — CLI argument parsing in `src/cli/args.rs`
- **`reqwest 0.13`** with `json`, `stream` features — HTTP client for `InkwellClient` and Voyage/Anthropic API calls
- **`tower-http`** — `CompressionLayer` + `TraceLayer` + `security_headers` middleware
- **`tracing` + `tracing-subscriber`** — structured logging; `env-filter` + `json` features; log level via `RUST_LOG`
- **`time 0.3`** — datetime types; `OffsetDateTime` used on `Document`; custom RFC3339 serde in `src/domain/document.rs`
- **`uuid`** with `v4`, `serde` features — document IDs
- **`sha2`** — SHA-256 used in `MockEmbedder` for deterministic embeddings
- **`subtle`** — constant-time comparison for API key auth in `src/http/auth.rs`
- **`unicode-normalization`** — used in slug generation (`src/domain/slug.rs`)

## What We Deliberately Do NOT Use

- No ORM (no Diesel, no SeaORM) — all DB access is raw `sqlx::query_as` / `sqlx::query` in `src/db/`; schema and column list are spelled out explicitly in every query
- No templating engine (no Tera, no Askama) — HTML pages are built with Rust string formatting in `src/views/`
- No Redis or secondary cache — PostgreSQL is the only data store; no application-level caching layer
- No LangChain or AI framework — Voyage and Anthropic are called directly via `reqwest` in `src/ai/voyage.rs` and `src/ai/claude.rs`

## Version Constraints

- Rust 2024 edition required (let-chain `if let … && let …` used in `src/http/api.rs`)
- pgvector Postgres extension must be installed before migration 0009 runs (`CREATE EXTENSION IF NOT EXISTS vector`)
- SQLx macro checking requires `DATABASE_URL` at compile time (or use `SQLX_OFFLINE=true` with cached query metadata)
