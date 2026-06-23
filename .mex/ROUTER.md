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
last_updated: 2026-06-23
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
- Webmention receiving (always on) + sending (opt-in via `INKWELL_WEBMENTION_SEND=true`)
- Railway production deployment (auto-deploy on main push)
- Docker Compose local stack (migrate → seed → serve)
- `inkwell author` CLI for local authoring against a remote or local server
- `inkwell import` CLI for bulk import from Markdown files

**Not yet built:**

- Slug rename / redirect handling (slugs are currently immutable after creation)
- Media/image upload (notes are Markdown text only)
- Multi-user / per-user auth (single shared API key today)

**Known issues:**

- `set_document_status` (`publish`/`unpublish`) does not bump `version` or `updated_at` — status changes are invisible to the `If-Match` concurrency guard

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
