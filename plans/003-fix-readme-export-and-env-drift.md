# Plan 003: Correct README public-surface and env-var documentation drift

> **Executor instructions**: This is a docs-only plan. Follow each step in order, edit only `README.md`, and run every **Verify** command. Obey the STOP conditions exactly — if any fires, halt and report instead of guessing. When finished, update the status row for plan 003 in `plans/README.md` (create that file if it does not exist; see the Done criteria for the row format).
>
> **Drift check (run first)**: `git diff --stat 6bf6a27..HEAD -- src/index.ts README.md` — if either in-scope/reference file changed, compare the Current-state excerpts below to the live code; on mismatch, STOP and report what differs.

## Status

- **Priority**: P1
- **Effort**: S
- **Risk**: LOW
- **Depends on**: none
- **Category**: docs
- **Planned at**: commit `6bf6a27`, 2026-06-19

## Why this matters

Inkwell ships as a library, so its README is the contract for what callers can `import`. The README's "public module surface" table currently advertises two exports that do **not** exist in `src/index.ts` (`deriveExcerpt`, `handleSitemapRequest`) — anyone who copies those import names gets a runtime/type error — and it omits `renderNotFoundPage`, which **is** exported. The env-vars table also omits `INKWELL_SITE_URL`, an operative variable that controls every absolute URL (canonical / OpenGraph / sitemap / feed); operators reading only the table will deploy with wrong origins. Finally, the `/health` row mislabels a DB-backed readiness probe as a "Liveness check". Fixing the docs removes user-facing breakage at zero code risk.

## Current state

Files in play:

- `src/index.ts` — package entry point; the single source of truth for the public export surface. **Reference only — do NOT edit.**
- `README.md` — user-facing documentation; the only file this plan edits.

### What `src/index.ts` actually exports (lines 10–26, HEAD 6bf6a27)

```ts
export const NAME = 'inkwell';
export const VERSION = '0.1.0';

export { renderMarkdown, renderDocumentHtml } from './rendering.js';
export { handleApiRequest, ApiError, type ApiRequest, type ApiResponse } from './api.js';
export { createServer, createRequestListener } from './server.js';
export {
  handlePageRequest,
  renderIndexPage,
  renderDocumentPage,
  renderNotFoundPage,
  escapeHtml,
  type PageRequest,
  type PageResponse,
} from './pages.js';
export { slugify } from './slug.js';
```

So the public surface is exactly: `NAME`, `VERSION`, `renderMarkdown`, `renderDocumentHtml`, `handleApiRequest`, `ApiError`, type `ApiRequest`, type `ApiResponse`, `createServer`, `createRequestListener`, `handlePageRequest`, `renderIndexPage`, `renderDocumentPage`, `renderNotFoundPage`, `escapeHtml`, type `PageRequest`, type `PageResponse`, `slugify`. It does **NOT** export `deriveExcerpt` or `handleSitemapRequest` (those are internal helpers in `src/feed.ts`/`src/sitemap.ts`/rendering and are not re-exported).

### README "public module surface" table (lines 342–359, HEAD 6bf6a27)

```
The current public module surface is:

| Export                       | Description                                                 |
| ---------------------------- | ----------------------------------------------------------- |
| `NAME`, `VERSION`            | Package metadata constants                                  |
| `slugify(title)`             | Derive a URL-safe slug from a document title                |
| `renderMarkdown(markdown)`   | Render Markdown to sanitized, XSS-safe HTML                 |
| `renderDocumentHtml(body)`   | Produce a document's `rendered_html` from its Markdown body |
| `createServer(db)`           | Build a `node:http` server for the documents API            |
| `createRequestListener(db)`  | Bare request listener, for mounting on an existing server   |
| `handleApiRequest(db, req)`  | Framework-free request handler (routing + validation)       |
| `ApiError`                   | Error type carrying an HTTP status                          |
| `handlePageRequest(db,req)`  | Framework-free HTML page handler (index + document pages)   |
| `renderIndexPage(docs,info)` | Render a paginated index page from a list of documents      |
| `renderDocumentPage(doc)`    | Render a single document's public reading page (with SEO)   |
| `deriveExcerpt(markdown)`    | Derive a short plain-text excerpt/meta description          |
| `escapeHtml(value)`          | HTML-escape a plain-text value for safe interpolation       |
| `handleSitemapRequest(db,r)` | Framework-free `sitemap.xml` handler (published URLs)       |
```

The `deriveExcerpt` and `handleSitemapRequest` rows are wrong (not exported); there is no `renderNotFoundPage` row (but it is exported).

### README env-vars table (lines 63–68, HEAD 6bf6a27)

```
| Variable          | Required | Default   | Used by                | Description ... |
| ----------------- | -------- | --------- | ---------------------- | ... |
| `DATABASE_URL`    | yes      | —         | server, `db:*` scripts | Postgres connection string ... |
| `PORT`            | no       | `3000`    | server                 | TCP port the HTTP server listens on. |
| `HOST`            | no       | `0.0.0.0` | server                 | Address to bind. ... |
| `INKWELL_API_KEY` | no       | —         | server                 | Shared secret required on mutating requests ... |
```

`INKWELL_SITE_URL` is missing from this table. It is documented only in prose at lines 336–338: "Absolute URLs (canonical/OpenGraph/sitemap/feed) are built from the public origin in the `INKWELL_SITE_URL` environment variable ... when unset it falls back to `http://localhost`."

### README endpoints table (line 211, HEAD 6bf6a27)

```
| `GET`    | `/health`                    | Liveness check                        | `200`   | —                              |
```

The `/health` route now pings the DB (`SELECT 1`, returns `503` with `db:down` when unreachable), so "Liveness check" is stale; it is a readiness check.

### Pattern to follow

Pure Markdown table edits. Match the existing GitHub-flavored Markdown table style (pipe-aligned columns). The README is Prettier-formatted, so after editing, `pnpm run format:check` must still pass — Prettier may reflow table column widths, so do not hand-align beyond what Prettier accepts (run `pnpm run format` if alignment drifts, but prefer matching surrounding width). There is no automated test for this plan.

## Commands you will need

| Purpose       | Command                 | Expected                  |
| ------------- | ----------------------- | ------------------------- |
| Drift check   | `git diff --stat 6bf6a27..HEAD -- src/index.ts README.md` | empty or reviewed |
| Format check  | `pnpm run format:check` | exit 0                    |
| Format (fix)  | `pnpm run format`       | reformats files           |
| Status        | `git status`            | only README.md (+ plans/README.md) changed |

## Scope

**In scope:**

- `README.md` — module-surface table, env-vars table, `/health` endpoint row.
- `plans/README.md` (create if absent) — status row update only.

**Out of scope:**

- `src/index.ts` — changing the export set is a separate API decision (see Maintenance notes). Do not edit.
- `src/feed.ts`, `src/sitemap.ts`, `src/rendering.ts`, `src/pages.ts` — where the helpers actually live; no code change needed.
- Any test file — no automated test applies to docs.

## Git workflow

Branch: `advisor/003-fix-readme-export-and-env-drift`.

```
git checkout -b advisor/003-fix-readme-export-and-env-drift
```

Suggested commit (Conventional Commits, docs scope):

```
docs: align README public surface and env-var table with code

- drop non-exported deriveExcerpt/handleSitemapRequest rows
- add renderNotFoundPage to the module-surface table
- document INKWELL_SITE_URL in the env-vars table
- correct /health description to a DB readiness check
```

Do **not** push, commit, or open a PR unless the operator explicitly asks.

## Steps

1. **Module-surface table — remove the `deriveExcerpt` row.** Delete the line `| `deriveExcerpt(markdown)`    | Derive a short plain-text excerpt/meta description          |` from the table near line 357.
   **Verify**: `grep -n 'deriveExcerpt' README.md` -> no match.

2. **Module-surface table — remove the `handleSitemapRequest` row.** Delete the line `| `handleSitemapRequest(db,r)` | Framework-free `sitemap.xml` handler (published URLs)       |` near line 359.
   **Verify**: `grep -n 'handleSitemapRequest' README.md` -> no match.

3. **Module-surface table — add a `renderNotFoundPage` row.** Insert a row for it after the `renderDocumentPage(doc)` row, e.g.:
   `| `renderNotFoundPage(info)`   | Render the styled 404 page (status 404)                     |`
   (column alignment will be normalized by Prettier in step 6).
   **Verify**: `grep -n 'renderNotFoundPage' README.md` -> exactly one match.

4. **Env-vars table — add the `INKWELL_SITE_URL` row.** Insert a row after the `INKWELL_API_KEY` row (near line 68) with: Variable `INKWELL_SITE_URL`; Required `no`; Default `http://localhost`; Used by `server (feed/sitemap/pages)`; Description `Public origin for absolute canonical/OpenGraph/sitemap/feed URLs.`
   **Verify**: `grep -n 'INKWELL_SITE_URL' README.md` -> at least two matches (the new table row + the existing prose at ~line 337).

5. **Endpoints table — fix the `/health` description.** Change the `/health` row's Description column from `Liveness check` to `Readiness check (pings DB)` near line 211. Leave Success `200` and Errors `—` unchanged (the table's Errors column documents handler-validation errors, not transport failures; do not add `503` unless the operator asks).
   **Verify**: `grep -n 'Readiness check (pings DB)' README.md` -> one match; `grep -n 'Liveness check' README.md` -> no match.

6. **Reformat if needed.** Run `pnpm run format:check`. If it reports `README.md`, run `pnpm run format` to let Prettier normalize table alignment, then re-run `pnpm run format:check`.
   **Verify**: `pnpm run format:check` -> exit 0.

## Test plan

No automated test (docs-only change; project has no docs-linting test, and adding one is out of scope). Manual verification:

- Module surface matches code: the README table lists exactly the runtime/value exports from `src/index.ts` and no others. Run
  `grep -nE 'deriveExcerpt|handleSitemapRequest' README.md` -> **no matches**, and
  `grep -n 'renderNotFoundPage' README.md` -> **one match**.
- Env var documented: `grep -n 'INKWELL_SITE_URL' README.md` -> **>= 2 matches** (table row + prose).
- Health description corrected: `grep -n 'Liveness check' README.md` -> **no matches**.
- Formatting intact: `pnpm run format:check` -> **exit 0**.

(Models-after note: there is no existing test to mirror — `src/pages.test.ts` / `src/feed.test.ts` are code tests, not doc checks.)

## Done criteria

- [ ] `grep -nE 'deriveExcerpt|handleSitemapRequest' README.md` returns no matches.
- [ ] `grep -n 'renderNotFoundPage' README.md` returns exactly one match (the new table row).
- [ ] `grep -n 'INKWELL_SITE_URL' README.md` returns >= 2 matches.
- [ ] `grep -n 'Liveness check' README.md` returns no matches; `grep -n 'Readiness check (pings DB)' README.md` returns one match.
- [ ] `pnpm run format:check` exits 0.
- [ ] No out-of-scope files modified: `git status` shows only `README.md` (and `plans/README.md`) changed.
- [ ] `plans/README.md` status row for plan 003 updated to Done (create the file with a header + status table if it does not yet exist; suggested columns: `Plan | Title | Priority | Status`, and add the row `003 | Correct README public-surface and env-var documentation drift | P1 | Done`).

## STOP conditions

- **Export drift**: if `src/index.ts` at HEAD **does** export `deriveExcerpt` and/or `handleSitemapRequest`, then the module-surface table is (partly) correct — STOP and report. In that case only the `renderNotFoundPage` addition, the `INKWELL_SITE_URL` env row, and the `/health` description still need fixing; do not remove rows that now match real exports.
- **Renamed exports**: if `src/index.ts` no longer exports `renderNotFoundPage` (or it was renamed), STOP — do not add a row for a name that does not exist.
- **README restructured**: if the "public module surface" table, the env-vars table, or the endpoints table is missing or has materially different columns than the excerpts above, STOP and report drift rather than guessing placement.
- **Formatting can't converge**: if `pnpm run format:check` still fails after `pnpm run format`, STOP — something outside this edit is unformatted.

## Maintenance notes

- **The real decision deferred here**: this plan fixes docs to match code. The alternative — making `deriveExcerpt` and `handleSitemapRequest` part of the public API — is a code change to `src/index.ts` (re-export from `src/feed.ts`/`src/sitemap.ts`/rendering) plus tests, and is explicitly **out of scope**. If the maintainer wants those public, open a separate plan; do not do both in this branch.
- A reviewer should scrutinize that the module-surface table stays in lockstep with `src/index.ts` — this drift recurs whenever exports change. Consider a future follow-up (separate plan) adding a tiny test or doc-lint that diffs the README table against the actual `index.ts` export list so this cannot silently rot again.
- The `/health` Errors column intentionally stays `—`; if a later plan documents the `503 db:down` body shape, update both the endpoints table and any health-check ADR together.
