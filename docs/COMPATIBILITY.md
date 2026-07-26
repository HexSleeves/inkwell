# Compatibility Contracts — Inkwell v0.2

This document records which public behaviors are **stability commitments** before
v0.2 and which are **intentionally flexible**. A change to a stable surface
requires a migration note in `CHANGELOG.md` and a semver version bump. A
change to a flexible surface does not.

See also: [API reference](API.md) · [OpenAPI contract](openapi.yaml)

---

## Stable Contracts

These surfaces are covered by the contract-test suite in `tests/` and are
guaranteed not to change without a migration note.

### HTTP Routes

All routes in the "HTTP surface" section of the README are stable:

| Surface | Routes |
|---------|--------|
| REST API | `POST /documents`, `GET /documents`, `GET /documents/:slug`, `PATCH /documents/:slug`, `PUT /documents/:slug`, `DELETE /documents/:slug` |
| Publish | `POST /documents/:slug/publish`, `POST /documents/:slug/unpublish` |
| Graph | `GET /graph`, `GET /documents/:slug/graph` |
| Backlinks | `GET /documents/:slug/backlinks` |
| AI | `GET /ask`, `POST /ask`, `GET /documents/:slug/related` |
| Search | `GET /search` |
| Media | `POST /media`, `GET /media/:id` |
| Admin | `POST /admin/tokens`, `GET /admin/tokens`, `POST /admin/tokens/:prefix/revoke`, `POST /admin/tokens/prune` |
| Preview tokens | `GET /documents/:slug/preview-tokens`, `POST /documents/:slug/preview-tokens`, `DELETE /documents/:slug/preview-tokens/:prefix` |
| Preview read | `GET /documents/:slug/preview` |
| Health | `GET /health`, `GET /healthz`, `GET /readyz` |
| Metrics | `GET /metrics` (only when `INKWELL_METRICS_ENABLED=true`) |
| Public HTML | `GET /`, `GET /page/:page`, `GET /:slug`, `GET /tags`, `GET /tags/:tag`, `GET /tags/:tag/page/:page` |
| Archive | `GET /archive`, `GET /archive/:year/:month`, `GET /archive/:year/:month/page/:page` |
| Feeds | `GET /feed.xml`, `GET /sitemap.xml` |
| Webmention | `POST /webmention` |

A route will not be removed or renamed without a deprecation period and a
CHANGELOG migration note. The method set per route is stable (adding an
unsupported method already returns 405, not 404).

### Document JSON Envelope

`POST /documents`, `GET /documents/:slug`, `PATCH /documents/:slug`, and
`PUT /documents/:slug` return a JSON object with these stable camelCase fields:

| Field | Type | Notes |
|-------|------|-------|
| `id` | UUID string | Immutable, assigned at create |
| `slug` | string | URL-safe; may change via PUT (see slug rename) |
| `title` | string | |
| `bodyMarkdown` | string | Raw Markdown source |
| `renderedHtml` | string | Sanitized HTML; structure is NOT stable (see flexible section) |
| `status` | `"draft"` \| `"published"` | |
| `growth` | `"seedling"` \| `"budding"` \| `"evergreen"` | Default: `"seedling"` |
| `tags` | string[] | Lowercase, trimmed |
| `version` | integer | Monotonic; used with `If-Match` for optimistic concurrency |
| `createdAt` | ISO-8601 UTC datetime string | RFC 3339 format |
| `updatedAt` | ISO-8601 UTC datetime string | RFC 3339 format |

Field names are `camelCase` throughout. A field will not be renamed or removed
without a migration note. New optional fields may be added non-breakingly.

### List Envelope

`GET /documents` returns:
```json
{ "documents": [...], "total": 0, "limit": 20, "offset": 0 }
```
Fields `total`, `limit`, and `offset` are stable integers.

### Status and Enum Values

| Field | Stable values |
|-------|--------------|
| `status` | `"draft"`, `"published"` |
| `growth` | `"seedling"`, `"budding"`, `"evergreen"` |

### Error Envelope

Every 4xx and 5xx response carries a JSON error body with this stable shape:
```json
{
  "error": {
    "message": "human-readable description",
    "requestId": "trace-id-echoed-from-X-Request-Id"
  }
}
```
`error.message` is a non-empty string. `error.requestId` matches the
`X-Request-Id` response header (see below). The key paths `error.message` and
`error.requestId` are stable.

### Authentication

Both credential forms travel in the same header, `X-Api-Key`, and are accepted
interchangeably on every authenticated route:

| Form | Header |
|------|--------|
| Admin / shared key | `X-Api-Key: <key>` |
| Scoped author token | `X-Api-Key: ink_<prefix>_<secret>` |

`Authorization: Bearer` is **not** an authentication channel for API routes; it
carries only the optional `/metrics` scrape token. The header name `X-Api-Key`
and the `ink_<prefix>_<secret>` token format are stable.

A missing or invalid credential on a write returns `401 Unauthorized`. A valid
credential that lacks the required scope returns `403 Forbidden`.

Scope names are stable: `read`, `write`, `publish`, `admin`.

#### Rejected credentials on read routes — the credential channel decides

A credential that is *presented and rejected* (revoked, unknown prefix, wrong
secret, malformed header, expired session cookie) is reported on the channel a
client only ever populates deliberately, and absorbed on the channel a browser
populates automatically. This is the shipped contract, per
[ADR 0015](adr/0015-invalid-credential-on-read-routes.md):

| Case | Through `v0.2.0` | Shipped now |
|---|---|---|
| Rejected `X-Api-Key` on a read route | `200`, published only | **`401 Unauthorized`** |
| Rejected `inkwell_session` cookie on a read route | `200`, published only | `200`, published only (unchanged) |
| Valid credential without `read` scope | `200`, published only | `200`, published only (unchanged) |
| Token lookup fails on a **database error** | `200`, published only | **`503 Service Unavailable`** |
| Any rejected credential on a write/admin route | `401` | `401` (unchanged) |

"Rejected `X-Api-Key`" covers a revoked token, an unknown prefix, a hash
mismatch, a value that is neither the shared key nor token-shaped, and a
duplicated or non-ASCII `X-Api-Key` header.

The split is deliberate: `X-Api-Key` is only ever set on purpose, so rejecting it
silently hides client misconfiguration; the session cookie is sent automatically
by any browser that once logged in, so it must never break public pages.
Anonymous public page loads present neither credential and were unaffected by the
change.

**Migration.** A client that may send a stale `X-Api-Key` to a read route must
treat `401` as "re-authenticate", not "empty result". A client that treats any
non-`2xx` read as a hard failure should distinguish `401` (fix the credential)
from `503` (retry). See the `CHANGELOG.md` entry for the release that shipped it.

### Slug Rename and Redirect

A `PUT /documents/:slug` with a `"slug"` field renames the document. The old
slug is recorded as an alias and returns `301 Moved Permanently` with a
`Location` header pointing to the new slug — on both `GET /documents/:old` and
`GET /:old`. Chains of renames resolve to the current slug in one hop. An alias
to a draft is suppressed for anonymous callers (no existence leak).

### Media

`POST /media` (authenticated) returns:
```json
{ "id": "<uuid>", "url": "/media/<uuid>" }
```
`GET /media/<id>` returns the file with its original `Content-Type` and
`Cache-Control: public, max-age=31536000, immutable`. Both the URL path
format (`/media/<uuid>`) and the `id`/`url` field names are stable.

Accepted MIME types: `image/png`, `image/jpeg`, `image/gif`, `image/webp`.
SVG is not accepted (script-injection risk).

### Rate Limiting

Write requests over the configured limit return `429 Too Many Requests` with a
`Retry-After` header (value in seconds). The `429` status and the header name
are stable. The default limit is 60 requests/minute per credential.

### Request Correlation

Every response carries `X-Request-Id`. A well-formed inbound `X-Request-Id` is
echoed unchanged; a missing or malformed one is replaced with a fresh UUID v4.
The header name and the presence of `requestId` in the error envelope are stable.

### Health Endpoints

`GET /health` returns `200 OK` with body `{"status":"ok","db":"up"}` when the
database is reachable, or `503 Service Unavailable` with
`{"status":"error","db":"down"}` otherwise. Both field names are stable.

`GET /readyz` is identical to `GET /health`. `GET /healthz` is liveness only: it
consults no dependency and returns `200 OK` with `{"status":"ok"}` (no `db`
field). The split is additive — `/health` is unchanged. See
[ADR 0012](adr/0012-observability.md).

### Wikilink HTML Output

Resolved `[[note]]` wikilinks render to `<a href="/slug">display text</a>`.
Unresolved links carry a `class="stub"` attribute. Wikilinks inside inline
code spans and fenced code blocks are never rewritten. These output shapes
are stable for tooling that post-processes rendered HTML.

### Public HTML CSS Classes

The following CSS class names on the public site are stable and may be used by
themes or browser extensions:

| Class | Element |
|-------|---------|
| `site-body` | `<body>` |
| `site-shell` | Outer container |
| `site-header` | Header landmark |
| `site-main` | Main landmark |
| `site-footer` | Footer landmark |
| `site-brand` | Home link |
| `site-nav` | Navigation link |
| `tags` | `<ul>` of tag chips |
| `excerpt` | `<p>` excerpt on listing pages |
| `pager` | `<nav>` pagination control |

Inner HTML structure, class values on non-listed elements, and all CSS
property values are **not** stable.

### Atom Feed

`GET /feed.xml` returns a valid Atom 1.0 feed (`Content-Type:
application/atom+xml`). The presence of `<feed>`, `<entry>`, `<title>`,
`<link>`, and `<updated>` elements is stable. Exact attribute values and
namespace prefixes may change.

### MCP Server

The `inkwell mcp` server exposes five stable tools:

| Tool | Description |
|------|-------------|
| `search_notes` | Full-text + semantic search |
| `read_note` | Fetch a note by slug |
| `list_notes` | Paginated note listing |
| `create_note` | Create a new draft |
| `update_note` | Update with `expected_version` guard |

Tool names, required parameters, and return shapes are stable. The MCP server
authenticates with a scoped token set in `INKWELL_API_KEY`.

---

## Intentionally Flexible

Changes to these surfaces **do not** require migration notes:

- **`renderedHtml` internals** — the Markdown-to-HTML pipeline (Comrak options,
  backlink panel markup, embed transclusion HTML, syntax highlighting class
  names) may change between patch versions.
- **CSS property values** — colors, spacing, typography, and the Nunito font may
  change.
- **AI synthesis** — the LLM model, prompt, chunk size, and answer quality are
  deployment-tunable and not part of the API contract.
- **Seed note content** — `inkwell seed` sample vault text, slugs, and structure
  may change.
- **Atom feed attribute values** — internal ordering and formatting within valid
  Atom 1.0 are not stable.
- **Sitemap sub-paths** — `/sitemaps/documents/:page` and `/sitemaps/tags/:page`
  are implementation details of the sitemap index; only `/sitemap.xml` is a
  stable entry point.
- **Chunk/embedding internals** — `note_chunks` schema, provenance fields, and
  vector dimensions are subject to change with model upgrades.
- **`write_audit` table** — internal audit trail; not part of the public API.

---

## What Triggers a Migration Note

A CHANGELOG migration note and a semver version bump are required when:

- Any stable route is removed, renamed, or has its method set changed.
- Any stable JSON field is renamed, removed, or its type changes.
- A stable enum value (`status`, `growth`, scope name) is renamed or removed.
- The error envelope shape (`error.message`, `error.requestId`) changes.
- The authentication header names or token format changes.
- A stable CSS class is renamed or removed.
- A stable MCP tool name or required parameter changes.

Adding new optional JSON fields, new routes, or new enum values is
non-breaking and does not require a migration note.
