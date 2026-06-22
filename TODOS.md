# TODOS

Deferred items captured during the /plan-eng-review of the digital-garden design
(2026-06-21). See `~/.gstack/projects/HexSleeves-inkwell/lecoqjacob-main-design-20260621-193844.md`.

## FTS before RAG (search recall)
- **What:** Add Postgres full-text search as an intermediate recall upgrade between P1's substring (ILIKE) search and P3's semantic embeddings.
- **Why:** MCP agents and the public search get poor recall from substring matching. FTS is a cheap, boring win that materially improves agent + human search before the heavier embedding work lands.
- **Pros:** Big recall improvement for low cost; no new infra (built into Postgres); helps the MCP search tool feel good immediately.
- **Cons:** A bit of schema/query work (tsvector column + GIN index + query rewrite); two search paths until embeddings subsume one.
- **Context:** Surfaced by Codex outside-voice #11. P1 ships substring search by decision; this is the next recall step. Start in `src/http/search.rs` + a migration adding a `tsvector` column.
- **Depends on / blocked by:** P1 (search exists); ideally before or alongside P3.

## SQLite mode (answer the "won't run Postgres" objection)
- **What:** An optional SQLite-backed mode so a user can run Inkwell with zero external database.
- **Why:** Premise 5 says one-command, no-ops self-host is existential, and Codex #18 flags that Postgres-first contradicts it. SQLite mode (or a hosted default) is the strongest answer to "cool, but I won't run Postgres."
- **Pros:** Removes the biggest adoption blocker for the target audience; trivial single-binary deploy.
- **Cons:** Large — dual database support across the SQLx layer; pgvector/embeddings have no SQLite equivalent (AI features would be Postgres-only); migration parity burden.
- **Context:** Surfaced by Codex #18. Big architectural fork — evaluate against "hosted demo + dead-simple Docker" as the lighter alternative first.
- **Depends on / blocked by:** None technically; decide after P1 validates demand.

## Old-slug redirect / alias table
- **What:** When a note's slug changes, 301 the old URL to the new one and resolve historical `[[old-slug]]` references.
- **Why:** Slug-first resolution (decided) means a rename can orphan external inbound links and old wikilinks. An alias table preserves link integrity across renames.
- **Pros:** No broken external links or dead wikilinks after a rename; better SEO continuity.
- **Cons:** A `slug_aliases` table + redirect route + resolution fallback; small ongoing complexity.
- **Context:** Surfaced by Codex #2/#4. The P1 bounded re-render fixes internal stored hrefs; this covers external/historical references. Start with a `slug_aliases(old_slug, note_id)` table + a redirect in the slug router.
- **Depends on / blocked by:** P0 identity + P1 wikilinks.
