# Plan 013: Design spike — tags / collections direction (post-ship)

> **Executor instructions**: Follow the steps in order. Run the **Drift check** FIRST — it is load-bearing for this plan, because the spike's original premise (tags unbuilt) is already FALSE at HEAD. Read the "Why this matters" and "Current state" sections completely before writing anything. This is a DESIGN/DOCUMENTATION plan: you create exactly one new markdown file (`docs/adr/0007-tags-and-collections.md`) and update `plans/README.md`. Touch NO source code. Run every **Verify** command. Obey every **STOP condition**. When done, update the status row in `plans/README.md`.
> **Drift check (run first)**: `git diff --stat 6bf6a27..HEAD -- docs/adr/0006-content-discovery-and-seo.md src/sitemap.ts src/db/migrations.ts src/db/documents.ts src/api.ts src/pages.ts src/search.ts` — if any in-scope/grounding file changed, compare the Current-state excerpts below to live code; on mismatch, STOP and re-derive what is actually shipped before writing the ADR.

## Status

- **Priority**: P2
- **Effort**: L
- **Risk**: LOW
- **Depends on**: 001 (paginated index discipline) — already shipped; see note below
- **Category**: direction
- **Planned at**: commit `6bf6a27`, 2026-06-19

## Why this matters

> **CRITICAL PLANNING NOTE — READ BEFORE EXECUTING.** This plan was specified as a green-field design spike for tags/collections, on the assumption (from ADR 0006's original body) that tags were _deferred and unbuilt_. **That assumption is stale at HEAD `6bf6a27`.** Tags AND search both already SHIPPED (migration `0003`, a full data-access layer, API validation, tag listing pages, sitemap fold-in, and a separate `src/search.ts`). ADR 0006 itself carries an "Update — phases 2 & 3 shipped (CYP-23)" section documenting this. **Therefore the spike does not design a feature from scratch — it captures the direction decisions that are still genuinely open** now that the v0.x array-backed model is in production. The concrete cost this plan addresses: the shipped model made a deliberate, reversible bet (`tags text[]` over a join table) whose migration trigger and trade-offs are scattered across code comments and one ADR section; there is no single decision record for "when and how do we graduate to a referential tag entity, per-tag feeds, and `tsvector` search?" ADR 0007 makes that the durable, reviewable record and slices the follow-up work, so the next implementer starts from a decision instead of re-litigating it.

## Current state

The files in play and their role:

- **`docs/adr/0006-content-discovery-and-seo.md`** — the discovery/SEO ADR. Its original "Decision" deferred phase 2 (tags) and phase 3 (search); a later "Update — phases 2 & 3 shipped (CYP-23)" section records that both shipped and the schema bets made. ADR 0007 is the follow-up this ADR's Consequences anticipated.
- **`src/db/migrations.ts`** — ordered, immutable `{ id, name, up, down }` migrations applied in `id` order, recorded in `schema_migrations`. Never edit a shipped migration — add the next zero-padded id.
- **`src/db/documents.ts`** — data-access layer; already exposes tag reads/writes (`listDocumentsByTag`, `countDocumentsByTag`, `listPublishedTags`, `TagCount`).
- **`src/api.ts`** — JSON API; already validates/normalizes a `tags` field on POST/PATCH.
- **`src/pages.ts`** — HTML frontend; already imports `listDocumentsByTag`, `countDocumentsByTag`, `listPublishedTags`, `TagCount` to render tag pages.
- **`src/sitemap.ts`** — already folds `/tags` + per-tag URLs into the sitemap.
- **`src/search.ts`** — full-text-ish search (`ILIKE`) already shipped (phase 3).

### Verified excerpts at HEAD `6bf6a27`

ADR 0006 originally deferred tags (`docs/adr/0006-content-discovery-and-seo.md:21`):

```
Ship phases **1, 4, and 5** now; defer **2 (tags)** and **3 (search)** to a
tracked follow-up because both require a schema migration and broader API
surface than the SEO core.
```

…and described the seam (`docs/adr/0006-content-discovery-and-seo.md:78-81`):

```
follow-up; the sitemap and index are designed to fold in tag URLs/collection
pages without restructuring (a note marks the extension point in
`src/sitemap.ts`).
```

**But the same ADR records the shipped reality (`docs/adr/0006-content-discovery-and-seo.md:86-107`)** — reproduce this in ADR 0007's context so a reviewer sees the bet was already made:

```
## Update — phases 2 & 3 shipped (CYP-23)

The deferred phases later shipped under CYP-23. The schema decisions:

### Phase 2 — tags

- **Storage: a `tags text[]` column on `documents`, not a `document_tags` join
  table** (migration `0003`, GIN-indexed). Tags are a small, unordered set that
  is always read and written with the document and never queried independently of
  it, so an array keeps reads single-row and writes atomic with no join. A join
  table buys a referential tag entity (descriptions, colours, rename-with-history)
  that v0.x does not need; it can be migrated to later. Existing rows backfill to
  `'{}'` so none is left NULL.
- **Grammar:** tags reuse the slug grammar (lowercase alphanumerics, single
  hyphens) so each is a safe `/tags/:tag` URL segment. The API normalizes on
  write (trim, lower-case, de-dupe, order-preserved) and caps at 20 per document /
  50 chars each.
```

Migration `0003` is shipped (`src/db/migrations.ts:79-98`):

```
const addDocumentTags: Migration = {
  id: '0003',
  name: 'add_document_tags',
  up: `
    ALTER TABLE documents
      ADD COLUMN tags text[] NOT NULL DEFAULT '{}';
    CREATE INDEX documents_tags_idx ON documents USING gin (tags);
  `,
  down: `
    DROP INDEX documents_tags_idx;
    ALTER TABLE documents DROP COLUMN tags;
  `,
};

/** All migrations, in apply order. */
export const MIGRATIONS: readonly Migration[] = [
  createDocuments,
  addDocumentStatus,
  addDocumentTags,
];
```

The `status` column comment confirms the additive-migration discipline this ADR relies on (`src/db/migrations.ts:51-52`):

```
 * The status is a CHECK-constrained text column rather than a native enum so a
 * future value can be added with a plain migration (no `ALTER TYPE` dance).
```

The sitemap seam is already filled, not pending (`src/sitemap.ts:13-15`):

```
 * documents are listed, so the sitemap never leaks a draft. The `/tags` index
 * and one `/tags/:tag` URL per published tag are listed alongside documents.
 */
```

The DAL already aggregates the published tag set in application code because `pg-mem` lacks `unnest` (`src/db/documents.ts:407-413`):

```
 * List the distinct tags across all `published` documents, with a per-tag
 * document count, sorted by descending count then tag name.
 *
 * Postgres could do this with `unnest(tags)` + `GROUP BY`, but `pg-mem` (the
 * ... ) does not implement it, so we aggregate in
 * application code. The published tag set is small and this only backs the tags
 * index page and sitemap, so reading published rows' tag arrays and tallying
```

The API already validates/normalizes tags on create (`src/api.ts:329`, `src/api.ts:233-248`):

```
/** POST /documents — create a document from `{ title, bodyMarkdown, slug?, tags? }`. */
...
    const tag = raw.trim().toLowerCase();
    if (tag.length > MAX_TAG_LENGTH || !TAG_PATTERN.test(tag)) {
      throw new ApiError(
        400,
        `Tag "${raw}" must be lowercase alphanumerics separated by single hyphens (≤ ${MAX_TAG_LENGTH} chars).`,
      );
    }
    ...
  if (tags.length > MAX_TAGS) {
    throw new ApiError(400, `A document may have at most ${MAX_TAGS} tags.`);
  }
```

**Conclusion to encode in ADR 0007:** the spike's (a)–(e) design questions are already _answered and shipped_ for v0.x. ADR 0007 therefore (1) ratifies the shipped model as the accepted decision, citing where each piece lives, and (2) records the still-open direction: the array→join-table migration trigger, per-tag Atom feeds, and the `tsvector` search upgrade. It must NOT propose re-implementing what exists.

### Exemplar to follow

Match the prose, heading shape, and Prettier formatting of an existing accepted ADR — model after **`docs/adr/0006-content-discovery-and-seo.md`** (Status / Context / Decision / Safety / Consequences / Alternatives considered). ADR 0007 has **no tests**: it is a documentation artifact. The only gate is `pnpm run format:check` (Prettier-formatted markdown). There is no existing `plans/README.md` — see Step 4.

## Commands you will need

| Purpose | Command | Expected |
| --- | --- | --- |
| Drift check | `git diff --stat 6bf6a27..HEAD -- <paths above>` | empty (or reconcile) |
| Format check | `pnpm run format:check` | exit 0 |
| Format fix | `pnpm run format` (if `format:check` fails) | exit 0, file reformatted |
| Status (no src touched) | `git status --porcelain` | only the two in-scope files |

(Typecheck/lint/test/build are intentionally NOT in this table — this plan touches no `.ts` and produces no executable change.)

## Scope

**In scope:**

- `docs/adr/0007-tags-and-collections.md` _(create)_ — the design/decision record.
- `plans/README.md` _(create if absent, else edit)_ — add/update the status row for Plan 013.

**Out of scope (do NOT touch — one line each):**

- `src/db/migrations.ts` — the spike must NOT land a migration; any new migration is a follow-up plan.
- `src/db/documents.ts` — tag DAL already exists; no code changes in a design spike.
- `src/api.ts` — tag validation already exists; do not alter the contract here.
- `src/pages.ts`, `src/sitemap.ts`, `src/feed.ts`, `src/search.ts` — read surfaces already shipped; per-tag feeds are a follow-up, not this plan.
- `docs/adr/0006-content-discovery-and-seo.md` — do not rewrite the shipped ADR; ADR 0007 supersedes only the _open_ parts and cross-references it.

## Git workflow

- Branch: `advisor/013-spike-tags-collections`.
- Conventional-commit example: `docs(adr): add 0007 tags & collections direction record`.
- Do NOT push, commit to `main`, or open a PR unless the operator explicitly asks. Leave the branch checked out with the two files staged-or-unstaged for the operator to review.

## Steps

1. **Run the drift check** (command above). Tags/search are expected to be shipped at `6bf6a27`; if the excerpts above no longer match live code, STOP and re-derive the actual shipped surface before writing — the ADR must describe reality, not this plan's snapshot.
   **Verify**: `git diff --stat 6bf6a27..HEAD -- docs/adr/0006-content-discovery-and-seo.md src/db/migrations.ts src/db/documents.ts src/api.ts src/sitemap.ts src/search.ts` → empty, or differences reconciled in your notes.

2. **Create the branch**: `git switch -c advisor/013-spike-tags-collections`.
   **Verify**: `git branch --show-current` → `advisor/013-spike-tags-collections`.

3. **Write `docs/adr/0007-tags-and-collections.md`** with the structure below (Prettier-clean markdown, wrap prose ~80 cols to match 0006). The ADR MUST contain these sections:

   - **Title + `Status: accepted`** — `# 0007 — Tags and collections`.
   - **Context** — state that phases 2 (tags) and 3 (search) shipped under CYP-23; cite the array-vs-join bet from migration `0003` and the additive-migration discipline (`status` CHECK column comment). Make explicit that this ADR is the durable home for the tags/collections _direction_, ratifying the shipped v0.x model and recording what remains open.
   - **Decision** — five sub-sections (a)–(e), each phrased as "shipped: X; rationale; open: Y":
     - **(a) Data model** — _Shipped:_ `tags text[]` on `documents` (migration `0003`, GIN-indexed), NOT a join table. _Rationale (justify, do not re-decide):_ small unordered set, read/written with the document, single-row reads, atomic writes, no join. _Open:_ graduate to a `tags` table + `documents_tags` join when a referential tag entity is needed (descriptions, colors, rename-with-history, cross-document tag rename). **Recommend the join table as the eventual normalized target and justify it**, but record that v0.x stays on the array until a trigger fires (see Open questions).
     - **(b) Tag-input contract** — _Shipped:_ POST/PATCH `/documents` accept a `tags` array of slug-grammar strings; the API normalizes (trim, lowercase, de-dupe, order-preserved) and caps at `MAX_TAGS` (20) / `MAX_TAG_LENGTH` (50). Validation mirrors the slug grammar (`TAG_PATTERN`). Cite `src/api.ts:233-248`. _Open:_ none for v0.x; note collections (named ordered groupings distinct from flat tags) are unspecified.
     - **(c) Read surfaces** — _Shipped:_ tag listing via `listDocumentsByTag`/`countDocumentsByTag` and a distinct published-tag set via `listPublishedTags` (`TagCount`); `/tags` index + `/tags/:tag` pages render in `src/pages.ts` reusing the index renderer; a `?tag=` style filter is served through the DAL. Cite `src/db/documents.ts:354-428`. _Open:_ pagination of very large per-tag result sets (currently bounded by the same `PAGE_SIZE` discipline from Plan 001).
     - **(d) Sitemap/feed fold-in** — _Shipped:_ `src/sitemap.ts` lists `/tags` + one `/tags/:tag` per published tag (no `<lastmod>`). Cite `src/sitemap.ts:13-15`. _Open:_ **per-tag Atom feeds** (`/tags/:tag/feed.xml`) are NOT shipped — list as an open question with the cost/benefit (subscribe-to-a-topic vs. feed-endpoint proliferation + crawl budget).
     - **(e) Supporting index** — _Shipped:_ GIN index `documents_tags_idx` over `tags` backs `tag = ANY(tags)`. Tie to Plan 001's index discipline. Note the `pg-mem` caveat: index/EXPLAIN behavior is not testable in `pg-mem`, so correctness is asserted via DAL/migration tests, never query-planner output; the published-tag aggregation runs in application code because `pg-mem` lacks `unnest`. Cite `src/db/documents.ts:407-413`.
   - **Migration sketch (PSEUDOCODE / SQL ONLY — DO NOT APPLY)** — show what the _future_ join-table migration `0004` would look like as illustrative SQL inside a fenced block, plus the `listDocuments` filter extension as pseudocode. Prefix the block with a bold "NOT APPLIED — illustrative" warning. This is the only migration content allowed and it lives only in the ADR text.
   - **Safety** — note tag reads are gated to `published` (no draft leak), tags are XML/HTML-escaped on every discovery surface, and the slug grammar keeps `/tags/:tag` URL-safe.
   - **Consequences** — array model keeps v0.x simple; the migration to a join table is reversible and additive (matches the established discipline); per-tag feeds and `tsvector` search remain deferred.
   - **Follow-up plans (decomposition)** — enumerate 2–4 implementation plans with rough effort. Because tags already shipped, frame these as the _remaining_ direction work:
     - _Plan A — array→join-table migration + DAL (Effort M):_ migration `0004` adding `tags`/`documents_tags`, dual-read shim, backfill; only when a trigger fires.
     - _Plan B — referential tag metadata + rename-with-history (Effort M):_ depends on A; tag descriptions/colors and a rename that updates all documents atomically.
     - _Plan C — per-tag Atom feeds + nav (Effort S):_ `/tags/:tag/feed.xml` reusing `src/feed.ts`; independent of A.
     - _Plan D — `tsvector` search upgrade (Effort M):_ replace the `ILIKE` fallback once tests run against real Postgres (per ADR 0006's recorded migration path); independent of A.
   - **Open questions** — explicit section. Include at minimum: (1) what concrete signal triggers the array→join-table migration (tag count? need for tag metadata? rename-with-history demand?); (2) do per-tag Atom feeds justify the endpoint proliferation; (3) are "collections" (curated, named, possibly ordered sets) a distinct first-class concept from flat tags, and if so do they want their own table now; (4) is the data-model graduation a call that needs product input (see STOP).
   - **Alternatives considered** — at least: join table from day one (rejected for v0.x), native enum/lookup for tags (rejected; array is simpler), `unnest`+`GROUP BY` for the tag set (rejected due to `pg-mem`).

   **Verify**: `pnpm run format:check` → exit 0. If it fails, run `pnpm run format` and re-check.

4. **Update `plans/README.md`** (the `plans/` directory currently contains only plan files and no README). If `plans/README.md` does not exist, create it with a markdown status table header `| Plan | Title | Priority | Effort | Status |` and add the Plan 013 row. If it exists, add/update the Plan 013 row to `Done`. Keep it Prettier-clean.
   **Verify**: `pnpm run format:check` → exit 0; `grep -c '013' plans/README.md` → ≥ 1.

5. **Confirm no source files changed.**
   **Verify**: `git status --porcelain` → lists ONLY `docs/adr/0007-tags-and-collections.md` and `plans/README.md` (no `src/` paths).

## Test plan

None — this is a design artifact, not executable code. The "test" is editorial completeness, machine-checked only for formatting:

- No new `*.test.ts`, no `pnpm test` requirement.
- Model the document's tone/structure after `docs/adr/0006-content-discovery-and-seo.md`.
- The regression this plan guards against (a re-litigation of a settled decision) is handled by the ADR explicitly ratifying the shipped model and citing file:line, so a future reader does not re-design from scratch.
- Verification command: `pnpm run format:check` → exit 0.

## Done criteria

- [ ] `docs/adr/0007-tags-and-collections.md` exists and contains: `Status: accepted`, Decision sub-sections (a)–(e), the NOT-APPLIED migration sketch, a Follow-up plans list of 2–4 items with effort, an Open questions section, and Alternatives considered.
- [ ] The ADR ratifies the _shipped_ array-backed model (does not propose rebuilding tags) and records only the genuinely open direction work.
- [ ] `pnpm run format:check` → exit 0.
- [ ] No out-of-scope files modified: `git status --porcelain` shows only the ADR and `plans/README.md`.
- [ ] `plans/README.md` row for Plan 013 updated to `Done`.
- [ ] Branch `advisor/013-spike-tags-collections` checked out; nothing pushed/committed unless the operator asked.

## STOP conditions

- **STOP if the drift check shows tags/search are NOT shipped at the executing HEAD** (i.e. migration `0003` / `src/search.ts` / the ADR 0006 "Update" section are absent). That would mean the tree regressed or you are on an unexpected commit; re-derive the actual state before writing — the ADR must describe reality.
- **STOP and ask the operator if the array→join-table data-model decision cannot be finalized without product input.** This is the spike's designated unknown: if choosing the migration _trigger_ (when to graduate) requires roadmap/product signal you do not have, do NOT invent a trigger — capture the full trade-off (array simplicity + atomic writes vs. join-table referential metadata + rename-with-history + tag pages-with-descriptions) in Open questions and flag it for product.
- **STOP if you find yourself editing any `src/` file.** This plan lands zero code. Any schema/API/page change is a separate follow-up plan (A–D above).
- **STOP if `pnpm run format:check` cannot be made to pass** after `pnpm run format` — investigate before declaring done.

## Maintenance notes

- **What future changes interact:** the array→join-table migration (Plan A) will touch `src/db/migrations.ts` (new `0004`), `src/db/documents.ts` (dual-read during transition), and every tag read surface; ADR 0007's migration sketch is the reference for that work. Per-tag feeds (Plan C) interact with `src/feed.ts` and `src/server.ts` routing. The `tsvector` upgrade (Plan D) is blocked on the test harness moving off `pg-mem` to real Postgres — that harness change is itself a prerequisite worth its own plan.
- **What a reviewer should scrutinize:** that ADR 0007 does NOT contradict ADR 0006's shipped "Update" section; that the migration sketch is clearly marked NOT-APPLIED; that the array→join trigger is stated as an open question (not silently decided); and that no executable code changed.
- **Deferred follow-ups:** collections-as-a-distinct-concept (vs. flat tags) is intentionally left open — if product wants curated/ordered sets, that is a larger data-model decision than the tag graduation and deserves its own ADR.
