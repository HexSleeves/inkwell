# Inkwell v0.1 — Technical & Product Gap Audit

_Audit date: 2026-06-18 · Scope: `src/` at current `main`. Assessment only — no
production code changed in this task._

## 0. TL;DR

Inkwell v0.1 is a small, clean, well-tested core: framework-free request
handlers, a typed Postgres data-access layer, ordered migrations, allowlist HTML
sanitization, and ~1,600 lines of integration tests running against an in-memory
Postgres (`pg-mem`). The engineering quality is above what the v0.1 label
implies.

**Important correction to the task framing.** The issue brief lists "no auth on
write API" as the headline risk. That is **no longer true** — a shared-secret
`X-API-Key` scheme (`src/api.ts`) already gates every mutation, uses a
constant-time comparison, and **fails closed** when the key is unset. Reads are
open but draft documents are invisible to unauthenticated callers. So the
security headline is not "writes are open"; it is "writes are protected by a
**single shared god-key with no rotation, scoping, revocation, or audit
trail**." That reframing drives the recommendations below.

The real blockers to "worth using" are **product**, not security:
**discovery/SEO**, **authoring UX**, and **navigation (tags, pagination)**.

---

## 1. Code Health

### Strengths (keep doing this)

- **Clean seam between transport and logic.** `handleApiRequest`,
  `handlePageRequest`, `handleFeedRequest` are pure-ish functions over a
  normalized request + a `Queryable`. The `node:http` adapter (`server.ts`) is
  thin. This is why the tests can exercise real behavior without binding a
  socket.
- **Typed DAL + parameterized SQL everywhere** (`db/documents.ts`). No string
  interpolation into SQL; `snake_case`↔`camelCase` mapping is centralized.
- **Migrations are immutable, ordered, reversible** with a clear "never edit a
  shipped migration" convention (`db/migrations.ts`).
- **Sanitize-on-write, embed-verbatim-on-read.** `rendered_html` is sanitized
  once at write time; pages/feed escape every *other* interpolated value.

### Gaps & foot-guns

| # | Severity | Finding | Location | Recommendation |
|---|----------|---------|----------|----------------|
| C1 | High | **Unbounded index page.** The public index loads *all* published documents with no `limit` and renders them into one HTML page. Fine at 10 docs, a problem at 10,000. | `pages.ts` `handlePageRequest` → `listDocuments(db,{status:'published'})` | Add a `limit` + pagination (the DAL already supports `limit`/`offset`). |
| C2 | High | **Synchronous render is a CPU/DoS vector.** `bodyMarkdown` field length cap is `Number.MAX_SAFE_INTEGER` — effectively unbounded; only the 1 MB transport cap protects it. markdown-it + highlight.js + sanitize-html run **synchronously on the event loop** on every create/update. An authenticated client can stall the server with large/pathological input. | `api.ts` `requireString(..., Number.MAX_SAFE_INTEGER)`; `rendering.ts` | Set a real `bodyMarkdown` byte/char cap (e.g. 256 KB); consider offloading render or bounding highlight.js work. |
| C3 | Med | **Health check doesn't check the database.** `GET /health` returns `{status:'ok'}` without pinging Postgres, so a liveness probe stays green while the DB is down. | `api.ts` health branch | `SELECT 1` (with timeout) before reporting ok, or split liveness vs readiness. |
| C4 | Med | **No observability.** Only `console.log` on boot/shutdown; no structured request logging, no error correlation id. A 500 is opaque to operators. | `server.ts` catch | Add minimal structured logging + a request id echoed on 500s. |
| C5 | Med | **No coverage gate.** `test:coverage` exists but the `ci` script runs `test` without a threshold, so coverage can silently regress. | `package.json` | Add a coverage threshold to the `ci` pipeline. |
| C6 | Low | **No CORS handling** on an "API-first" service. Browser clients on another origin cannot call the API. Acceptable if same-origin-only is intended; should be a documented decision. | `server.ts`/`api.ts` | Decide and document; add configurable allowed origins if browser clients are a goal. |
| C7 | Low | **No caching/conditional-GET headers** on public pages or the feed (no `ETag`/`Last-Modified`/`Cache-Control`). Every hit is a full render+DB read; no CDN/304 path. | `server.ts` write helpers | Emit `Last-Modified`/`ETag` from `updatedAt`; honor `If-None-Match`. (Also a perf/SEO win — see quick wins.) |
| C8 | Low | **`getDocumentById` is exported but unused** by any route. Either wire it (id-based fetch) or drop it. | `db/documents.ts` | Remove or use. |

**Type safety:** strong overall — `strict` TS, `unknown` bodies narrowed at the
edge, branded status via `asDocumentStatus`. No `any` leakage spotted in the
reviewed files.

---

## 2. Security

### 2.1 Write/publish auth — the headline, reframed

**Status: implemented, not open.** `src/api.ts`:

- Every mutation (`POST`/`PATCH`/`PUT`/`DELETE` + `publish`/`unpublish`) calls
  `requireApiKey`, which checks `X-API-Key` against `INKWELL_API_KEY`.
- Comparison is **constant-time**: both sides SHA-256-hashed, then
  `timingSafeEqual` over fixed-length digests (avoids length-leak and the throw
  on length mismatch). Good.
- **Fails closed:** if the server key is unset/empty, no key can match → all
  writes 401. A misconfigured server does not silently serve open writes.
- Reads stay open but **draft documents never leak** (single get → 404; list/feed
  → filtered in SQL). A valid key unlocks draft visibility.

**Residual risk — single shared god-key:**

| # | Severity | Finding | Recommendation |
|---|----------|---------|----------------|
| S1 | High | One key grants *all* write power to *all* documents. A leak = full compromise, and the only revocation is redeploying with a new `INKWELL_API_KEY`. No per-author identity, no scopes, no audit of who wrote what. | Move to a **token model**: hashed tokens stored in a table, each with an owner, scopes (e.g. publish vs author), and a revoked flag. Keep the shared key as a bootstrap/admin fallback. |
| S2 | Med | **No write audit log.** There is no record of which key performed which mutation. | Log mutations (key id, action, slug, timestamp) — pairs naturally with S1. |
| S3 | Med | **No rate limiting.** Brute force / abuse is unthrottled. The 256-bit key makes guessing infeasible and the compare is constant-time, so this is lower priority, but write-endpoint flooding (see C2) is still possible. | Add basic per-IP / per-key rate limiting at the edge or in-process. |

### 2.2 Injection & sanitization (rendering)

- **SQL injection:** not present — every query is parameterized. ✔
- **XSS:** markdown is parsed with `html:true` (raw inline HTML allowed) **then**
  run through `sanitize-html` with a strict **allowlist** (`rendering.ts`).
  Allowlist-based = anything not explicitly permitted is dropped: `script`,
  `iframe`, `style`, `form`, event handlers, and `javascript:`/`data:` link
  schemes are all stripped. Links are hardened with
  `rel="noopener noreferrer nofollow"`. Images restricted to `http(s)`. This is
  the correct design. ✔
- **Defense-in-depth in the feed:** `renderedHtml` is additionally XML-escaped
  before going into `<content type="html">`. ✔

| # | Severity | Finding | Recommendation |
|---|----------|---------|----------------|
| S4 | Low | `class` is allowed on `span`/`code`/`pre` to carry highlight.js tokens. Class values are author-controllable but inert (styles are inlined and don't expose dangerous attribute selectors). No CSP is sent, so the residual XSS blast radius relies entirely on the sanitizer being complete. | Add a `Content-Security-Policy` header (e.g. `default-src 'self'; style-src 'unsafe-inline'` while styles are inlined) as a second layer behind the sanitizer. |
| S5 | Low | No TLS in-app (expected — terminate at a reverse proxy). | Document the reverse-proxy/TLS expectation in the deploy guide. |
| S6 | Info | `markdown-it` is configured once at module load; a future flip to `html:false` would not reduce safety (sanitizer is the net) but would change author capability. No action — noted for awareness. | — |

**Secrets hygiene:** API key is env-only; no secrets in the repo or migrations. ✔

---

## 3. Product Gaps (ranked: what's missing to be "worth using")

A publishing platform lives or dies on **being found and being easy to write
for**. Ranked by distance-to-"worth using":

1. **Discovery / SEO (biggest lever).** No `sitemap.xml`, no `robots.txt`, no
   per-page `<meta name="description">`, no Open Graph / Twitter Card tags, no
   canonical URL, no JSON-LD. Pages are clean HTML but not optimally
   findable/shareable. The Atom feed and an index page already exist — a good
   foundation, but the SEO surface is the growth multiplier.
2. **Authoring UX.** Authoring is API/`curl`-only. No web editor, no admin
   dashboard, no live preview, no official CLI. This is the single biggest
   adoption barrier for non-developer authors. "API-first" is fine as an
   architecture, but a thin admin UI or a published CLI is needed for humans.
3. **Navigation & structure.** Flat list only — no tags, categories, series, or
   collections, and the index page is unpaginated (see C1). No "next/prev",
   no archive-by-date. Readers can't browse a growing site.
4. **Search.** None. Expected on any content site past a handful of pages.
5. **Media / images.** Markdown can *reference* images, but there is no upload or
   asset hosting — authors must self-host every image elsewhere. No image
   pipeline (resize, optimize).
6. **Multi-author / accounts.** The single shared key (S1) blocks team or
   multi-tenant use; no author identity is attached to documents.
7. **Shareable draft previews.** Drafts are visible only with the full god-key.
   No per-draft preview tokens to share work-in-progress with a reviewer.
8. **Theming / branding.** One inlined theme; only `INKWELL_SITE_URL` is
   configurable. No per-site title, logo, or custom CSS.
9. **Publish events / webhooks & analytics.** No hooks on publish, no view
   counts — limits integrations and feedback loops.
10. **Feed richness.** Atom entries lack `<author>` and `<summary>`; full content
    only.

---

## 4. Prioritized Recommendation — Top 5 by Growth Impact vs. Effort

| Rank | Initiative | Growth impact | Effort | Why now |
|------|-----------|---------------|--------|---------|
| 1 | **SEO surface**: sitemap.xml, robots.txt, per-page meta description + canonical, Open Graph/Twitter cards, JSON-LD article schema | **High** | **Low–Med** | Discovery is the growth flywheel for a publishing tool; mostly templating on top of data that already exists. Highest ROI. |
| 2 | **Minimal authoring UI** (or a first-class CLI + live preview) | **High** | **Med–High** | Removes the #1 adoption barrier — humans can't be expected to author via `curl`. Even a thin authenticated admin page is transformative. |
| 3 | **Navigation: index pagination + tags/collections** (also fixes C1 unbounded index) | **Med–High** | **Med** | Lets a site scale past a handful of posts and gives readers a reason to stay; closes a real foot-gun at the same time. |
| 4 | **Token/key model**: hashed per-author tokens with scopes, revocation, and a write audit log (S1/S2) | **Med** | **Med** | Unblocks multi-author use and makes ops safe (revoke without redeploy). Security + product in one. |
| 5 | **Image/media upload + hosting** | **Med** | **Med–High** | Removes a constant authoring friction; images are table-stakes for most posts. |

**Quick wins (low effort, do alongside the above):** caching/conditional-GET
headers on pages + feed (C7 — also an SEO/perf win), a DB-aware health check
(C3), a coverage gate in CI (C5), a real `bodyMarkdown` size cap (C2), and a CSP
header (S4).

---

## Appendix — Surface reviewed

- API + auth: `src/api.ts`, `src/server.ts`, `src/main.ts`
- Rendering/sanitization: `src/rendering.ts`
- Public pages + feed: `src/pages.ts`, `src/feed.ts`
- Data + schema: `src/db/documents.ts`, `src/db/migrations.ts`
- Tests: `src/*.test.ts`, `src/db/*.test.ts` (~1,577 lines)
- Config/docs: `package.json`, `README.md`, `docs/adr/`
