# ADR 0005: Public web frontend

- **Status:** Accepted
- **Date:** 2026-06-18

## Context

ADR 0004 added the JSON CRUD API. Inkwell's product promise is that authored
Markdown becomes a "clean, fast public web page", so the next layer is a
human-facing frontend: a styled reading page per document and an index of
published documents. We need this without enlarging the dependency surface of a
deliberately small core (no React/SSR framework, no static-asset build step for
v0.1) and without reopening the XSS surface that ADR 0002's sanitizing pipeline
closed.

## Decision

- **Server-rendered HTML, framework-free.** Pages are produced by a pure
  `handlePageRequest(db, req)` handler in `src/pages.ts` that returns a complete
  HTML string, mirroring the framework-free `handleApiRequest`. No client-side
  framework and no bundler; this matches the founding "boring, minimal
  dependencies" precedent (dependency-free `slugify`, hand-rolled routing).
- **Inlined CSS.** The stylesheet is a small `<style>` block inlined into every
  page, so one request renders a styled page and there is no static-asset
  pipeline to operate for v0.1.
- **Routes.** `GET /` renders the index (documents newest first); `GET /:slug`
  renders a document's reading page; unknown slugs and over-deep paths return a
  styled `404`; non-GET methods return `405`.
- **Shared port, reserved API prefixes.** The transport adapter routes any path
  whose first segment is not `documents` or `health` to the frontend, so the API
  and the website are served from the same server. Consequence: a document whose
  slug is exactly `documents` or `health` is unreachable as a public page — those
  words are reserved.
- **Trust boundary.** A document's `rendered_html` is sanitized at write time by
  the ADR 0002 pipeline, so it is embedded verbatim. Every other interpolated
  value (titles, the index link text) is HTML-escaped via `escapeHtml`, so an
  author-controlled title can never inject markup.

## Consequences

- The frontend is integration-tested against the data-access layer on `pg-mem`
  and over a real `node:http` server (asserting `text/html` content type, the
  embedded rendered HTML, title escaping, and that script never re-enters the
  page). `pnpm test` needs no database server.
- Sharing the root path between API and pages keeps the surface tiny but couples
  slug space to the reserved API prefixes; revisit (e.g. mount the API under
  `/api`) before the reserved-word list grows or slug collisions matter.
- Inlined CSS is duplicated on every response. Fine at v0.1 page sizes; an
  external stylesheet + caching layer is a future ADR if payload size matters.
- The handler is transport-agnostic like the API, so a future framework or
  serverless adapter can reuse `handlePageRequest` unchanged.
