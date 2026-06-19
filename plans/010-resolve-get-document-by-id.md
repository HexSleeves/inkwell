# Plan 010: Resolve the getDocumentById export asymmetry

> **Executor instructions**: Follow the steps in order, top to bottom. Run every **Verify** command and confirm the expected result before moving on. Obey all STOP conditions — if one triggers, halt and ask the maintainer rather than improvising. This is the DEFAULT path (drop the unused public export). The ESCAPE HATCH (adding an HTTP route) requires explicit maintainer confirmation; do NOT do it unless told. When done, update the status row for plan 010 in `plans/README.md`.

> **Drift check (run first)**: `git diff --stat 6bf6a27..HEAD -- src/db/index.ts src/db/documents.ts src/db/documents.test.ts src/api.ts README.md` — if any in-scope file changed, compare the Current-state excerpts below against live code; on any mismatch, STOP and re-assess before editing.

## Status

- **Priority**: P3
- **Effort**: S
- **Risk**: LOW
- **Depends on**: none
- **Category**: tech-debt
- **Planned at**: commit `6bf6a27`, 2026-06-19

## Why this matters

`getDocumentById` is implemented in the DAL, re-exported as part of the curated public `inkwell/db` surface, and documented in the README, yet no HTTP route or page consumes it — only its own unit test does. This is either dead public API surface (which carries a maintenance/compat cost: every public export is an implicit promise) or a half-finished feature (a missing id-addressing route). The honest, cheapest resolution is to stop advertising it publicly while keeping the function available for internal use and tests, shrinking the public contract to what is actually wired up. This removes ambiguity for integrators reading the README and for future maintainers deciding whether the export is load-bearing.

## Current state

Files and their role:

- `src/db/documents.ts` — Data Access Layer (DAL); raw `pg` queries, no ORM. Defines `getDocumentById`, mirroring `getDocumentBySlug`.
- `src/db/index.ts` — curated public surface of the persistence layer. Header comment states test-only helpers are deliberately omitted; it re-exports `getDocumentById`.
- `README.md` — public docs; a table row documents `getDocumentById`.
- `src/api.ts` — JSON API handler; routes documents by **slug** only. There is no GET-by-id route.
- `src/db/documents.test.ts` — the ONLY consumer of `getDocumentById`. It imports the symbol directly from `./documents.js` (NOT via `./index.js` / `inkwell/db`), so dropping the public re-export does not break it.

Verified excerpts at HEAD `6bf6a27`:

`src/db/documents.ts` (lines 193–200):

```ts
/** Fetch a document by id, or `null` if none exists. */
export async function getDocumentById(db: Queryable, id: string): Promise<Document | null> {
  const result = await db.query<DocumentRow>(`SELECT ${RETURNING} FROM documents WHERE id = $1`, [
    id,
  ]);
  const row = result.rows[0];
  return row ? toDocument(row) : null;
}
```

`src/db/index.ts` (lines 14–30, the curated-surface header + the export block containing `getDocumentById` on line 21):

```ts
/**
 * Public surface of the persistence layer.
 *
 * Import database primitives from `inkwell/db` (or relative `./db/index.js`)
 * rather than reaching into individual modules. Test-only helpers are
 * deliberately not re-exported here.
 */

export { createPool, type Queryable } from './pool.js';
export { MIGRATIONS, type Migration } from './migrations.js';
export {
  ensureMigrationsTable,
  appliedMigrationIds,
  migrate,
  rollback,
  type RollbackOptions,
} from './migrate.js';
export {
  createDocument,
  getDocumentBySlug,
  getDocumentById,
  listDocuments,
  countDocuments,
  updateDocumentBySlug,
  setDocumentStatus,
  deleteDocumentBySlug,
  listDocumentsByTag,
  countDocumentsByTag,
  listPublishedTags,
  searchPublishedDocuments,
```

`README.md` (line 411, inside the DAL exports table):

```
| `getDocumentById(db, …)`      | Fetch by id, or `null`                          |
```

`src/db/documents.test.ts` (lines 3–17, import block — note it imports from `./documents.js`, line 17):

```ts
import {
  DuplicateSlugError,
  countDocumentsByTag,
  countSearchPublishedDocuments,
  createDocument,
  deleteDocumentBySlug,
  getDocumentById,
  getDocumentBySlug,
  listDocuments,
  listDocumentsByTag,
  listPublishedTags,
  searchPublishedDocuments,
  setDocumentStatus,
  ...
} from './documents.js';
```

Consumer grep at HEAD `6bf6a27` (`grep -rn "getDocumentById" src --include="*.ts"`) returned ONLY:

- `src/db/documents.ts:194` (definition)
- `src/db/index.ts:21` (re-export — to be removed)
- `src/db/documents.test.ts:9` and `:56` (the test, importing from `./documents.js`)

Exemplar/test pattern to follow if the escape hatch is chosen: existing `handleGet`-by-slug tests in `src/api.test.ts` (setup: `createMemoryDatabase()` + `migrate(db)`, then `handleApiRequest(db, { headers: { 'x-api-key': KEY }, ... }, { apiKey: KEY })`).

## Commands you will need

| Purpose        | Command                                   | Expected                          |
| -------------- | ----------------------------------------- | --------------------------------- |
| Install        | `pnpm install`                            | exit 0                            |
| Typecheck      | `pnpm run typecheck`                      | exit 0, no errors                 |
| Lint           | `pnpm run lint`                           | exit 0                            |
| Format check   | `pnpm run format:check`                   | exit 0                            |
| Tests (one)    | `pnpm test src/db/documents.test.ts`      | pass                              |
| Tests (all)    | `pnpm test`                               | all pass                          |
| Full gate      | `pnpm run ci`                             | exit 0                            |

## Scope

**In scope (DEFAULT path):**

- `src/db/index.ts` — remove the `getDocumentById,` line from the re-export block.
- `README.md` — remove the `getDocumentById(db, …)` table row.

**Out of scope (do NOT touch):**

- `src/db/documents.ts` — KEEP the `getDocumentById` function; it stays available for internal use and is imported by its test.
- `src/db/documents.test.ts` — the test imports from `./documents.js`, unaffected by the public-surface change; no edit needed.
- `src/api.ts` — only touched under the escape hatch, which requires maintainer confirmation.
- Any other DAL module — unrelated.

## Git workflow

1. Create a branch: `advisor/010-resolve-get-document-by-id`.
2. Make the edits.
3. Commit (conventional commits, scope `db` or `refactor`), e.g.:
   `refactor(db): drop unused getDocumentById from public surface`
4. Do NOT push, open a PR, or merge unless the operator explicitly asks.

## Steps

1. **Confirm there is no non-test consumer.** Run `grep -rn "getDocumentById" src --include="*.ts"`. The output MUST be exactly the four lines listed in Current state (definition in `documents.ts`, re-export in `index.ts`, two hits in `documents.test.ts`). If anything else imports it (a page, the API, the CLI, a feed/sitemap builder), STOP — it is not dead surface; re-assess with the maintainer.
   **Verify**: `grep -rn "getDocumentById" src --include="*.ts"` -> only `documents.ts`, `index.ts`, `documents.test.ts` appear.

2. **Confirm the test import path.** Run `grep -n "from '" src/db/documents.test.ts`. Confirm `getDocumentById` is imported via `} from './documents.js';` and NOT via `./index.js` or `inkwell/db`. If it is imported via the index/package path, STOP and instead update that test import to pull from `./documents.js` before removing the re-export.
   **Verify**: `grep -n "documents.js" src/db/documents.test.ts` -> shows the import line `} from './documents.js';`.

3. **Remove the public re-export.** In `src/db/index.ts`, delete the single line `  getDocumentById,` from the `export { ... } from './documents.js';` block (between `getDocumentBySlug,` and `listDocuments,`). Leave every other export untouched.
   **Verify**: `grep -n "getDocumentById" src/db/index.ts` -> no output (exit 1).

4. **Remove the README row.** In `README.md`, delete the table row `| \`getDocumentById(db, …)\` ... | Fetch by id, or \`null\` ... |` (around line 411). Do not disturb adjacent rows or table alignment.
   **Verify**: `grep -n "getDocumentById" README.md` -> no output (exit 1).

5. **Typecheck.** The function still exists in `documents.ts` and the test still imports it directly, so nothing should break.
   **Verify**: `pnpm run typecheck` -> exit 0, no errors.

6. **Run the DAL test, then full gate.**
   **Verify**: `pnpm test src/db/documents.test.ts` -> pass; then `pnpm run ci` -> exit 0.

## Test plan

No new tests are required for the DEFAULT path: the change only narrows the public re-export and docs. The regression guard for THIS finding is the existing `src/db/documents.test.ts`, which imports `getDocumentById` from `./documents.js` and exercises it (line 56: `const found = await getDocumentById(db, created.id);`) — it must continue to pass, proving the function remains usable internally after the export is dropped. Verification command: `pnpm test src/db/documents.test.ts` then `pnpm run ci`.

If (and only if) the maintainer chooses the ESCAPE HATCH instead, add `src/api.test.ts` cases modeled on the existing `handleGet`-by-slug tests: (a) 200 with the document body for an existing id; (b) 404 for an unknown id; (c) a draft document is hidden (404) for an unauthenticated request but visible for an authenticated one — matching `handleGet`'s draft-visibility gating.

## Done criteria

- [ ] `grep -rn "getDocumentById" src --include="*.ts"` shows only `documents.ts` (definition) and `documents.test.ts` (test) — no `index.ts` hit.
- [ ] `grep -n "getDocumentById" README.md` returns nothing.
- [ ] `src/db/documents.ts` still defines `getDocumentById` (function NOT removed).
- [ ] `pnpm run typecheck` -> exit 0.
- [ ] `pnpm run lint` -> exit 0.
- [ ] `pnpm run format:check` -> exit 0.
- [ ] `pnpm test` -> all pass.
- [ ] `pnpm run ci` -> exit 0.
- [ ] No out-of-scope files modified — `git status` shows only `src/db/index.ts`, `README.md`, and `plans/README.md` changed.
- [ ] `plans/README.md` status row for plan 010 updated to done/completed.

## STOP conditions

- **A non-test consumer imports `getDocumentById`** (a page, `src/api.ts`, the CLI, a feed/sitemap builder, or any non-`.test.ts` file): then it is NOT dead surface. STOP, do not remove the export, and re-assess with the maintainer.
- **The test imports `getDocumentById` via `./index.js` or `inkwell/db`** rather than `./documents.js`: STOP; fix the test import to `./documents.js` first, otherwise removing the re-export breaks the test.
- **The maintainer wants id-addressing exposed** (for the CLI in Plan 014, webhooks, or an admin UI): STOP the default path and switch to the ESCAPE HATCH — add `GET /documents/by-id/:id` to `src/api.ts` reusing `getDocumentById` with the SAME draft-visibility gating as `handleGet` (unauthenticated callers see published only), KEEP the export and README row, document the new route, and add the `src/api.test.ts` cases above. Do this ONLY on explicit instruction.
- **Drift**: any in-scope file differs from the excerpts above at execution time — STOP and reconcile before editing.

## Maintenance notes

- The choice here is a small policy call about public-surface size; a reviewer should confirm the grep truly shows no live consumer before accepting the removal — that single fact is what makes "dead export" honest rather than a premature deletion of a wanted feature.
- This interacts with Plan 014 (CLI): if the CLI later needs to fetch documents by id, the escape hatch (or re-adding the export) becomes the right move. Note that dependency when reviewing 014.
- Keeping the function in `documents.ts` is deliberate — it is cheap, tested, and ready to re-expose if id-addressing is wanted later; do not "finish the cleanup" by deleting it without a reason.
- If the escape hatch is taken, ensure the id route does not leak draft existence to unauthenticated callers (return 404, not 403/200) to match `handleGet`'s behavior, and confirm pg-mem-based API tests assert response status/body only, never query-planner behavior.
