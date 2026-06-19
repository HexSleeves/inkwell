# Plan 012: Add a Content-Security-Policy and hardening headers to HTML pages

> **Executor instructions**: Follow the steps in order, top to bottom. Run every **Verify** command and confirm the expected result before moving on. Obey the STOP conditions — if one triggers, stop and report rather than guessing. When finished, update the status row for this plan in `plans/README.md` (create that file/row if it does not exist yet).
> **Drift check (run first)**: `git diff --stat 6bf6a27..HEAD -- src/server.ts src/api.test.ts src/pages.ts` — if any in-scope file changed, compare the Current-state excerpts below to live code; on mismatch, STOP and report.

## Status
- **Priority**: P3
- **Effort**: S-M
- **Risk**: LOW
- **Depends on**: none
- **Category**: security
- **Planned at**: commit `6bf6a27`, 2026-06-19

## Why this matters
`sanitize-html` is currently the SOLE XSS defense for the public HTML surface (markdown is sanitized on write). A Content-Security-Policy plus a couple of standard hardening response headers is cheap, well-understood defense-in-depth sitting *behind* the sanitizer: if a sanitizer bypass or a future markup regression ever lets script through, a restrictive CSP (`default-src 'self'`, `object-src 'none'`, no inline/external script allowed) blocks execution, and `frame-ancestors 'none'` prevents clickjacking. This is low priority because sanitize-on-write already covers the primary risk, but it is standard hygiene for any public HTML endpoint and costs only a handful of header lines.

## Current state

Files and their roles:
- `src/server.ts` — the `node:http` transport adapter; the only place that touches Node HTTP types and the only place response headers are set. HTML, JSON, XML, and string responses each have their own `write*Response` helper.
- `src/pages.ts` — builds the public HTML. It uses an **inlined** `<style>` block (line 278: `<style>${STYLES}</style>`), so any CSP must permit inline styles (`style-src 'unsafe-inline'`). The sanitizer allows `<img>` with `http(s)` sources, so `img-src` must allow `http: https: data:`.
- `src/api.test.ts` — the test exemplar. The bottom suite (`describe('documents HTTP API (node:http transport)', ...)`, line 641) binds a real `node:http` server via `createServer(db)` on an ephemeral port and drives it with `fetch`. This is the pattern to model the new header test after.

`writeHtmlResponse` today sets only content-type and content-length — no security headers anywhere (a repo-wide grep confirms no CSP/`setHeader` for security):

```ts
// src/server.ts ~lines 95-104
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

For contrast, the JSON helper (`writeResponse`, ~line 81) and XML helper (`writeXmlResponse`, ~line 110) and string/search helper (`writeStringResponse`, ~line 129) must remain unchanged — CSP belongs on HTML only.

The transport-test exemplar to copy (from `src/api.test.ts`, line 641 onward) sets up the server like this:

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
  // ...fetch-based tests...
});
```

Imports already present at the top of `src/api.test.ts`: `afterEach, beforeEach, describe, expect, it` from `vitest`; `type { AddressInfo }` from `node:net`; `type { Server }` from `node:http`; `createServer` from `./server.js`; `migrate` from `./db/migrate.js`; `createMemoryDatabase` from `./db/test-helpers.js`.

## Commands you will need
| Purpose | Command | Expected |
| --- | --- | --- |
| Install | `pnpm install` | exit 0 |
| Typecheck | `pnpm run typecheck` | exit 0, no errors |
| Lint | `pnpm run lint` | exit 0 |
| Format check | `pnpm run format:check` | exit 0 |
| Tests (one file) | `pnpm test src/api.test.ts` | pass |
| Tests (all) | `pnpm test` | all pass (164+) |
| Full gate | `pnpm run ci` | exit 0 |

## Scope
**In scope**:
- `src/server.ts` — add the security headers inside `writeHtmlResponse` only.
- `src/api.test.ts` — add a transport-level test asserting the headers are present on an HTML page response (extend the existing `node:http transport` suite, or add a small new suite that binds a server the same way).

**Out of scope** (do NOT touch):
- `src/api.ts` and the JSON `writeResponse` helper — JSON API responses must NOT carry the CSP (it would be meaningless and could mask issues).
- `src/feed.ts`, `src/sitemap.ts`, and `writeXmlResponse` — XML feed/sitemap responses must NOT carry the CSP.
- `src/search.ts` and `writeStringResponse` — out of scope; CSP for the search HTML surface is a separate, optional follow-up, not this plan.
- `src/pages.ts` markup — the inline `<style>` stays; the policy is written to be compatible with it. Do not externalize the stylesheet.

## Git workflow
- Branch: `advisor/012-csp-security-headers`.
- Conventional commit example: `feat(server): add CSP and hardening headers to HTML page responses`.
- Do NOT push, commit beyond this branch, or open a PR unless the operator explicitly asks.

## Steps

1. **Drift check.** Run the drift-check command in the header. If `src/server.ts`, `src/api.test.ts`, or `src/pages.ts` changed since `6bf6a27`, compare them to the excerpts above; on any meaningful mismatch (e.g. `writeHtmlResponse` already sets security headers, or the inline `<style>` was removed), STOP and report.
   **Verify**: `git diff --stat 6bf6a27..HEAD -- src/server.ts src/api.test.ts src/pages.ts` -> empty output, or differences that do not contradict the excerpts.

2. **Add the security headers in `writeHtmlResponse`** (`src/server.ts`). Add the three headers to the `res.writeHead(...)` header object so they ship on every HTML page response. Keep `content-type` and `content-length` as-is. Use this exact policy string (inline-style compatible, no `script-src` allowance since pages have no inline/external script):

   - `'content-security-policy': "default-src 'self'; style-src 'unsafe-inline'; img-src http: https: data:; object-src 'none'; base-uri 'self'; frame-ancestors 'none'"`
   - `'x-content-type-options': 'nosniff'`
   - `'referrer-policy': 'strict-origin-when-cross-origin'`

   (`frame-ancestors 'none'` supersedes the legacy `X-Frame-Options` header, so do NOT add `X-Frame-Options`.) The resulting helper should look like:

   ```ts
   /** Serialize an HTML page response from the public frontend. */
   function writeHtmlResponse(res: ServerResponse, method: string, response: PageResponse): void {
     const payload = Buffer.from(response.html, 'utf8');
     res.writeHead(response.status, {
       'content-type': 'text/html; charset=utf-8',
       'content-length': payload.length,
       // Defense-in-depth behind the on-write sanitizer. The pages embed an
       // inline <style> block, so style-src must allow 'unsafe-inline'; there is
       // no inline or external script, so no script-src allowance is granted.
       // frame-ancestors 'none' supersedes the legacy X-Frame-Options header.
       'content-security-policy':
         "default-src 'self'; style-src 'unsafe-inline'; img-src http: https: data:; object-src 'none'; base-uri 'self'; frame-ancestors 'none'",
       'x-content-type-options': 'nosniff',
       'referrer-policy': 'strict-origin-when-cross-origin',
     });
     // HEAD requests get headers (including content-length) but no body.
     res.end(method === 'HEAD' ? undefined : payload);
   }
   ```

   Do not modify `writeResponse`, `writeXmlResponse`, or `writeStringResponse`.
   **Verify**: `pnpm run typecheck` -> exit 0; `pnpm run lint` -> exit 0; `pnpm run format:check` -> exit 0 (run `pnpm run format` / Prettier if it flags the multi-line string).

3. **Add the regression test** to `src/api.test.ts` (see Test plan for exact cases). Bind a server the same way the existing `node:http transport` suite does, `fetch` an HTML page (`GET /` index, or a published document page), and assert the CSP and `x-content-type-options` headers. Also assert the JSON API path does NOT carry the CSP header, to lock in the HTML-only scope.
   **Verify**: `pnpm test src/api.test.ts` -> all pass.

4. **Run the full gate.**
   **Verify**: `pnpm run ci` -> exit 0 (lint + format + typecheck + test:coverage + build all green).

5. **Update `plans/README.md`** — set this plan's status row to done/complete (create the file and a simple table row if it does not yet exist; the `plans/` directory is new).
   **Verify**: `git status --porcelain` shows only `src/server.ts`, `src/api.test.ts`, `plans/012-csp-security-headers.md`, and `plans/README.md` as changed/added — no other files.

## Test plan
- **File**: `src/api.test.ts` (extend the existing `describe('documents HTTP API (node:http transport)', ...)` suite at line 641, which already provides `baseUrl`, server setup/teardown, and the `API_KEY` env wiring).
- **New cases**:
  1. *HTML page carries the CSP + hardening headers* — `fetch(`${baseUrl}/`)` (the index page is reachable unauthenticated); assert `res.headers.get('content-type')` starts with `text/html`, `res.headers.get('content-security-policy')` is non-null and contains `style-src 'unsafe-inline'` and `default-src 'self'` and `frame-ancestors 'none'`, and `res.headers.get('x-content-type-options')` equals `nosniff`. This is the regression test for this exact finding.
  2. *JSON API response does NOT carry the CSP* — `fetch(`${baseUrl}/documents/ghost`)` (returns 404 JSON, no auth needed); assert `res.headers.get('content-security-policy')` is `null` and `content-type` starts with `application/json`. Locks in the HTML-only scope.
- **Inline-style still renders**: the policy includes `style-src 'unsafe-inline'`, so the existing `<style>${STYLES}</style>` block in `src/pages.ts` is permitted — no page markup change is needed and no existing page test should break. Confirm by running the full suite.
- **Model after**: the `node:http transport` suite already in `src/api.test.ts` (line 641+).
- **Verification command**: `pnpm test src/api.test.ts`, then `pnpm test`.

## Done criteria
- [ ] `writeHtmlResponse` in `src/server.ts` sets `content-security-policy`, `x-content-type-options: nosniff`, and `referrer-policy: strict-origin-when-cross-origin`; the policy string matches Step 2 exactly.
- [ ] `writeResponse` (JSON), `writeXmlResponse` (XML), and `writeStringResponse` (search) are unchanged — no CSP added there.
- [ ] `src/pages.ts` markup is unchanged (inline `<style>` retained).
- [ ] New test in `src/api.test.ts` asserts the CSP + nosniff headers on an HTML page AND absence of CSP on a JSON response.
- [ ] `pnpm run typecheck` exit 0.
- [ ] `pnpm run lint` exit 0.
- [ ] `pnpm run format:check` exit 0.
- [ ] `pnpm test` all pass; `pnpm run ci` exit 0.
- [ ] No out-of-scope files modified (`git status` shows only the in-scope files plus `plans/README.md`).
- [ ] `plans/README.md` status row for plan 012 updated to done.

## STOP conditions
- A stricter, **nonce-based** CSP is wanted (dropping `'unsafe-inline'` from `style-src`). That requires moving the inlined stylesheet to an external file or attaching a per-response nonce to the `<style>` tag, which is a larger change spanning `src/pages.ts` — out of scope here. Report and keep the inline-compatible policy unless the operator explicitly asks for the nonce approach.
- The drift check shows `writeHtmlResponse` already sets security headers, or the inline `<style>` block in `src/pages.ts` was removed/externalized — STOP and report; the policy may need to change (e.g. tighten `style-src` to `'self'`).
- A new inline `<script>` or external script/style is discovered in `src/pages.ts` that the policy above would block — STOP; the CSP would break the page and the policy must be reconsidered with the operator.
- `pnpm run ci` fails on the coverage gate because the new branch lowered coverage below thresholds (stmts/lines/funcs 80, branches 75) — investigate; the added test should cover the new header path, so a failure here likely means the test is not exercising it.

## Maintenance notes
- If the public pages later move to an **external** stylesheet, tighten `style-src` to `'self'` (or a nonce) and drop `'unsafe-inline'`; update the test assertion accordingly.
- Reviewers should scrutinize any future inline `<script>`/`<style>` or external resource added to `src/pages.ts` (or `src/search.ts` if it gains an HTML surface) and confirm it is accounted for in the CSP — an un-allowed inline script will silently fail to execute under this policy.
- Deferred follow-up (not this plan): consider adding the same headers to the search HTML response in `writeStringResponse`, and consider `Strict-Transport-Security` once TLS/origin handling is finalized (HSTS is only safe behind HTTPS).
- The CSP is set in the transport layer (`src/server.ts`), not the page builder, deliberately — it is a transport-level response concern and keeps `src/pages.ts` a pure HTML builder.
