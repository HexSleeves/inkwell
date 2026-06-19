# Plan 007: Add ETag + Cache-Control + conditional GET (304) to public HTML/XML responses

> **Executor instructions**: Follow the Steps in order; the tree must compile and pass tests after every step. Run every **Verify** command and do not advance past a failing one. Obey the STOP conditions — they protect against the one real design trap here (handler-interface churn). When done, create/update the status row in `plans/README.md` for this plan. Do NOT commit, push, or open a PR unless the operator explicitly asks.

> **Drift check (run first)**: `git diff --stat 6bf6a27..HEAD -- src/server.ts src/api.test.ts` — if any in-scope file changed, compare the Current-state excerpts below to the live code before editing; on mismatch that contradicts an excerpt you depend on, STOP and report.
>
> NOTE (known drift): this plan was authored against base commit `6bf6a27`, but the live tree already contains a later change that added a full-text **search** endpoint to `src/server.ts` (a new `writeStringResponse` helper plus a `GET /search` branch in the listener). That drift is expected and does NOT block this plan — `writeHtmlResponse`, `writeXmlResponse`, and the listener dispatch for pages/feed/sitemap are unchanged in shape. The excerpts below reflect the live (post-search) file. If you also want caching on `/search`, that is OPTIONAL (see Steps step 6); the search response is the same `{ status, contentType, body }` string shape, so it is trivial to include, but it is not required by this plan's acceptance criteria.

## Status
- **Priority**: P2
- **Effort**: M
- **Risk**: LOW
- **Depends on**: none
- **Category**: perf
- **Planned at**: commit `6bf6a27`, 2026-06-19

## Why this matters
Public HTML pages, the Atom feed, and the sitemap currently send only `content-type` + `content-length` — no `ETag`, `Last-Modified`, or `Cache-Control`. Every hit (including repeat crawler hits and CDN revalidations) costs a full render plus a DB read, and there is no cheap `304 Not Modified` path. Adding a body-derived weak `ETag` with a conservative `Cache-Control` lets browsers, crawlers, and CDNs revalidate cheaply: matching `If-None-Match` returns a bodyless `304`, cutting response bandwidth and improving crawl-budget/SEO without ever serving stale published content. The render still runs (the ETag is computed from the rendered body), so this is a bandwidth + client/CDN caching win, not a render-skip — that is called out honestly so nobody over-claims it.

## Current state
Files and their roles:
- `src/server.ts` — the `node:http` transport adapter (the only module touching Node HTTP types). It reads the body, normalizes the request, delegates routing/logic to handlers, and serializes responses to the wire. This is the **only** source file changed by this plan.
- `src/pages.ts` / `src/feed.ts` / `src/sitemap.ts` — pure handlers that return response objects (`PageResponse`, `FeedResponse`, `SitemapResponse`). Their shapes must NOT change.
- `src/api.test.ts` — holds the existing `node:http` transport suite (`describe('documents HTTP API (node:http transport)')`) that binds a real server and drives it with `fetch`. This is the exemplar to model the new tests after.

Verified excerpts at the relevant lines (live tree):

`src/server.ts` — `writeHtmlResponse` (around lines 96-104):
```ts
/** Serialize an HTML page response from the public frontend. */
function writeHtmlResponse(res: ServerResponse, method: string, response: PageResponse): void {
  const payload = Buffer.from(response.html, 'utf8');
  res.writeHead(response.status, {
    'content-type': 'text/html; charset=utf-8',
    'content-length': payload.length,
  });
  // HEAD requests get headers (including content-length) but no body.
  res.end(method === 'HEAD' ? undefined : payload);
}
```

`src/server.ts` — `writeXmlResponse` (around lines 110-122):
```ts
function writeXmlResponse(
  res: ServerResponse,
  method: string,
  response: FeedResponse | SitemapResponse,
): void {
  const payload = Buffer.from(response.xml, 'utf8');
  res.writeHead(response.status, {
    'content-type': response.contentType,
    'content-length': payload.length,
  });
  // HEAD requests get headers (including content-length) but no body.
  res.end(method === 'HEAD' ? undefined : payload);
}
```

`src/server.ts` — listener dispatch (around lines 143-185). The listener has `method` (uppercased) and `req` (so `req.headers['if-none-match']` is reachable). It dispatches feed/sitemap to `writeXmlResponse` and pages to `writeHtmlResponse`:
```ts
export function createRequestListener(db: Queryable) {
  return async (req: IncomingMessage, res: ServerResponse): Promise<void> => {
    try {
      const segments = splitPath(req.url ?? '/');
      const method = (req.method ?? 'GET').toUpperCase();
      const siteUrl = process.env.INKWELL_SITE_URL;

      if (segments.length === 1 && segments[0] === 'feed.xml') {
        const feedResponse = await handleFeedRequest(db, { method }, { siteUrl });
        writeXmlResponse(res, method, feedResponse);
        return;
      }
      // ... sitemap.xml -> writeXmlResponse ; search -> writeStringResponse ...
      if (segments.length === 0 || !API_PREFIXES.has(segments[0] as string)) {
        const pageResponse = await handlePageRequest(db, { method, segments }, { siteUrl });
        writeHtmlResponse(res, method, pageResponse);
        return;
      }
      // ... JSON API path below (writeResponse) ...
```

The JSON `writeResponse` (around lines 81-93) and the catch-block error responses (413/500, around lines 210-218) must NOT get caching headers — only `200` GET/HEAD HTML/XML.

Exemplar test pattern — the existing transport suite in `src/api.test.ts` (around lines 641-668) binds a real server and drives it with `fetch`:
```ts
describe('documents HTTP API (node:http transport)', () => {
  let server: Server;
  let baseUrl: string;
  let previousApiKey: string | undefined;

  beforeEach(async () => {
    previousApiKey = process.env.INKWELL_API_KEY;
    process.env.INKWELL_API_KEY = API_KEY;
    const db = createMemoryDatabase().db;
    await migrate(db);
    server = createServer(db);
    await new Promise<void>((resolve) => server.listen(0, '127.0.0.1', resolve));
    const { port } = server.address() as AddressInfo;
    baseUrl = `http://127.0.0.1:${port}`;
  });

  afterEach(async () => {
    if (previousApiKey === undefined) {
      delete process.env.INKWELL_API_KEY;
    } else {
      process.env.INKWELL_API_KEY = previousApiKey;
    }
    await new Promise<void>((resolve, reject) =>
      server.close((err) => (err ? reject(err) : resolve())),
    );
  });
  // ... `it(...)` blocks call `fetch(`${baseUrl}/...`)` and assert `res.status` / `res.headers.get(...)`.
});
```
Imports already present in `src/api.test.ts`: `createServer` from `./server.js`, `migrate` from `./db/migrate.js`, `createMemoryDatabase` from `./db/test-helpers.js`, plus `type Server` from `node:http` and `type AddressInfo` from `node:net`.

Conventions to match:
- ESM imports use explicit `.js` extensions; type-only imports use `import type` (verbatimModuleSyntax). `node:crypto` is a Node built-in: `import { createHash } from 'node:crypto';`.
- `noUncheckedIndexedAccess` is on, so `req.headers['if-none-match']` is typed `string | string[] | undefined` — narrow it before comparing.

## Commands you will need
| Purpose | Command | Expected |
| Install | `pnpm install` | exit 0 |
| Typecheck | `pnpm run typecheck` | exit 0, no errors |
| Lint | `pnpm run lint` | exit 0 |
| Format check | `pnpm run format:check` | exit 0 |
| Tests (one file) | `pnpm test src/api.test.ts` | pass |
| Tests (all) | `pnpm test` | all pass (164+) |
| Coverage | `pnpm run test:coverage` | thresholds met (stmts/lines/funcs 80, branches 75) |
| Build | `pnpm run build` | exit 0 |

## Scope
**In scope**:
- `src/server.ts` — add ETag/Cache-Control + conditional-GET handling inside the transport (`writeHtmlResponse`, `writeXmlResponse`, and a small shared helper). No handler-shape changes.
- `src/api.test.ts` (extend) — add the new transport-level caching tests in a new `describe` block, reusing the existing bind/teardown pattern. (If you strongly prefer a separate file, `src/server.test.ts` (create) is acceptable, but extending `src/api.test.ts` is simpler because the bind harness already exists there.)

**Out of scope** (do NOT touch):
- `src/pages.ts`, `src/feed.ts`, `src/sitemap.ts` — their `PageResponse` / `FeedResponse` / `SitemapResponse` interfaces must stay unchanged; ETag is computed in the transport, not in handlers.
- The JSON API path (`writeResponse` / `handleApiRequest`) — API responses get no caching headers.
- The `/search` endpoint / `writeStringResponse` — OPTIONAL only (see Steps step 6); not required.
- Error responses (404/405/500/503/413) — caching applies only to `200` GET/HEAD HTML/XML.
- `Last-Modified` — intentionally deferred in favor of a body-hash ETag (see Maintenance notes).

## Git workflow
- Branch: `advisor/007-http-caching-conditional-get`.
- Conventional-commit example: `perf(server): add weak ETag + Cache-Control and conditional GET (304) to public HTML/XML`.
- Do NOT push or open a PR unless the operator asks.

## Steps

1. **Add the crypto import.** At the top of `src/server.ts`, add a value import for the hash function alongside the existing imports:
   ```ts
   import { createHash } from 'node:crypto';
   ```
   Place it with the other `node:*` imports (keep import grouping/ordering consistent with the file so lint/format pass).
   **Verify**: `pnpm run typecheck` -> exit 0.

2. **Add an ETag helper.** Add a small pure helper near the other `write*` helpers (above `writeHtmlResponse`). It computes a weak ETag from the payload bytes — first 32 hex chars of a sha256 of the body, plus the byte length — using a stable, conservative `Cache-Control`:
   ```ts
   /** A conservative caching policy: cache but always revalidate (cheap 304s), never serve stale published content. */
   const PUBLIC_CACHE_CONTROL = 'public, max-age=0, must-revalidate';

   /** Compute a weak ETag from already-rendered bytes: `W/"<sha256-prefix>-<len>"`. */
   function weakETag(payload: Buffer): string {
     const hash = createHash('sha256').update(payload).digest('hex').slice(0, 32);
     return `W/"${hash}-${payload.length}"`;
   }

   /**
    * If the request's `If-None-Match` matches `etag`, send a bodyless 304 (with the
    * ETag echoed back) and return true. Otherwise send nothing and return false.
    */
   function respondNotModifiedIfMatch(
     req: IncomingMessage,
     res: ServerResponse,
     etag: string,
   ): boolean {
     const ifNoneMatch = req.headers['if-none-match'];
     if (typeof ifNoneMatch === 'string' && ifNoneMatch === etag) {
       res.writeHead(304, { etag, 'cache-control': PUBLIC_CACHE_CONTROL });
       res.end();
       return true;
     }
     return false;
   }
   ```
   Note: `if-none-match` is compared with strict equality against the single weak ETag we emit. We do not implement full RFC list/`*` parsing — a single value is what browsers/CDNs send back for a single-ETag resource, and keeping it strict avoids accidentally matching the wrong representation. Document this in a brief inline comment.
   **Verify**: `pnpm run typecheck` -> exit 0 (helper is defined but unused yet — if the unused-var lint trips, proceed to the next steps which wire it up before running lint).

3. **Wire caching into `writeHtmlResponse`.** The helpers need `req` to read `if-none-match`. Change the signature to accept `req`, and apply caching only for `200` GET/HEAD. Keep the existing non-200 behavior byte-for-byte. New body:
   ```ts
   function writeHtmlResponse(
     req: IncomingMessage,
     res: ServerResponse,
     method: string,
     response: PageResponse,
   ): void {
     const payload = Buffer.from(response.html, 'utf8');
     const cacheable = response.status === 200 && (method === 'GET' || method === 'HEAD');
     if (cacheable) {
       const etag = weakETag(payload);
       if (respondNotModifiedIfMatch(req, res, etag)) return;
       res.writeHead(response.status, {
         'content-type': 'text/html; charset=utf-8',
         'content-length': payload.length,
         etag,
         'cache-control': PUBLIC_CACHE_CONTROL,
       });
     } else {
       res.writeHead(response.status, {
         'content-type': 'text/html; charset=utf-8',
         'content-length': payload.length,
       });
     }
     // HEAD requests get headers (including content-length) but no body.
     res.end(method === 'HEAD' ? undefined : payload);
   }
   ```
   **Verify**: `pnpm run typecheck` -> exit 0.

4. **Wire caching into `writeXmlResponse`** with the same pattern (feed + sitemap), preserving `response.contentType`:
   ```ts
   function writeXmlResponse(
     req: IncomingMessage,
     res: ServerResponse,
     method: string,
     response: FeedResponse | SitemapResponse,
   ): void {
     const payload = Buffer.from(response.xml, 'utf8');
     const cacheable = response.status === 200 && (method === 'GET' || method === 'HEAD');
     if (cacheable) {
       const etag = weakETag(payload);
       if (respondNotModifiedIfMatch(req, res, etag)) return;
       res.writeHead(response.status, {
         'content-type': response.contentType,
         'content-length': payload.length,
         etag,
         'cache-control': PUBLIC_CACHE_CONTROL,
       });
     } else {
       res.writeHead(response.status, {
         'content-type': response.contentType,
         'content-length': payload.length,
       });
     }
     res.end(method === 'HEAD' ? undefined : payload);
   }
   ```
   **Verify**: `pnpm run typecheck` -> exit 0.

5. **Update the three call sites in the listener** to pass `req`:
   - `writeXmlResponse(res, method, feedResponse);` -> `writeXmlResponse(req, res, method, feedResponse);`
   - `writeXmlResponse(res, method, sitemapResponse);` -> `writeXmlResponse(req, res, method, sitemapResponse);`
   - `writeHtmlResponse(res, method, pageResponse);` -> `writeHtmlResponse(req, res, method, pageResponse);`
   Do NOT change the `writeResponse(...)` (JSON) or `writeStringResponse(...)` (search) call sites.
   **Verify**: `pnpm run typecheck` -> exit 0, and `pnpm run lint` -> exit 0 (the helper is now used).

6. **(OPTIONAL — not required) `/search` caching.** If you choose to also cache the search HTML/JSON, give `writeStringResponse` the same `(req, res, method, response)` signature + 200-GET/HEAD gating and update its one call site. If you do this, add a fourth test case mirroring the page cases. If in doubt, SKIP this step — it is out of the required scope.
   **Verify** (only if done): `pnpm run typecheck` -> exit 0; `pnpm run lint` -> exit 0.

7. **Run the format/build gates** before tests to catch formatting early.
   **Verify**: `pnpm run format:check` -> exit 0; `pnpm run build` -> exit 0.

8. **Add the tests** (see Test plan), then run the full suite.
   **Verify**: `pnpm test src/api.test.ts` -> pass; then `pnpm test` -> all pass.

## Test plan
Add a new `describe('public response caching (node:http transport)')` block in `src/api.test.ts`, modeled exactly on the existing `documents HTTP API (node:http transport)` suite (same `beforeEach`/`afterEach` bind/teardown, same `createMemoryDatabase().db` + `migrate(db)` + `createServer(db)` + `server.listen(0, '127.0.0.1', ...)` pattern). You will need a published document so `GET /` (the index page) returns `200` — create + publish one over HTTP first (reuse the `POST /documents` then `POST /documents/<slug>/publish` flow already used by the round-trip test, with `headers: { 'x-api-key': API_KEY }`), or hit a known-200 path. Use `GET /` and `GET /feed.xml` and `GET /sitemap.xml` as cacheable targets.

Cases (regression coverage for THIS finding):
1. **Headers present on 200**: `GET /` returns `200`; assert `res.headers.get('etag')` is a non-null string starting with `W/"` and `res.headers.get('cache-control')` equals `public, max-age=0, must-revalidate`. Repeat for `GET /feed.xml` and `GET /sitemap.xml`.
2. **Conditional GET -> 304**: do a first `GET /`, read `etag = res.headers.get('etag')`, then a second `GET /` with `headers: { 'if-none-match': etag }`; assert status `304`, that the response body is empty (`await res2.text()` -> `''`), and that `res2.headers.get('etag')` still equals `etag`.
3. **Non-matching If-None-Match -> 200 with body**: `GET /` with `headers: { 'if-none-match': 'W/"deadbeef-1"' }`; assert status `200` and a non-empty body (`(await res.text()).length > 0`).
4. **(only if step 6 done)** same three assertions for `GET /search?q=...`.

Edge note: `fetch` in Node does not auto-handle `If-None-Match`, so the assertions above observe the raw status/headers as intended. Model assertions after existing `res.status` / `await res.json()` usage in the file; use `res.headers.get(...)` (lowercased header names) and `await res.text()`.

Verification: `pnpm test src/api.test.ts` -> pass; `pnpm run typecheck` -> exit 0; then `pnpm test` -> all pass; finally `pnpm run test:coverage` -> thresholds met.

## Done criteria
- [ ] `pnpm run typecheck` -> exit 0.
- [ ] `pnpm run lint` -> exit 0.
- [ ] `pnpm run format:check` -> exit 0.
- [ ] `pnpm test` -> all pass (164+, including the 3+ new caching tests).
- [ ] `pnpm run test:coverage` -> thresholds met (stmts/lines/funcs 80, branches 75).
- [ ] `pnpm run build` -> exit 0.
- [ ] `pnpm run ci` -> exit 0.
- [ ] `git status` shows only in-scope files modified: `src/server.ts`, `src/api.test.ts` (and `plans/README.md` + this plan). No out-of-scope files modified.
- [ ] `plans/README.md` status row for plan 007 updated to Done (create the file/table if it does not yet exist).

## STOP conditions
- If implementing conditional GET cleanly seems to require changing `PageResponse`, `FeedResponse`, or `SitemapResponse` (e.g., to carry a precomputed ETag or `Last-Modified`) in more than a trivial way — STOP and report the design tension. The whole point is to keep ETag computation in the transport so handler shapes stay frozen.
- If the change cascades beyond `src/server.ts` + the test file (e.g., you find yourself editing `src/pages.ts`/`src/feed.ts`/`src/sitemap.ts` or the JSON API) — STOP and report.
- If the drift check reveals `writeHtmlResponse`/`writeXmlResponse` no longer match the excerpts above (signature or 200/HEAD handling changed by another commit) — STOP and reconcile before editing.
- If a test reveals that some "200" page path is actually served with a different status, or that `GET /` is not 200 in the empty-DB case you set up — fix the test fixture (publish a document) rather than weakening the 200-only gate.

## Maintenance notes
- **Render is not skipped.** The ETag is derived from the already-rendered body, so a `304` still costs the full render + DB read; only bandwidth and downstream caching improve. A future optimization could compute a cheaper upstream key (e.g., max `updated_at` across published docs) to short-circuit before render — if that is ever added, revisit `respondNotModifiedIfMatch` so the upstream key and the body-hash ETag don't disagree.
- **`Last-Modified` intentionally deferred.** We chose a body-hash weak ETag over `Last-Modified` because the handlers don't currently surface a reliable modification timestamp to the transport, and a content hash can't go stale the way a clock-based header can. If `Last-Modified` is later wanted, it would pair naturally with the upstream-key optimization above.
- **Reviewer focus**: confirm caching headers appear ONLY on 200 GET/HEAD HTML/XML and never on the JSON API, error pages (404/405/500/503/413), or HEAD/GET non-200s; confirm the `If-None-Match` comparison is strict-equality against the single emitted ETag (no accidental list/`*` semantics); confirm `304` responses carry no body and echo the ETag.
- **Deferred follow-ups**: optional `/search` caching (step 6); strong vs. weak ETag if byte-exact validators are ever needed; CDN `s-maxage`/`stale-while-revalidate` tuning once a CDN is actually in front of the app.
