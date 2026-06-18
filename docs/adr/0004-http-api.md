# ADR 0004: Document CRUD HTTP API

- **Status:** Accepted
- **Date:** 2026-06-18

## Context

ADR 0003 added durable storage and a typed data-access layer for documents.
Inkwell is "API-first", so the next layer up is a public HTTP API: authors and
tooling create, read, update, and delete documents over JSON. We need input
validation, predictable error responses, and integration coverage against the
real data-access layer — without prematurely committing to a web framework that
would enlarge the dependency surface of a deliberately small core.

## Decision

- **No web framework.** The API is built on Node's built-in `http` module. The
  founding precedent (dependency-free `slugify`, a thin `pg` query layer, no
  ORM) is "boring, minimal dependencies"; an Express/Fastify dependency is not
  justified for five routes. Adding one is a future ADR if/when routing,
  middleware, or content negotiation grows beyond what a hand-written dispatch
  can carry.
- **Framework-free handler core.** Routing, validation, and business logic live
  in `handleApiRequest(db, req)` (`src/api.ts`), which operates on a normalized
  `{ method, segments, body }` request and returns a `{ status, body }`
  response. A thin `node:http` adapter (`src/server.ts`) reads/parses the body,
  normalizes the request, delegates, and serializes the response. This keeps the
  interesting behaviour testable without binding a socket and isolates all
  Node-HTTP coupling to one small file.
- **Dependency-injected database.** `createServer(db)` / `handleApiRequest(db, …)`
  take a `Queryable`, so the same code serves a real `pg` pool in production and
  an in-memory `pg-mem` database in tests.
- **Resource model.** `slug` is the public URL key. Routes:
  `GET/POST /documents`, `GET/PATCH/DELETE /documents/:slug`, plus `GET /health`.
  `PUT` is accepted as an alias for `PATCH`. `rendered_html` is always derived
  server-side from `bodyMarkdown` via the ADR 0002 sanitizing pipeline — clients
  never supply HTML, closing an XSS vector by construction.
- **Validation and error contract.** Missing/blank required fields, malformed
  bodies, and bad slug formats are `400`; an explicit slug must match
  `^[a-z0-9]+(?:-[a-z0-9]+)*$`. A duplicate slug surfaces the data layer's
  `DuplicateSlugError` as `409`; missing documents are `404`; unsupported methods
  on a known path are `405`; unknown paths are `404`; unexpected failures are a
  non-leaky `500`. Every error response shares one shape:
  `{ "error": { "message": string, ... } }`.

## Consequences

- The full request path (routing → validation → rendering → SQL) is
  integration-tested against the data-access layer on `pg-mem`, with an
  additional suite that binds a real `node:http` server and drives it over
  `fetch`. `npm test` needs no database server.
- Hand-rolled routing is fine at this size but does not scale to many resources;
  revisit the no-framework decision before the route count grows substantially.
- The handler is transport-agnostic, so a future framework or serverless adapter
  can reuse `handleApiRequest` unchanged.
- Request bodies are capped at 1 MB in the transport adapter to bound memory;
  oversized bodies return `413`.
