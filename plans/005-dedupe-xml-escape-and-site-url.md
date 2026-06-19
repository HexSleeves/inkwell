# Plan 005: Consolidate duplicated escapeXml and normalizeSiteUrl

> **Executor instructions**: Follow each step in order; the tree must stay green between steps. Run every **Verify** command and confirm the expected result before moving on. Obey all STOP conditions literally. When finished, update the status row for plan 005 in `plans/README.md` (create that file if it does not exist — see Done criteria for the row format). Do NOT push, commit, or open a PR unless the operator explicitly asks.

> **Drift check (run first)**: `git diff --stat 6bf6a27..HEAD -- src/feed.ts src/sitemap.ts src/site-url.ts src/feed.test.ts src/sitemap.test.ts` — if any in-scope file changed, compare the Current-state excerpts below to the live code; on any mismatch, STOP and report.

## Status
- **Priority**: P2
- **Effort**: S
- **Risk**: LOW
- **Depends on**: none
- **Category**: tech-debt
- **Planned at**: commit `6bf6a27`, 2026-06-19

## Why this matters
The XML-escaping function (`escapeXml`) is duplicated byte-for-byte across `src/feed.ts` and `src/sitemap.ts`, and the site-URL normalizer (`normalizeSiteUrl`) exists in three forms: a private copy in `src/feed.ts`, the canonical export in `src/site-url.ts`, and the import already in use by `src/sitemap.ts`. Hand-maintained duplicate copies of an escaping function are a latent correctness/security bug: if one copy is changed (e.g. to escape an additional character) and the other is not, the feed and sitemap silently diverge and one surface can emit unescaped, injectable XML. Consolidating to a single source means future XML/escaping fixes happen once and cannot drift.

## Current state
Files and their roles:
- `src/feed.ts` — builds the Atom 1.0 feed. Defines and **exports** `escapeXml`, defines a **private** `normalizeSiteUrl`, and defines its own private `const DEFAULT_SITE_URL`.
- `src/sitemap.ts` — builds `sitemap.xml`. Defines and **exports** a byte-identical `escapeXml`, and **already imports** `normalizeSiteUrl` from `./site-url.js`.
- `src/site-url.ts` — canonical home of `normalizeSiteUrl` and `DEFAULT_SITE_URL`; its header comment notes that `feed.ts` predates it and keeps its own copy.
- `src/feed.test.ts` — imports `escapeXml` from `./feed.js` and asserts its behavior.
- `src/sitemap.test.ts` — imports `escapeXml` from `./sitemap.js` and asserts its behavior.

Verified excerpts at HEAD `6bf6a27`:

`src/feed.ts:60-74`:
```ts
/** Escape the five characters that are unsafe in XML text/attribute contexts. */
export function escapeXml(value: string): string {
  return value
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
    .replace(/'/g, '&apos;');
}

/** Normalize a configured site URL to an absolute origin with no trailing slash. */
function normalizeSiteUrl(siteUrl: string | undefined): string {
  const base = (siteUrl ?? '').trim() || DEFAULT_SITE_URL;
  return base.replace(/\/+$/, '');
}
```

`src/feed.ts:29` (private constant feed.ts relies on):
```ts
const DEFAULT_SITE_URL = 'http://localhost';
```
`src/feed.ts:51` references it in a doc comment (`falls back to {@link DEFAULT_SITE_URL}`), and `src/feed.ts:83` calls `const base = normalizeSiteUrl(options.siteUrl);`.

`src/sitemap.ts:54-62` (byte-identical to feed.ts's escapeXml):
```ts
/** Escape the five characters that are unsafe in XML text/attribute contexts. */
export function escapeXml(value: string): string {
  return value
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
    .replace(/'/g, '&apos;');
}
```

`src/sitemap.ts:25` (already importing the canonical normalizer):
```ts
import { normalizeSiteUrl } from './site-url.js';
```

`src/site-url.ts:14-25` (canonical copies):
```ts
/** Default site origin used when `INKWELL_SITE_URL` is not configured. */
export const DEFAULT_SITE_URL = 'http://localhost';

/**
 * Normalize a configured site URL to an absolute origin with no trailing slash.
 * A trailing slash is tolerated and trimmed so `https://x/` and `https://x`
 * produce identical absolute URLs downstream.
 */
export function normalizeSiteUrl(siteUrl: string | undefined): string {
  const base = (siteUrl ?? '').trim() || DEFAULT_SITE_URL;
  return base.replace(/\/+$/, '');
}
```

Test imports verified:
- `src/feed.test.ts:15-21` imports `escapeXml` from `./feed.js` (alongside `ATOM_CONTENT_TYPE`, `FEED_MAX_ENTRIES`, `buildAtomFeed`, `handleFeedRequest`). Cases at `:77-84`.
- `src/sitemap.test.ts:15` imports `escapeXml` from `./sitemap.js` (`import { SITEMAP_CONTENT_TYPE, buildSitemap, escapeXml, handleSitemapRequest } from './sitemap.js';`). Cases at `:66`.

Confirmed at HEAD: the two `escapeXml` bodies are byte-identical, so consolidation cannot change output. The `normalizeSiteUrl` body in `feed.ts` is identical to the canonical one in `site-url.ts` (both use `(siteUrl ?? '').trim() || DEFAULT_SITE_URL` then `.replace(/\/+$/, '')`), and both `DEFAULT_SITE_URL` constants equal `'http://localhost'`.

Pattern to follow: `src/sitemap.ts` already imports `normalizeSiteUrl` from `./site-url.js` — mirror exactly that import for `feed.ts`. Tests are pure-builder + handler tests; do not add a live DB.

## Commands you will need
| Purpose | Command | Expected |
| --- | --- | --- |
| Install | `pnpm install` | exit 0 |
| Typecheck | `pnpm run typecheck` | exit 0, no errors |
| Lint | `pnpm run lint` | exit 0 |
| Format check | `pnpm run format:check` | exit 0 |
| Targeted tests | `pnpm test src/feed.test.ts src/sitemap.test.ts` | pass |
| Tests (all) | `pnpm test` | all pass (164+) |
| Build | `pnpm run build` | exit 0 |
| Full gate | `pnpm run ci` | exit 0 |

## Scope
**In scope**:
- `src/xml.ts` (create) — new home for the single shared `escapeXml`.
- `src/feed.ts` — import shared `escapeXml`; delete its local `escapeXml`, its private `normalizeSiteUrl`, and its private `DEFAULT_SITE_URL`; import `normalizeSiteUrl` (and `DEFAULT_SITE_URL` if still referenced) from `./site-url.js`.
- `src/sitemap.ts` — import shared `escapeXml`; delete its local `escapeXml`.
- `src/feed.test.ts` — update the `escapeXml` import source (see Step 3 for the re-export-vs-update decision).
- `src/sitemap.test.ts` — update the `escapeXml` import source (same decision).

**Out of scope** (do not touch):
- The actual feed/sitemap XML output strings/templates — output must be byte-identical before and after. If any output/snapshot assertion changes, you broke something; STOP.
- `src/site-url.ts` — it is already canonical and correct; only import from it. (Do not move `escapeXml` into it; this plan creates `src/xml.ts` instead — see Step 1.)
- `src/pages.ts`, `src/rendering.ts`, `src/api.ts`, `src/server.ts` — unrelated; they do not host these helpers.

## Git workflow
Create a branch `advisor/005-dedupe-xml-escape-and-site-url`. Make focused commits, e.g.:
- `refactor(feed): share escapeXml and normalizeSiteUrl instead of local copies`

Do NOT push or open a PR unless the operator asks.

## Steps

1. **Create the shared module.** Create `src/xml.ts` exporting `escapeXml`, copied byte-for-byte from the HEAD body (do not rewrite the regex chain). Include the existing doc comment so JSDoc `{@link escapeXml}` references in `feed.ts`/`sitemap.ts` keep resolving. Content:
   ```ts
   /**
    * Shared XML-escaping helper used by the Atom feed and the sitemap.
    *
    * Both `src/feed.ts` and `src/sitemap.ts` build XML by hand from template
    * literals; every interpolated value must be escaped through this single
    * function so the two surfaces can never drift in their escaping rules.
    */

   /** Escape the five characters that are unsafe in XML text/attribute contexts. */
   export function escapeXml(value: string): string {
     return value
       .replace(/&/g, '&amp;')
       .replace(/</g, '&lt;')
       .replace(/>/g, '&gt;')
       .replace(/"/g, '&quot;')
       .replace(/'/g, '&apos;');
   }
   ```
   **Verify**: `pnpm run typecheck` -> exit 0.

2. **Point `src/sitemap.ts` at the shared module.** Add `import { escapeXml } from './xml.js';` (keep the existing `import { normalizeSiteUrl } from './site-url.js';`). Delete the local `escapeXml` function (the `:54-62` block). Decide on its `export`: see Step 3 for whether to re-export it. For now, after deleting the local copy, the file still references `escapeXml` internally via the import.
   **Verify**: `pnpm run typecheck` -> exit 0.

3. **Resolve the test imports (re-export vs. update tests).** `escapeXml` is currently a public export of both `feed.ts` and `sitemap.ts`, consumed by `src/feed.test.ts` (from `./feed.js`) and `src/sitemap.test.ts` (from `./sitemap.js`). These are internal test imports, not a published API contract, so the cleaner option is to **update the two test imports to point at `./xml.js`** and NOT re-export `escapeXml` from `feed.ts`/`sitemap.ts`. Do that:
   - In `src/sitemap.test.ts:15`, remove `escapeXml` from the `./sitemap.js` import and add `import { escapeXml } from './xml.js';`.
   - In `src/feed.test.ts:15-21`, remove `escapeXml` from the `./feed.js` import and add `import { escapeXml } from './xml.js';`.
   (If you instead prefer to keep `escapeXml` re-exported from both modules — `export { escapeXml } from './xml.js';` — and leave the tests unchanged, that is acceptable, but pick ONE approach and apply it consistently. The recommended approach is updating the tests and dropping the re-exports, so neither `feed.ts` nor `sitemap.ts` exports `escapeXml`.)
   **Verify**: `pnpm test src/sitemap.test.ts` -> pass.

4. **Point `src/feed.ts` at the shared modules.** Add `import { escapeXml } from './xml.js';`. Delete feed.ts's local `escapeXml` (`:60-68`). Delete feed.ts's private `normalizeSiteUrl` (`:70-74`) and add `normalizeSiteUrl` to an import from `./site-url.js`. Delete the private `const DEFAULT_SITE_URL` at `:29`; if `feed.ts` still references `DEFAULT_SITE_URL` anywhere (e.g. the `{@link DEFAULT_SITE_URL}` doc comment at `:51` or any code), import `DEFAULT_SITE_URL` from `./site-url.js` too. The call site `const base = normalizeSiteUrl(options.siteUrl);` (`:83`) stays unchanged. Per Step 3's recommended approach, do NOT export `escapeXml` from `feed.ts`.
   **Verify**: `pnpm run typecheck` -> exit 0 (this catches any dangling `DEFAULT_SITE_URL` reference).

5. **Confirm no behavioral drift in feed tests.**
   **Verify**: `pnpm test src/feed.test.ts` -> pass.

6. **Full gate.**
   **Verify**: `pnpm run ci` -> exit 0.

## Test plan
No new behavior is introduced, so no new test files are required. The existing escaping assertions ARE the regression coverage for this exact finding (one shared function now backs both surfaces):
- `src/feed.test.ts:77-84` — `escapeXml` cases (`& < > " '` -> `&amp; &lt; &gt; &quot; &apos;`, `<a>` -> `&lt;a&gt;`, `a & b` -> `a &amp; b`). Must still pass after the import is repointed to `./xml.js`.
- `src/sitemap.test.ts:66+` — `escapeXml` cases. Must still pass after the import is repointed to `./xml.js`.
- The feed/sitemap builder + handler tests in both files assert the rendered XML; they must remain byte-identical (no expected-string edits). Model new wiring after `src/sitemap.ts`'s existing `import { normalizeSiteUrl } from './site-url.js';`.

Verification commands: `pnpm test src/feed.test.ts src/sitemap.test.ts`, then `pnpm run typecheck`, then `pnpm test`.

## Done criteria
- [ ] `src/xml.ts` exists and exports a single `escapeXml`.
- [ ] No `escapeXml` function definition remains in `src/feed.ts` or `src/sitemap.ts` (`grep -n "function escapeXml" src/feed.ts src/sitemap.ts` returns nothing).
- [ ] No private `normalizeSiteUrl` remains in `src/feed.ts` (`grep -n "function normalizeSiteUrl" src/feed.ts` returns nothing); `feed.ts` imports it from `./site-url.js`.
- [ ] No private `const DEFAULT_SITE_URL` remains in `src/feed.ts` (`grep -n "const DEFAULT_SITE_URL" src/feed.ts` returns nothing); any remaining reference imports it from `./site-url.js`.
- [ ] `pnpm run typecheck` exits 0.
- [ ] `pnpm run lint` exits 0.
- [ ] `pnpm run format:check` exits 0.
- [ ] `pnpm test` passes (164+).
- [ ] `pnpm run build` exits 0.
- [ ] Feed/sitemap output assertions are unchanged (no expected-XML strings edited in tests).
- [ ] No out-of-scope files modified (`git status` shows only the in-scope files plus `plans/README.md`).
- [ ] `plans/README.md` row for plan 005 updated to Done. If `plans/README.md` does not exist, create it with a header and a table whose columns are `Plan | Title | Priority | Effort | Status`, and add the row: `005 | Consolidate duplicated escapeXml and normalizeSiteUrl | P2 | S | Done`.

## STOP conditions
- **escapeXml bodies differ at HEAD.** Re-read `src/feed.ts:60-68` and `src/sitemap.ts:54-62`. The advisor verified them byte-identical at `6bf6a27`. If at execution time they are NOT identical, consolidating could change output — STOP and report the exact difference instead of merging them.
- **normalizeSiteUrl bodies differ.** If `feed.ts`'s private `normalizeSiteUrl` or its `DEFAULT_SITE_URL` value no longer matches `src/site-url.ts` (different trim/replace logic or a different default origin), STOP and report — replacing it with the canonical import would change behavior.
- **Any feed/sitemap output test starts failing on an expected-string mismatch** (not an import error). That means output changed; revert and STOP.
- **Drift check tripped** (any in-scope file changed since `6bf6a27` and no longer matches the excerpts). STOP and report.

## Maintenance notes
- After this change, all XML escaping lives in `src/xml.ts` and all site-URL normalization in `src/site-url.ts`. A reviewer should confirm no second copy of `escapeXml` reappears in any new XML-producing module (e.g. a future RSS/JSON-feed surface) and that new modules import from `./xml.js`.
- The `src/site-url.ts` header comment (lines 9-11) currently says "`src/feed.ts` predates this module and keeps its own copy." Once `feed.ts` imports from `site-url.ts`, that note is stale — optionally update or remove it (low priority; not required for Done).
- Deferred follow-up: if more XML helpers accumulate (CDATA wrapping, attribute escaping vs. text escaping), `src/xml.ts` is the place to grow them.
