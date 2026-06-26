---
name: router
description: Session bootstrap and navigation hub. Read at the start of every session before any task. Contains project state, routing table, and behavioural contract.
edges:
  - target: context/architecture.md
    condition: when working on system design, integrations, or understanding how components connect
  - target: context/stack.md
    condition: when working with specific technologies, libraries, or making tech decisions
  - target: context/conventions.md
    condition: when writing new code, reviewing code, or unsure about project patterns
  - target: context/decisions.md
    condition: when making architectural choices or understanding why something is built a certain way
  - target: context/setup.md
    condition: when setting up the dev environment or running the project for the first time
  - target: context/ai.md
    condition: when working on semantic search, RAG, embeddings, or the /ask endpoint
  - target: patterns/INDEX.md
    condition: when starting a task — check the pattern index for a matching pattern file
last_updated: 2026-06-25
---

# Session Bootstrap

If you haven't already read `AGENTS.md`, read it now — it contains the project identity, non-negotiables, and commands.

Then read this file fully before doing anything else in this session.

## Current Project State

**Working:**

- Full REST CRUD API for documents (`/documents`, `/documents/{slug}`, publish/unpublish)
- Backlinks (`/documents/{slug}/backlinks`) + garden graph (`/graph`, `/documents/{slug}/graph`)
- Wikilink rendering with `[[links]]` and `![[embeds]]` transclusion (depth + cycle bounded)
- Postgres full-text search (`/search`) via `search_vector` generated column (migration 0008)
- Semantic search + RAG via pgvector (`note_chunks` table, migration 0009); `/ask` endpoint
- MCP server (`inkwell mcp`) with 5 tools: `search_notes`, `read_note`, `list_notes`, `create_note`, `update_note`
- HTML public site: index, paginated, document pages, tag index/pages, RSS feed, sitemap
- Optimistic concurrency via `version` + `If-Match` (409 Conflict on stale writes)
- Scoped author tokens (ADR 0009, plan 023): authors, `documents.owner_id`, durable write-audit trail (slice 1); token issuance via admin routes (`/admin/tokens` create/list/revoke), token resolution in `authenticate()` → `Principal`, per-author audit attribution, `inkwell author token` CLI (slice 2); **enforcement (slice 3)** — mutations require the right scope (`write` create/update/delete, `publish` publish/unpublish; missing scope → 403) and ownership is enforced **atomically** in the write (`owner_filter` → `WHERE … AND owner_id = $owner`; non-owner → 0 rows → 404, no TOCTOU; admin bypasses); `create` stamps `owner_id`; draft READ requires the `read` scope (admin implies all). **Per-owner draft read isolation (slice 3b)** — the binary `Visibility` (Public/All) is reworked into an owner-aware filter (`Public` / `Owner(id)` / `All`) derived once in `resolve_visibility` (`src/http/api.rs`) and threaded through every read surface (documents, links/backlinks/graph, RAG chunks, garden embeds, `/ask`+`/related`), so a `read`-scoped author sees only their OWN drafts + all published; admin sees all; the public sees published-only. Public HTML/RSS surfaces stay pinned to `Visibility::Public`; ownership lands in the same `WHERE` as the read (no TOCTOU). **Tightening (slice 4):** `documents.owner_id` is `NOT NULL` (migration 0017; DB default kept as a safety net); `INKWELL_MCP_KEY` **retired** — the MCP server authenticates with `INKWELL_API_KEY` set to a scoped token. Shared `INKWELL_API_KEY` is the admin/bootstrap key.
- Request correlation IDs (CIL-125): `request_id` middleware (`src/http/request_id.rs`) honours a well-formed inbound `X-Request-Id` (else mints a UUID v4), stashes it in a task-local, adds it to the `TraceLayer` span (every log line carries `request_id`), echoes it on every response via `X-Request-Id`, and includes it in the JSON error envelope (`error.requestId`) so a user-reported error traces to its logs
- Pragmatic write rate limiting (CIL-128): a process-wide GCRA limiter (`governor`) throttles mutations (`POST`/`PUT`/`PATCH`/`DELETE`) + `/ask`, bucketed by **validated principal** (keying reuses `authenticate`, so a forged/invalid credential can't mint a bucket) else client IP, configurable via `INKWELL_WRITE_RATE_LIMIT` req/min (default 60; `0` disables); over-limit → `429` + `Retry-After`. Forwarded-header trust for IP keying is opt-in (`INKWELL_TRUST_FORWARDED_HEADERS`, default off). Reads + public HTML never throttled. Middleware in `src/http/rate_limit.rs`.
- Webmention receiving (always on) + sending (opt-in via `INKWELL_WEBMENTION_SEND=true`)
- Railway production deployment (auto-deploy on main push)
- Docker Compose local stack (migrate → seed → serve)
- `inkwell author` CLI for local authoring against a remote or local server
- `inkwell import` CLI for bulk import from Markdown files

**Not yet built:**

- Media upload UI — file-picker / drag-drop page that POSTs to `/media` and inserts
  the returned URL (API shipped in #28; UI deferred — see `plans/027-media-upload-ui.md`)
- Browser auth/login UI — the session backend shipped flag-gated (#29, ADR 0010,
  `INKWELL_BROWSER_LOGIN` default off); the actual login HTML page is still deferred

**Recently shipped (this run):**

- Request correlation IDs — `X-Request-Id` middleware: span + response header +
  error-envelope `requestId` (CIL-125).
- Pragmatic write rate limiting — `governor` GCRA middleware on mutations + `/ask`,
  keyed by principal/credential else IP, `INKWELL_WRITE_RATE_LIMIT` env (default 60),
  429 + Retry-After; reads/public site unthrottled (CIL-128).
- Slug rename + 301 alias redirect — mutable slug, owner-enforced, no draft leak
  (ADR 0011, migration 0021, PR #31).
- Media upload/serve API — `POST /media` + `GET /media/{id}` (PR #28).
- Owner-aware draft reads — slice 3b, completes ADR 0009 (PR #30).
- Flag-gated browser session login — ADR 0010, default off (PR #29).

**Known issues:**

- None currently tracked.

## Routing Table

Load the relevant file based on the current task. Always load `context/architecture.md` first if not already in context this session.

| Task type | Load |
|-----------|------|
| Understanding how the system works | `context/architecture.md` |
| Working with a specific technology or crate | `context/stack.md` |
| Writing or reviewing code | `context/conventions.md` |
| Making a design decision | `context/decisions.md` |
| Setting up or running the project | `context/setup.md` |
| Working on AI/embeddings/RAG/ask | `context/ai.md` |
| Adding a new HTTP route | `patterns/add-endpoint.md` |
| Adding a new DB migration or column | `patterns/database-migration.md` |
| Adding a new MCP tool | `patterns/add-mcp-tool.md` |
| Debugging a failing request | `patterns/debug-request-failures.md` |
| Any specific task | Check `patterns/INDEX.md` for a matching pattern |

## Behavioural Contract

For every task, follow this loop:

1. **CONTEXT** — Load the relevant context file(s) from the routing table above. Check `patterns/INDEX.md` for a matching pattern. If one exists, follow it. Narrate what you load: "Loading architecture context..."
2. **BUILD** — Do the work. If a pattern exists, follow its Steps. If you are about to deviate from an established pattern, say so before writing any code — state the deviation and why.
3. **VERIFY** — Load `context/conventions.md` and run the Verify Checklist item by item. State each item and whether the output passes. Do not summarise — enumerate explicitly.
4. **DEBUG** — If verification fails or something breaks, check `patterns/INDEX.md` for a debug pattern. Follow it. Fix the issue and re-run VERIFY.
5. **GROW** — After meaningful work, run this binary checklist:
   - **Ground:** What changed in reality? Name the changed behavior, system, command, dependency, or workflow.
   - **Record:** If project state changed, update the "Current Project State" section above. If documented facts changed, update the relevant `context/` file surgically.
   - **Orient:** If this task can recur and no pattern exists, create one in `patterns/` using `patterns/README.md`, then add it to `patterns/INDEX.md`. If a pattern exists but you learned a gotcha, update it.
   - **Write:** Bump `last_updated` in every scaffold file you changed. If the why matters, run `mex log --type decision "<what changed and why>"` or `mex log "<note>"`.
