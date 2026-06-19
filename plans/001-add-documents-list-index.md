# Plan 001: Add a Postgres index for the hot status-ordered list query

> **Executor instructions**: Follow the steps in order; the working tree must stay green between steps. Run every **Verify** command and confirm the expected result before moving on. Obey all STOP conditions — if one triggers, stop and report rather than improvising. When done, add/update this plan's row in `plans/README.md` (create that file if it does not exist; see Done criteria).
>
> **Drift check (run first)**: `git diff --stat 6bf6a27..HEAD -- src/db/migrations.ts src/db/migrate.test.ts src/db/documents.ts` — if any in-scope file changed, compare the Current-state excerpts below to the live code. **This plan was written at advisor commit `6bf6a27` but the repo HEAD has already moved on, and `src/db/migrations.ts` HAS changed**: a tags migration now occupies id `0003`. The Current-state excerpts below have been reconciled to the live HEAD code. Before editing, re-read `src/db/migrations.ts` and confirm it matches the "Current state" excerpt (three migrations, `0003 = add_document_tags`). If it does NOT match (e.g. only two migrations, or `0003` is something else), STOP and report — the id you must use may differ.

## Status

- **Priority**: P1
- **Effort**: S
- **Risk**: LOW
- **Depends on**: none
- **Category**: perf / migration
- **Planned at**: commit `6bf6a27`, 2026-06-19 (reconciled against live HEAD `10ee86c`)

## Why this matters

Every public read surface — the HTML index, `/page/:n` pagination, the Atom feed, the sitemap, and the authenticated `GET /documents` list — runs a status-filtered, date-ordered query (`WHERE status = $1 ORDER BY created_at DESC, id DESC`). The `documents` table currently has only the primary-key index on `id`, a unique index on `slug`, and a GIN index on `tags`. None of these support the status-filter + created-at ordering, so Postgres does a sequential scan plus an in-memory sort on **every** public request. This is invisible at ~10 documents but degrades to O(n log n) per request as the corpus grows — exactly the read path that gets the most traffic. A single composite B-tree index makes the filter+sort an index range scan and is the cheapest high-leverage performance win available.

## Current state

Files and roles:

- `src/db/migrations.ts` — ordered, immutable `Migration` records (`{ id, name, up, down }`) and the `MIGRATIONS` array listing them in apply order. Header comment: "Never edit a migration that has shipped — add a new one instead." **In scope (append a new migration).**
- `src/db/migrate.ts` — the runner. `migrate(db)` applies pending migrations in ascending id order; `rollback(db, { steps })` reverts newest-first; `appliedMigrationIds(db)` returns recorded ids ascending. **Read-only context, do not edit.**
- `src/db/migrate.test.ts` — Vitest tests for the runner, using `createMemoryDatabase()` (pg-mem, in-memory, no live DB). **In scope (update assertions, add a rollback test).**
- `src/db/documents.ts` — the DAL. `listDocuments` and `listPublishedDocuments` build the hot queries the new index serves. **Out of scope (queries are already correct; the index just needs to exist).**

`src/db/migrations.ts` currently ends with THREE migrations (live HEAD, reconciled — id `0003` is the tags migration, NOT the list index):

```ts
/** All migrations, in apply order. */
export const MIGRATIONS: readonly Migration[] = [
  createDocuments,
  addDocumentStatus,
  addDocumentTags,
];
```

`createDocuments` (id `0001`) creates `documents` with `id uuid PRIMARY KEY DEFAULT gen_random_uuid()`, `slug text NOT NULL UNIQUE`, plus `title`, `body_markdown`, `rendered_html`, `created_at timestamptz NOT NULL DEFAULT now()`, `updated_at`. `addDocumentStatus` (id `0002`) adds `status text NOT NULL DEFAULT 'draft' CHECK (status IN ('draft','published'))` then `UPDATE documents SET status = 'published';`. `addDocumentTags` (id `0003`) adds `tags text[] NOT NULL DEFAULT '{}'` and `CREATE INDEX documents_tags_idx ON documents USING gin (tags);` with a `down` that drops the index then the column. There is **no** B-tree index supporting the status + created_at ordering.

`src/db/documents.ts` `listDocuments` (~lines 223-241) builds: `SELECT <cols> FROM documents [WHERE status = $n] ORDER BY created_at DESC, id DESC [LIMIT $n] [OFFSET $n]`. `listPublishedDocuments` (just below) builds `SELECT <cols> FROM documents WHERE status = $1 ORDER BY created_at DESC, id DESC [LIMIT $n]`. These are the queries the new index serves; they are already shaped correctly and must not be touched.

Existing `migrate.test.ts` already expects three migrations (verified at HEAD):

```ts
it('applies all migrations to a fresh database', async () => {
  const applied = await migrate(db);
  expect(applied).toEqual(MIGRATIONS.map((m) => m.id));
  expect(await appliedMigrationIds(db)).toEqual(['0001', '0002', '0003']);
});
```
```ts
it('is idempotent — a second run applies nothing', async () => {
  await migrate(db);
  const secondRun = await migrate(db);
  expect(secondRun).toEqual([]);
  expect(await appliedMigrationIds(db)).toEqual(['0001', '0002', '0003']);
});
```
```ts
it('records the migration name in the ledger', async () => {
  await migrate(db);
  const ledger = await db.query<{ id: string; name: string }>(
    `SELECT id, name FROM schema_migrations ORDER BY id`,
  );
  expect(ledger.rows).toEqual([
    { id: '0001', name: 'create_documents' },
    { id: '0002', name: 'add_document_status' },
    { id: '0003', name: 'add_document_tags' },
  ]);
});
```

Exemplar to model the new rollback test after — the existing "rolls back the most recent migration" test:

```ts
it('rolls back the most recent migration', async () => {
  await migrate(db);
  const reverted = await rollback(db);
  expect(reverted).toEqual(['0003']);
  expect(await appliedMigrationIds(db)).toEqual(['0001', '0002']);

  await expect(db.query(`SELECT tags FROM documents`)).rejects.toThrow();
  const stillThere = await db.query(`SELECT slug, status FROM documents`);
  expect(stillThere.rows).toEqual([]);
});
```

**Test conventions**: pg-mem implements only a Postgres SUBSET. Index existence/EXPLAIN/query-planner behavior is NOT reliable in pg-mem — assert only that migrations apply and roll back and that schema/SQL is correct. NEVER assert that the index is "used."

## Commands you will need

| Purpose | Command | Expected |
| --- | --- | --- |
| Install | `pnpm install` | exit 0 |
| Tests (one file) | `pnpm test src/db/migrate.test.ts` | pass |
| Tests (all) | `pnpm test` | all pass |
| Typecheck | `pnpm run typecheck` | exit 0, no errors |
| Lint | `pnpm run lint` | exit 0 |
| Format check | `pnpm run format:check` | exit 0 |
| Build | `pnpm run build` | exit 0 |
| Full gate | `pnpm run ci` | exit 0 |

## Scope

**In scope:**
- `src/db/migrations.ts` — append ONE new migration record and add it to the end of the `MIGRATIONS` array.
- `src/db/migrate.test.ts` — add ledger-name assertion for the new migration and a rollback regression test for it. (The id-count assertions in "applies all migrations" and "idempotent" already use the array-derived value or the literal `['0001','0002','0003']` — see Steps for exactly what to change.)

**Out of scope:**
- `src/db/documents.ts` — query SQL is already correct; the index serves the existing queries.
- Migrations `0001`, `0002`, `0003` (the create/status/tags records) — immutable; never edit a shipped migration.
- `src/db/migrate.ts` — runner needs no change.
- `src/feed.ts`, `src/sitemap.ts`, `src/pages.ts`, `src/api.ts` — they call the DAL; no change needed for the index to take effect.

## Git workflow

Create a branch: `advisor/001-add-documents-list-index`. Make focused commits using Conventional Commits, e.g.:

```
perf(db): add composite index for status-ordered document list

Backs the hot WHERE status / ORDER BY created_at DESC, id DESC read path
(index, page, feed, sitemap, GET /documents) with a B-tree index instead
of a seq scan + sort. Adds migration 0004 and rollback coverage.
```

Do NOT push, commit, or open a PR unless the operator explicitly asks.

## Steps

1. **Append the new migration to `src/db/migrations.ts`.** The next free id is `0004` (`0003` is already `add_document_tags`). Insert a new `const` record after `addDocumentTags` and before the `MIGRATIONS` array, with a doc comment explaining the index. Use:
   - `id: '0004'`
   - `name: 'add_documents_list_index'`
   - `up`: `CREATE INDEX documents_status_created_at_id_idx ON documents (status, created_at DESC, id DESC);`
   - `down`: `DROP INDEX documents_status_created_at_id_idx;`

   Then append `addDocumentsListIndex` as the LAST element of the `MIGRATIONS` array (order: `[createDocuments, addDocumentStatus, addDocumentTags, addDocumentsListIndex]`). Match the existing file style: a JSDoc block above the record, template-literal `up`/`down`, two-space indent.

   **Verify**: `pnpm run typecheck` -> exit 0, no errors.

2. **Confirm the migration applies and rolls back under pg-mem** before touching tests. Run the existing suite for this file; it will FAIL now because the id-count assertions still expect three ids — that is expected and fixed in Step 3. But watch for any error mentioning `CREATE INDEX` / `DROP INDEX` syntax: if pg-mem throws on applying the new migration, that is a STOP condition (see below), not a test-assertion failure.

   **Verify**: `pnpm test src/db/migrate.test.ts` -> the only failures are assertion mismatches expecting `['0001','0002','0003']` (NOT a thrown SQL/`CREATE INDEX` error during `migrate`).

3. **Update assertions in `src/db/migrate.test.ts`.** Make these exact edits:
   - "applies all migrations to a fresh database": change `expect(await appliedMigrationIds(db)).toEqual(['0001', '0002', '0003']);` to `expect(await appliedMigrationIds(db)).toEqual(['0001', '0002', '0003', '0004']);`. (The `expect(applied).toEqual(MIGRATIONS.map((m) => m.id));` line needs no change — it derives from the array.)
   - "is idempotent — a second run applies nothing": change `expect(await appliedMigrationIds(db)).toEqual(['0001', '0002', '0003']);` to `['0001', '0002', '0003', '0004']`.
   - "records the migration name in the ledger": add a fourth row to the expected array: `{ id: '0004', name: 'add_documents_list_index' }`.
   - "rolls back every migration when asked": change `expect(reverted).toEqual(['0003', '0002', '0001']);` to `['0004', '0003', '0002', '0001']` (rollback is newest-first).

   **Verify**: `pnpm test src/db/migrate.test.ts` -> all existing cases pass.

4. **Add a rollback regression test** for the new migration, modeled after "rolls back the most recent migration". Add an `it('rolls back the documents list index migration', ...)` (or similar) that: runs `migrate(db)`, then `rollback(db)`, asserts `reverted` equals `['0004']` and `appliedMigrationIds(db)` equals `['0001','0002','0003']`, and asserts the table plus the `status` and `tags` columns still exist (e.g. `const stillThere = await db.query(`SELECT slug, status, tags FROM documents`); expect(stillThere.rows).toEqual([]);` does not throw). Do NOT assert index existence or query-planner output (pg-mem cannot model it). Note: rolling back a single step now reverts `0004`, so the existing "rolls back the most recent migration" test (which reverts `0003 = add_document_tags`) must FIRST roll back twice or be left as-is only if it already accounts for `0004` being newest — re-read it: it currently calls `rollback(db)` once and expects `['0003']`. After adding `0004`, a single rollback reverts `0004`, so that test will now fail. Fix it by either (a) updating its expectation to revert `0004` and adjusting its column assertions, or (b) having it roll back two steps. Prefer (a): rename/repoint your NEW test to be the single-step `0004` rollback, and update the existing "rolls back the most recent migration" test to use `rollback(db, { steps: 2 })` expecting `['0004','0003']` with the tags-gone assertion intact. Pick whichever keeps both behaviors covered with no duplicate; ensure exactly one test exercises single-step rollback of `0004` and one exercises the tags-gone path.

   **Verify**: `pnpm test src/db/migrate.test.ts` -> all pass (including the new/updated rollback cases).

5. **Run the full quality gate.**

   **Verify**: `pnpm run ci` -> exit 0.

## Test plan

- **File**: `src/db/migrate.test.ts` (existing).
- **Cases**:
  - Updated id assertions (count now includes `0004`) in "applies all migrations" and "idempotent".
  - Updated ledger-name assertion includes `{ id: '0004', name: 'add_documents_list_index' }`.
  - Updated "rolls back every migration" newest-first list includes `0004` first.
  - **Regression for this finding**: a single-step `rollback(db)` after `migrate(db)` reverts exactly `['0004']`, leaves `['0001','0002','0003']` applied, and the table + `status` + `tags` columns remain usable (the `down` dropped only the index). This proves the new migration's `up`/`down` are well-formed and reversible.
  - A test that exercises the tags-gone rollback path remains (multi-step or repointed), so `0003` coverage is not lost.
- **Model after**: the existing "rolls back the most recent migration" test.
- **Do NOT**: assert index usage, EXPLAIN output, or planner behavior — pg-mem cannot model these.
- **Verification command**: `pnpm test src/db/migrate.test.ts`, then `pnpm test`.

## Done criteria

- [ ] `src/db/migrations.ts` has a new `id: '0004'`, `name: 'add_documents_list_index'` migration with the `CREATE INDEX` up / `DROP INDEX` down, appended last in `MIGRATIONS`.
- [ ] Migrations `0001`/`0002`/`0003` are byte-for-byte unchanged.
- [ ] `pnpm run typecheck` exits 0.
- [ ] `pnpm run lint` exits 0.
- [ ] `pnpm run format:check` exits 0 (run `pnpm prettier --write` on touched files if it fails, then re-check).
- [ ] `pnpm test` all pass; `src/db/migrate.test.ts` includes the new `0004` rollback regression.
- [ ] `pnpm run ci` exits 0.
- [ ] No out-of-scope files modified — confirm with `git status` (only `src/db/migrations.ts`, `src/db/migrate.test.ts`, and `plans/README.md` should appear).
- [ ] `plans/README.md` row for Plan 001 updated to Done (create `plans/README.md` with a status table if it does not yet exist; add a row: `| 001 | Add documents list index | perf/migration | P1 | Done |`).

## STOP conditions

- **Drift / id collision**: if `src/db/migrations.ts` does not match the Current-state excerpt (e.g. `0003` is not `add_document_tags`, or a `0004` already exists, or the highest id is something else), STOP and report. Using the wrong id or duplicating one corrupts the ordered ledger.
- **pg-mem rejects the DDL**: if `migrate(db)` THROWS on the new migration's `CREATE INDEX ... (status, created_at DESC, id DESC)` (as opposed to a plain assertion mismatch), STOP and report. pg-mem may not accept the `DESC` qualifiers or the multi-column form; do not silently weaken the production index just to satisfy pg-mem. Report the exact error so the operator can decide whether to (a) keep the production-correct `up` and gate/skip the apply assertion, or (b) adjust syntax. Note the existing `0003` GIN index already applies under pg-mem, so basic `CREATE INDEX` works — the risk is specifically the `DESC` ordering qualifiers.
- **Existing rollback test can't be cleanly reconciled**: if you cannot arrange for both the `0004` single-step rollback and the `0003` tags-gone path to be covered without a duplicate/contradictory test, STOP and report rather than deleting coverage.

## Maintenance notes

- This index `documents_status_created_at_id_idx` is column-order-sensitive: it serves `WHERE status = ? ORDER BY created_at DESC, id DESC`. If the public list ordering columns or direction ever change (in `listDocuments` / `listPublishedDocuments`), revisit this index — a mismatched index silently stops helping.
- The tags feature (migration `0003`) already added a GIN index for `tag = ANY(tags)` containment. If tag-listing pages later add their own status-filtered ordered query, consider whether a `(status, ...)`-leading composite covering tags is warranted; do not overload this index.
- A reviewer should scrutinize: (1) the migration id is `0004` and unique, (2) `0001`/`0002`/`0003` are untouched, (3) `down` exactly reverses `up` (drops the same index name), and (4) no test asserts planner behavior.
- Deferred follow-up: once a live Postgres integration test harness exists (pg-mem cannot verify index usage), add an `EXPLAIN`-based assertion there confirming the list query uses an index scan rather than a seq scan + sort.
