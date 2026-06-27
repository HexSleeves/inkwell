# Plan 040: Populate the empty AGENTS.md template

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `plans/README.md`.
>
> **Drift check (run first)**: `git diff --stat 0819727..HEAD -- AGENTS.md`
> If AGENTS.md changed since this plan was written, compare the "Current state"
> excerpt against the live file before proceeding.

## Status

- **Priority**: P1
- **Effort**: S
- **Risk**: LOW
- **Depends on**: none
- **Category**: dx
- **Planned at**: commit `0819727`, 2026-06-26

## Why this matters

`AGENTS.md` is described in its own frontmatter as the "always-loaded project anchor. Read this first." But every content section is an empty HTML comment template, and `last_updated` is the literal placeholder `[YYYY-MM-DD]`. A new contributor or AI agent that reads AGENTS.md first (as instructed) learns nothing — it must fall back to `README.md` and `.mex/ROUTER.md`. For a repo where AI agents execute plans, an accurate AGENTS.md is high-leverage: it is the first file loaded and sets the non-negotiables and exact commands.

## Current state

**`AGENTS.md`** (entire file — all sections are empty comment blocks):
```markdown
---
name: agents
description: Always-loaded project anchor. Read this first. ...
last_updated: [YYYY-MM-DD]
---

# [Project Name]

## What This Is
<!-- One sentence. What does this project do? ... -->

## Non-Negotiables
<!-- Hard rules the agent must never violate. ... -->

## Commands
<!-- The exact commands needed to work on this project. ... -->

## After Every Task
... (GROW section — keep as-is) ...

## Navigation
... (keep as-is) ...
```

The facts to fill in are already established in the repo:
- **Project description** — from `README.md:3`: "An open, API-first Markdown publishing platform implemented as a Rust service."
- **Commands** — from `.github/workflows/ci.yml` and `README.md:27-37` (verified exact forms below).
- **Non-Negotiables** — from `.mex/context/conventions.md` "Verify Checklist".

## Commands you will need

| Purpose   | Command                                                       | Expected on success |
|-----------|--------------------------------------------------------------|---------------------|
| Fmt       | `cargo fmt --all -- --check`                                 | exit 0              |
| Lint      | `cargo clippy --all-targets --all-features --locked -- -D warnings` | exit 0       |
| Fast test | `cargo nextest run --lib` (no DB needed)                    | all pass            |
| Full test | `cargo test --all` (DB-backed tests need `DATABASE_URL`)    | all pass            |
| Build     | `cargo build --release --bin inkwell --locked`              | exit 0              |

(These are the exact commands CI runs — see `.github/workflows/ci.yml`.)

## Scope

**In scope** (only file to modify):
- `AGENTS.md`

**Out of scope**:
- The `## After Every Task` (GROW) and `## Navigation` sections — keep them verbatim; they are correct
- `.mex/ROUTER.md`, `README.md` — do not edit
- The frontmatter `name` / `description` fields — keep; only update `last_updated`

## Git workflow

- Branch: `advisor/040-populate-agents-md`
- Commit: `docs(agents): populate project identity, non-negotiables, and commands`

## Steps

### Step 1: Replace the title and "What This Is"

Change `# [Project Name]` to `# Inkwell`.

Replace the `## What This Is` comment block with:
```markdown
## What This Is

Inkwell is an open, API-first Markdown publishing platform (a "digital garden")
implemented as a Rust service: Axum + SQLx + PostgreSQL, with a public HTML
site, a REST API, wikilink/backlink graph, Postgres FTS + pgvector semantic
search, and an MCP server for AI agents.
```

**Verify**: `grep -c "API-first Markdown" AGENTS.md` → 1

### Step 2: Fill in Non-Negotiables

Replace the `## Non-Negotiables` comment block with (drawn from `context/conventions.md` Verify Checklist):
```markdown
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
```

**Verify**: `grep -c "Raw SQL lives only" AGENTS.md` → 1

### Step 3: Fill in Commands

Replace the `## Commands` comment block with:
```markdown
## Commands

- Format:   `cargo fmt --all -- --check`
- Lint:     `cargo clippy --all-targets --all-features --locked -- -D warnings`
- Test (fast, no DB): `cargo nextest run --lib`
- Test (full): `cargo test --all`  — DB-backed contract tests need `DATABASE_URL`
  set (or `INKWELL_REQUIRE_DB_TESTS=1` to fail fast when it is missing).
- Build:    `cargo build --release --bin inkwell --locked`
- Run:      `cargo run --bin inkwell -- serve`  (migrate first: `... -- db migrate`)
- Local stack: `docker compose up`  (migrate → seed → serve)
```

**Verify**: `grep -c "cargo nextest run" AGENTS.md` → 1

### Step 4: Update last_updated

Change `last_updated: [YYYY-MM-DD]` to today's date in `YYYY-MM-DD` form. If you do not know the date, run `git log -1 --format=%cd --date=short` and use that.

**Verify**: `grep -c "YYYY-MM-DD" AGENTS.md` → 0

## Test plan

No code tests — this is documentation. Verification is the `grep` checks above plus a manual read confirming no `<!-- ... -->` template comment blocks remain in the three filled sections.

## Done criteria

- [ ] `grep -c "\[Project Name\]" AGENTS.md` → 0
- [ ] `grep -c "YYYY-MM-DD" AGENTS.md` → 0
- [ ] The three sections (What This Is, Non-Negotiables, Commands) contain real content, no `<!--` template comments
- [ ] `## After Every Task` and `## Navigation` sections unchanged
- [ ] No other files modified (`git status`)
- [ ] `plans/README.md` status row updated

## STOP conditions

- The commands in `.github/workflows/ci.yml` differ from those quoted above (CI changed). Use the CI file's actual commands and note the discrepancy.
- AGENTS.md has been populated already by someone else (drift). Report and stop.

## Maintenance notes

- When build/test/lint commands change, update AGENTS.md Commands and `.github/workflows/ci.yml` together — they must agree.
- AGENTS.md non-negotiables should mirror the Verify Checklist in `.mex/context/conventions.md`; if one changes, change both.
