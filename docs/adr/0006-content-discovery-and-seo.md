# 0006 — Content discovery & SEO

Status: accepted

## Context

Inkwell can publish Markdown documents, serve them as styled HTML pages, and
emit an Atom feed — but published content had no way to be _found_. There was no
paginated index beyond a flat list, no machine-readable URL inventory for
crawlers, and no per-page social/structured metadata. For a publishing platform,
discoverability is the highest-leverage growth lever, and it compounds with the
existing feed and syntax highlighting.

CYP-22 scoped this in five independently-mergeable phases: (1) a paginated index,
(2) tags/collections, (3) full-text search, (4) `sitemap.xml`, (5) per-document
SEO metadata. The CEO prioritized **1, 4, 5** as the core SEO win, with 2 and 3
split into a follow-up if scope ran long.

## Decision

Ship phases **1, 4, and 5** now; defer **2 (tags)** and **3 (search)** to a
tracked follow-up because both require a schema migration and broader API
surface than the SEO core.

### Phase 1 — paginated index

- `GET /` is page 1; `GET /page/:n` is page N, **10 documents per page**
  (`PAGE_SIZE` in `src/pages.ts`), newest first, published only.
- Each entry shows title, published date, and a derived plain-text **excerpt**.
- A page number past the end (other than page 1 on an empty site) **404s** so
  crawlers don't chase phantom pages. `parsePageNumber` accepts only the
  canonical spelling of each page (no leading zeros/signs), so each page has one
  URL. Page 1's canonical is always the bare root `/`, never `/page/1`.

### Phase 4 — sitemap.xml

- New framework-free module `src/sitemap.ts`, mirroring `src/feed.ts`: a pure
  `buildSitemap()` plus a `handleSitemapRequest()` that the `node:http` adapter
  dispatches at the fixed top-level path `GET /sitemap.xml`.
- Lists the home page plus every **published** document, each with a `<lastmod>`
  from `updated_at`. Drafts are never listed.

### Phase 5 — per-document SEO metadata

- Every page carries a `<link rel="canonical">`, OpenGraph (`og:*`), and Twitter
  Card (`summary`) tags. Document pages additionally embed a JSON-LD
  `BlogPosting`. Document `<title>` becomes `"<title> — Inkwell"`.
- A feed `<link rel="alternate" type="application/atom+xml">` is advertised on
  every page so the existing Atom feed is auto-discoverable.

### Cross-cutting

- **Excerpts** are derived from `body_markdown` by a lightweight regex strip
  (`deriveExcerpt`), not an AST re-parse — good enough for a meta description and
  index summary, and dependency-free. Used for `<meta name="description">`,
  OpenGraph/Twitter descriptions, and the JSON-LD `description`.
- **Absolute URLs** for canonical/OG/sitemap/feed share one origin resolved by
  `src/site-url.ts` (`normalizeSiteUrl`), read from `INKWELL_SITE_URL`. Falls
  back to `http://localhost` so output is valid and deterministic in dev/tests.
  (`src/feed.ts` predates this helper and keeps its own equivalent; new code
  shares the helper.)

## Safety

A document's `rendered_html` is already sanitized at write time
(see [0002](0002-markdown-rendering.md)) and embedded verbatim. Every other
interpolated value is escaped for its context: HTML/attribute via `escapeHtml`,
XML via `escapeXml`, and the JSON-LD payload via a `<script>`-safe JSON encoder
that escapes `<`, `>`, and `&` to unicode escapes — so a hostile title cannot
close the `<script>` element or inject markup. Discovery surfaces only ever read
**published** documents, preserving the existing draft gating.

## Consequences

- Document slugs `feed.xml` and `sitemap.xml` are unreachable as public pages —
  consistent with the existing `documents`/`health` API reservations.
- Deferred: **tags** (phase 2) and **search** (phase 3). Both need a migration
  (a tags column/table; a `tsvector` index) and API changes. Tracked as a
  follow-up; the sitemap and index are designed to fold in tag URLs/collection
  pages without restructuring (a note marks the extension point in
  `src/sitemap.ts`).
- Postgres `tsvector` for search is the intended approach over a flat `ILIKE` so
  it can be indexed; recorded here so the follow-up starts from the decision
  rather than re-litigating it.

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
- **Queries:** containment via `tag = ANY(tags)` (GIN-accelerated). The distinct
  published-tag set (for the `/tags` index and sitemap) is aggregated in
  application code rather than with `unnest(tags) … GROUP BY`, because the test
  harness `pg-mem` does not implement `unnest`; the published tag set is small so
  this is well within budget.

### Phase 3 — search

- **Intended:** Postgres `tsvector` + GIN, as recorded above.
- **Shipped:** a case-insensitive substring match (`ILIKE` over `title` and
  `body_markdown`, title hits ranked first, LIKE metacharacters escaped). **This
  is a deliberate divergence from the intended `tsvector` approach** because
  `pg-mem` does not implement the `tsvector` type or `to_tsvector` /
  `plainto_tsquery`, and the data-access tests run their migrations against
  `pg-mem`. Adding a `tsvector` column/index to the shared migration would break
  the entire test suite. Using one `ILIKE` code path keeps tests and production
  identical and honours the issue's documented fallback.
- **Migration path:** once tests run against a real Postgres (e.g. a containerized
  instance), add a migration with a generated `tsvector` column + GIN index and
  switch `searchPublishedDocuments` to `@@ plainto_tsquery`, keeping `ILIKE` as
  the `pg-mem` fallback if the dual harness is retained.
- **Surfaces:** `GET /search?q=` serves an HTML results page (search form +
  paginated published matches) or JSON via `?format=json`. Drafts never match.
  Both are dispatched at a fixed top-level path (like `feed.xml`/`sitemap.xml`),
  so a document slug `search` is reserved.

### Discovery surfaces

Tag chips render on index entries and document pages; document JSON-LD gains a
`keywords` field from its tags. `sitemap.xml` lists the `/tags` index plus one
`/tags/:tag` URL per published tag (no `<lastmod>` — a tag page's freshness is
not captured by a single timestamp).

## Alternatives considered

- **A static-asset pipeline / templating engine** — rejected; the hand-built
  template-literal approach matches the existing pages/feed modules and keeps the
  core dependency-free for v0.x.
- **An og:image per document** — deferred; there is no image field on documents
  yet, and a sensible default needs design. Omitting it is valid OpenGraph.
