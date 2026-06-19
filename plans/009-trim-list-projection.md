# Plan 009: Trim `rendered_html` over-fetch on the public index listing

> **Executor instructions**: Follow the steps in order; the tree must typecheck/lint/test green between steps. Run EVERY `**Verify**` command and do not proceed past a failed one. Obey the STOP conditions verbatim — this is a deliberately small, low-leverage change and bailing out is an acceptable, expected outcome. When finished, add/update the status row for this plan in `plans/README.md` (create the file if it does not exist).
> **Drift check (run first)**: `git diff --stat 6bf6a27..HEAD -- src/db/documents.ts src/pages.ts src/db/documents.test.ts` — if any in-scope file changed, compare the Current-state excerpts below to live code; on mismatch, STOP and report the drift instead of editing.

## Status
- **Priority**: P3
- **Effort**: S
- **Risk**: LOW
- **Depends on**: none
- **Category**: perf
- **Planned at**: commit `6bf6a27`, 2026-06-19

## Why this matters
The public index/pagination surface fetches the full `documents` row — including `rendered_html`, which can be large — yet the index only renders the title, slug, dates, tags, and a short excerpt derived from `body_markdown`. On a site with long articles this transfers and deserializes a big HTML column per row, per index page, for no benefit. Adding a narrow list-projection that omits `rendered_html` removes that waste with a tiny, well-contained change. Be honest about leverage: this is a small win (single read path, modest payload reduction), so if threading the new type ripples widely, closing this as "not worth doing" is the correct outcome (see STOP conditions). Excerpt memoization is explicitly out of scope — its cache-invalidation cost outweighs the per-render saving.

## Current state
Files and their role:
- `src/db/documents.ts` — typed data-access layer for the `documents` table. Defines the `Document` domain type, the `DocumentRow` raw shape, the shared `RETURNING` column list, and the read functions `listDocuments` / `listPublishedDocuments`.
- `src/pages.ts` — framework-free HTML frontend. `handlePageRequest` serves the index; `renderIndexPage` builds the list markup; `deriveExcerpt` produces the short summary text from markdown.
- `src/feed.ts` — Atom feed builder; uses `listPublishedDocuments` and DOES need `rendered_html` for `<content>`. Leave it alone.
- `src/sitemap.ts` — also uses `listPublishedDocuments`; out of scope.

Verified excerpts (HEAD `6bf6a27`):

`src/db/documents.ts:27-44` — the public `Document` type (DO NOT change this type):
```ts
/** A document as stored in Postgres, mapped to domain shape. */
export interface Document {
  readonly id: string;
  readonly slug: string;
  readonly title: string;
  readonly bodyMarkdown: string;
  readonly renderedHtml: string;
  readonly status: DocumentStatus;
  readonly tags: readonly string[];
  readonly createdAt: Date;
  readonly updatedAt: Date;
}
```

`src/db/documents.ts:83-94` — the raw row shape:
```ts
/** Raw row shape returned by `SELECT * FROM documents`. */
interface DocumentRow {
  id: string;
  slug: string;
  title: string;
  body_markdown: string;
  rendered_html: string;
  status: DocumentStatus;
  tags: string[] | null;
  created_at: Date;
  updated_at: Date;
}
```

`src/db/documents.ts:138` — the shared full projection (NOTE: it includes `tags`, which the original spec excerpt omitted; the live column list is exactly this):
```ts
const RETURNING = `id, slug, title, body_markdown, rendered_html, status, tags, created_at, updated_at`;
```

`src/db/documents.ts:202-241` — the existing `ListOptions` + `listDocuments` (the function the index path calls; model the new query after this one):
```ts
/** Optional paging window and status filter for {@link listDocuments}. */
export interface ListOptions {
  readonly limit?: number;
  readonly offset?: number;
  readonly status?: DocumentStatus;
}

export async function listDocuments(db: Queryable, options: ListOptions = {}): Promise<Document[]> {
  const params: unknown[] = [];
  let sql = `SELECT ${RETURNING} FROM documents`;
  if (options.status) {
    params.push(options.status);
    sql += ` WHERE status = $${params.length}`;
  }
  sql += ` ORDER BY created_at DESC, id DESC`;
  if (options.limit !== undefined) {
    params.push(options.limit);
    sql += ` LIMIT $${params.length}`;
  }
  if (options.offset !== undefined) {
    params.push(options.offset);
    sql += ` OFFSET $${params.length}`;
  }
  const result = await db.query<DocumentRow>(sql, params);
  return result.rows.map(toDocument);
}
```

`src/pages.ts:347-370` — `renderIndexPage`; note it reads only `doc.bodyMarkdown` (for the excerpt), `doc.slug`, `doc.title`, `doc.createdAt`, and `doc.tags` — never `doc.renderedHtml`:
```ts
/** Render the index: a list of published documents, newest first, paginated. */
export function renderIndexPage(
  documents: readonly Document[],
  info: IndexPageInfo,
  options: PageOptions = {},
): string {
  const base = normalizeSiteUrl(options.siteUrl);
  const list =
    documents.length === 0
      ? `<p class="empty">No documents published yet.</p>`
      : `<ul class="index">
${documents
  .map((doc) => {
    const excerpt = deriveExcerpt(doc.bodyMarkdown);
    // ... uses doc.slug, doc.title, doc.createdAt, doc.tags — never doc.renderedHtml
```

`src/pages.ts:687-692` — the index call site inside `handlePageRequest` (this is the ONLY call to switch over). NOTE: the index path calls `listDocuments` with `status: 'published'`, NOT `listPublishedDocuments`:
```ts
    const docs = await listDocuments(db, {
      status: 'published',
      limit: PAGE_SIZE,
      offset: (page - 1) * PAGE_SIZE,
    });
    return { status: 200, html: renderIndexPage(docs, { page, totalPages }, options) };
```

Test exemplar to follow — `src/db/documents.test.ts:22-35` (the `sample` fixture and `beforeEach` setup; in-memory Postgres via `createMemoryDatabase`, then `migrate(db)`):
```ts
const sample = {
  slug: 'hello-world',
  title: 'Hello World',
  bodyMarkdown: '# Hello',
  renderedHtml: '<h1>Hello</h1>',
};

describe('documents data-access layer', () => {
  let db: Queryable;
  beforeEach(async () => {
    db = createMemoryDatabase().db;
    await migrate(db);
  });
```
And `src/db/documents.test.ts:71-119` for the list ordering/status-filter assertion patterns to mirror.

## Commands you will need
| Purpose | Command | Expected |
| --- | --- | --- |
| Install | `pnpm install` | exit 0 |
| Typecheck | `pnpm run typecheck` | exit 0, no errors |
| Lint | `pnpm run lint` | exit 0 |
| Format check | `pnpm run format:check` | exit 0 |
| Tests (one file) | `pnpm test src/db/documents.test.ts` | pass |
| Tests (one file) | `pnpm test src/pages.test.ts` | pass |
| Tests (all) | `pnpm test` | all pass (164+) |
| Full gate | `pnpm run ci` | exit 0 |

## Scope
**In scope** (only these files may change):
- `src/db/documents.ts` — add a `DocumentSummary` type, a `SUMMARY_RETURNING` column list (no `rendered_html`), a `toSummary` mapper, and a `listDocumentSummaries(db, options)` function.
- `src/pages.ts` — switch ONLY the index call site (the `listDocuments` call at ~line 687) to `listDocumentSummaries`, and widen `renderIndexPage`'s parameter type so it accepts the summary shape (it already touches no `renderedHtml`).
- `src/db/documents.test.ts` — add DAL tests for `listDocumentSummaries`.

**Out of scope** (do NOT touch):
- `src/feed.ts` — the Atom feed needs `rendered_html` for `<content>`; keep it on `listPublishedDocuments`.
- `src/sitemap.ts` — uses `listPublishedDocuments`; no payload concern worth changing.
- The `Document` interface — must stay exactly as-is; other call sites depend on `renderedHtml` being present.
- `listDocuments` / `listPublishedDocuments` themselves — leave their full projection intact (used by API list reads and the feed).
- Excerpt memoization / `deriveExcerpt` — deferred (see Maintenance notes).

## Git workflow
- Branch: `advisor/009-trim-list-projection`.
- Conventional commit example: `perf(db): add rendered_html-free list projection for the public index`.
- Do NOT push, commit beyond your own work, open a PR, or bump versions unless the operator explicitly asks.

## Steps

1. **Add the summary type, projection, mapper, and query in `src/db/documents.ts`.**
   - Define an exported `DocumentSummary` interface = `Document` minus `renderedHtml` (fields: `id`, `slug`, `title`, `bodyMarkdown`, `status`, `tags`, `createdAt`, `updatedAt`, all `readonly`, same types).
   - Define a module-private `SUMMARY_RETURNING` constant: `` const SUMMARY_RETURNING = `id, slug, title, body_markdown, status, tags, created_at, updated_at`; `` (the `RETURNING` list with `rendered_html` removed).
   - Define a module-private row shape for the projection, e.g. `type DocumentSummaryRow = Omit<DocumentRow, 'rendered_html'>;` and a `toSummary(row: DocumentSummaryRow): DocumentSummary` mapper that mirrors `toDocument` but omits `renderedHtml` and keeps the `tags: row.tags ?? []` defensive coalesce.
   - Add `listDocumentSummaries(db: Queryable, options: ListOptions = {}): Promise<DocumentSummary[]>`, structurally identical to `listDocuments` (same `status`/`limit`/`offset` handling, same `ORDER BY created_at DESC, id DESC`) but selecting `SUMMARY_RETURNING`, typing the query as `db.query<DocumentSummaryRow>(...)`, and mapping with `toSummary`. Reuse the existing `ListOptions` interface — do NOT introduce a new options type.
   - Use `import type` only where types are imported; keep `.js` extensions on any new imports (none expected here).
   - **Verify**: `pnpm run typecheck` -> exit 0.

2. **Switch the index call site in `src/pages.ts` to the summary query.**
   - Add `listDocumentSummaries` and `type DocumentSummary` to the existing import from `./db/documents.js` (keep `type` modifiers correct under `verbatimModuleSyntax`).
   - Change the index `listDocuments(db, { status: 'published', limit: PAGE_SIZE, offset: ... })` call (~line 687) to `listDocumentSummaries(db, { ... })` with the same options object.
   - Widen `renderIndexPage`'s `documents` parameter type from `readonly Document[]` to `readonly DocumentSummary[]` (the function already references no `renderedHtml`, so the body is unchanged). If `Document` is now unused in `pages.ts`, drop it from the type import; if still used elsewhere in the file, keep it.
   - Do NOT change the tag-listing or document-page paths.
   - **Verify**: `pnpm run typecheck` -> exit 0, then `pnpm test src/pages.test.ts` -> pass.

3. **Add DAL tests for `listDocumentSummaries` in `src/db/documents.test.ts`.**
   - Add `listDocumentSummaries` (and `type DocumentSummary` if you assert on shape) to the imports.
   - Add a `describe('listDocumentSummaries', ...)` (or `it` cases within the existing block) modeled on the `sample` fixture + `beforeEach` setup, covering:
     - Returns rows WITHOUT a `renderedHtml` property: create a published doc, then assert `expect('renderedHtml' in summary).toBe(false)` (or `expect((summary as Record<string, unknown>).renderedHtml).toBeUndefined()`), while `bodyMarkdown`, `title`, `slug`, `tags` are present and correct.
     - Respects the `status` filter (create one draft + one published; `{ status: 'published' }` returns only the published slug).
     - Respects newest-first ordering and `limit`/`offset` paging (create 3 published docs; assert the windowed slugs in the expected order). Mirror the ordering/paging assertions used by the existing `listDocuments` tests.
   - **Verify**: `pnpm test src/db/documents.test.ts` -> pass.

4. **Run the full local gate and tidy formatting.**
   - **Verify**: `pnpm run lint` -> exit 0; `pnpm run format:check` -> exit 0 (run the repo formatter if it flags files); `pnpm test` -> all pass; `pnpm run ci` -> exit 0.

5. **Update `plans/README.md`.**
   - Create the file if absent. Add/update a status row for Plan 009 marking it Done (or "Closed — not worth doing" if you hit a STOP condition), matching whatever table/format already exists there.
   - **Verify**: `git status --porcelain` shows ONLY the four in-scope files plus `plans/README.md` (and this plan file) changed.

## Test plan
- **New tests** (in `src/db/documents.test.ts`, modeled on the existing `documents data-access layer` block and its `listDocuments` cases at lines 71-119):
  - Regression for THIS finding: `listDocumentSummaries` returns objects with NO `renderedHtml` field while still carrying `bodyMarkdown` (the excerpt source) — this is what locks in the over-fetch trim.
  - Status filter: `{ status: 'published' }` excludes drafts.
  - Ordering + paging: newest-first, `limit`/`offset` window returns the expected slugs in order.
- **Existing tests to keep green**: `src/pages.test.ts` (index renders titles + excerpts via the new summary path), plus the full `src/db/documents.test.ts` and `src/feed.test.ts`/`src/sitemap.test.ts` (must be unaffected since their full-projection reads are untouched).
- **Verification command**: `pnpm test` then `pnpm run typecheck`.

## Done criteria
- [ ] `git diff --stat 6bf6a27..HEAD` drift check passed (or drift reported and STOPPED).
- [ ] `src/db/documents.ts` exports `DocumentSummary` and `listDocumentSummaries`; full `RETURNING` and `Document` interface unchanged.
- [ ] `src/pages.ts` index path uses `listDocumentSummaries`; tag/document-page paths unchanged; `feed.ts`/`sitemap.ts` untouched.
- [ ] New DAL tests added, including the no-`renderedHtml` regression assertion.
- [ ] `pnpm run typecheck` exit 0.
- [ ] `pnpm run lint` exit 0.
- [ ] `pnpm run format:check` exit 0.
- [ ] `pnpm test` all pass.
- [ ] `pnpm run ci` exit 0.
- [ ] No out-of-scope files modified (`git status` shows only in-scope files + `plans/README.md` + this plan).
- [ ] `plans/README.md` status row updated.

## STOP conditions
- **Ripple beyond the index path.** If widening `renderIndexPage` to `DocumentSummary` or threading the new type forces edits to more than `src/db/documents.ts`, `src/pages.ts`, and `src/db/documents.test.ts` (e.g. it cascades into the tag path, `index.ts` public exports, the API layer, or shared helpers), STOP. The win is small; report the cascade and recommend closing the plan as "not worth doing" rather than expanding scope.
- **Any temptation to change the `Document` interface or the shared `RETURNING` list** — do not. If the task seems to require it, STOP and report; that means the projection split is the wrong approach here.
- **pg-mem projection mismatch.** If the in-memory Postgres rejects the narrower `SELECT` column list or returns unexpected shapes, do NOT assert on query-planner/EXPLAIN behavior to work around it — STOP and report; pg-mem is a Postgres subset and column-projection support is the relevant risk, not indexing.
- **Drift**: any in-scope file differs from the Current-state excerpts -> STOP.

## Maintenance notes
- A reviewer should confirm `feed.ts` and `sitemap.ts` still call `listPublishedDocuments` (full projection) — the feed's `<content>` needs `rendered_html`; regressing those to the summary would break the feed.
- The `toSummary` mapper must keep the `tags: row.tags ?? []` defensive coalesce so a partial projection never yields `undefined` tags (matching `toDocument`).
- **Deferred / better long-term fix**: persist a precomputed `excerpt` column on `documents` via a NEW migration (next zero-padded id in `src/db/migrations.ts` — never edit a shipped migration), so the index needn't fetch `body_markdown` either and `deriveExcerpt` need not run per render. That supersedes both this projection trim and the deferred excerpt-memoization idea; track it separately.
- Excerpt memoization in `pages.ts` was intentionally NOT done: cache-invalidation complexity outweighs the per-page cost. Do not add an in-process excerpt cache under this plan.
