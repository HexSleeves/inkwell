# Inkwell HTTP API Reference

Inkwell is an API-first Markdown publishing platform. This document is the
authoritative reference for its HTTP surface. Client authors do not need to
read the source code or tests to integrate.

---

## Table of contents

- [Base URL](#base-url)
- [Authentication](#authentication)
- [Request correlation](#request-correlation)
- [Rate limiting](#rate-limiting)
- [Error envelope](#error-envelope)
- [Status codes](#status-codes)
- [Document object](#document-object)
- [Endpoints](#endpoints)
  - [Health](#health)
  - [Documents — CRUD](#documents-crud)
  - [Documents — state transitions](#documents-state-transitions)
  - [Documents — preview tokens](#documents-preview-tokens)
  - [Documents — linked surfaces](#documents-linked-surfaces)
  - [Garden graph](#garden-graph)
  - [AI — question answering](#ai-question-answering)
  - [AI — related notes](#ai-related-notes)
  - [Search](#search)
  - [Media](#media)
  - [Admin — token management](#admin-token-management)
  - [Feed and sitemaps](#feed-and-sitemaps)
- [Public HTML routes](#public-html-routes)
  - [Archive navigation](#archive-navigation)

---

## Base URL

All paths are relative to the server root, e.g. `https://blog.example.com`.

Configure the base URL with the `INKWELL_API_URL` environment variable when
using the `inkwell author` CLI.

---

## Authentication

All write endpoints and some read endpoints require authentication. Send the
credential in the `X-Api-Key` request header:

```
X-Api-Key: <credential>
```

Three credential families are accepted:

| Family | Format | Principal |
|--------|--------|-----------|
| Shared admin key | Any string set as `INKWELL_API_KEY` | Bootstrap admin (all scopes) |
| Scoped token | `ink_<prefix>_<secret>` | Named author with declared scopes |
| Browser session cookie | `inkwell_session=<value>` (only when `INKWELL_BROWSER_LOGIN=true`) | Author, capped at read/write/publish |

If `X-Api-Key` is present, the cookie path is never consulted. A missing or
invalid credential on a write endpoint returns `401`.

### Scopes

| Scope | Grants |
|-------|--------|
| `read` | Read the caller's own draft documents |
| `write` | Create, update, and delete documents; upload media |
| `publish` | Publish and unpublish documents |
| `admin` | All of the above, plus token management |

The shared `INKWELL_API_KEY` carries all scopes implicitly. Scoped tokens
carry exactly the scopes declared at mint time.

### Visibility

Read endpoints apply an owner-aware visibility filter based on credentials:

| Credentials | Visible documents |
|------------|-------------------|
| None / no `read` scope | Published only |
| Scoped token with `read` | Own drafts + all published |
| Admin / shared key | All documents |

---

## Request correlation

Every response carries an `X-Request-Id` header. The value is a UUID v4
minted per request (or forwarded from a well-formed inbound `X-Request-Id`).

Every error response also includes the same value in the JSON body as
`error.requestId`. When reporting a problem, include this ID so it can be
traced to server logs.

```
X-Request-Id: a3b5c7d9-1234-4abc-8def-0123456789ab
```

---

## Rate limiting

Mutations and the `/ask` endpoint are rate-limited by a GCRA (Generic Cell
Rate Algorithm) limiter. The limit is configured by `INKWELL_WRITE_RATE_LIMIT`
(default 60 requests per minute).

**Throttled methods:** `POST`, `PUT`, `PATCH`, `DELETE`, and `GET|POST /ask`.
Safe reads (`GET`, `HEAD`) and the public HTML site are never throttled.

**Keying:** by validated principal (the token is verified first, then keys the
bucket). Anonymous callers are keyed by client IP. When
`INKWELL_TRUST_FORWARDED_HEADERS=true`, `X-Forwarded-For` / `X-Real-IP` are
used for IP keying (leave off unless behind a trusted proxy).

Over-limit responses:

```
HTTP/1.1 429 Too Many Requests
Retry-After: 4
X-Request-Id: ...
Content-Type: application/json

{
  "error": {
    "message": "Rate limit exceeded. Slow down and retry later.",
    "requestId": "a3b5c7d9-..."
  }
}
```

Back off for at least `Retry-After` seconds before retrying.

---

## Error envelope

All error responses use this JSON shape:

```json
{
  "error": {
    "message": "Human-readable description.",
    "requestId": "a3b5c7d9-1234-4abc-8def-0123456789ab",
    "slug": "conflicting-slug",
    "allow": "GET, POST"
  }
}
```

| Field | Always present | Notes |
|-------|---------------|-------|
| `error.message` | Yes | Human-readable error text |
| `error.requestId` | Yes (inside a request) | Correlation ID for log tracing |
| `error.slug` | On `409 Conflict` for duplicate slug | The conflicting slug |
| `error.allow` | On `405 Method Not Allowed` | Comma-separated allowed methods |

---

## Status codes

| Code | Meaning |
|------|---------|
| `200 OK` | Success |
| `201 Created` | Resource created |
| `204 No Content` | Success, no body (DELETE) |
| `301 Moved Permanently` | Slug was renamed; `Location` header gives new path |
| `400 Bad Request` | Invalid request — malformed JSON, missing field, value out of range |
| `401 Unauthorized` | Missing or invalid `X-Api-Key` |
| `403 Forbidden` | Valid credential, but missing required scope |
| `404 Not Found` | Resource does not exist (or caller cannot see it) |
| `405 Method Not Allowed` | HTTP method not accepted; `error.allow` lists valid methods |
| `409 Conflict` | Duplicate slug, or `If-Match` version mismatch |
| `413 Payload Too Large` | Request body exceeds the size limit |
| `429 Too Many Requests` | Rate limit exceeded; `Retry-After` gives seconds to wait |
| `500 Internal Server Error` | Server-side failure |
| `503 Service Unavailable` | Health check: database unreachable |

---

## Document object

```json
{
  "id": "550e8400-e29b-41d4-a716-446655440000",
  "slug": "hello-world",
  "title": "Hello World",
  "bodyMarkdown": "# Hello World\n\nBody text here.",
  "renderedHtml": "<h1>Hello World</h1>\n<p>Body text here.</p>",
  "status": "published",
  "growth": "seedling",
  "tags": ["rust", "notes"],
  "version": 3,
  "createdAt": "2026-01-15T10:30:00Z",
  "updatedAt": "2026-06-01T08:00:00Z"
}
```

| Field | Type | Notes |
|-------|------|-------|
| `id` | UUID string | Stable identifier |
| `slug` | string | URL-safe identifier; mutable (see rename) |
| `title` | string | Max 500 characters |
| `bodyMarkdown` | string | Raw Markdown source; max 262,144 characters |
| `renderedHtml` | string | Server-rendered HTML with wikilink expansion |
| `status` | `"draft"` \| `"published"` | Publication state |
| `growth` | `"seedling"` \| `"budding"` \| `"evergreen"` | Digital-garden maturity |
| `tags` | string[] | Normalized lowercase tags; max 20, each max 50 chars |
| `version` | integer | Monotonic revision counter; use as `If-Match` value |
| `createdAt` | ISO 8601 UTC | Creation timestamp |
| `updatedAt` | ISO 8601 UTC | Last-modified timestamp |

---

## Endpoints

### Health

#### `GET /health`

Returns the service status and database connectivity. No authentication required.

**Response `200 OK`:**
```json
{ "status": "ok", "db": "up" }
```

**Response `503 Service Unavailable`:**
```json
{ "status": "error", "db": "down" }
```

---

### Documents — CRUD

#### `GET /documents`

List documents. Anonymous callers see only published documents. Authenticated
callers with the `read` scope also see their own drafts.

**Query parameters:**

| Parameter | Default | Notes |
|-----------|---------|-------|
| `limit` | `20` | Max `100` |
| `offset` | `0` | Zero-based |
| `status` | (all visible) | `"draft"`, `"published"`, or `"all"` |

**Response `200 OK`:**

```json
{
  "documents": [ /* Document objects */ ],
  "total": 42,
  "limit": 20,
  "offset": 0
}
```

The `status` filter ANDs with visibility: an anonymous caller requesting
`?status=draft` always gets an empty list.

---

#### `POST /documents`

Create a document. Requires the `write` scope. The new document is created in
`draft` status. The `owner_id` is set from the authenticated principal.

**Request body** (`Content-Type: application/json`, max ~1 MB):

```json
{
  "title": "Hello World",
  "bodyMarkdown": "# Hello World\n\nBody text here.",
  "slug": "hello-world",
  "tags": ["rust", "notes"],
  "growth": "seedling"
}
```

| Field | Required | Notes |
|-------|----------|-------|
| `title` | Yes | Non-empty; max 500 characters |
| `bodyMarkdown` | Yes | Non-empty; max 262,144 characters |
| `slug` | No | Auto-derived from title if omitted; lowercase alphanumerics and hyphens; max 200 chars |
| `tags` | No | Array of strings; max 20 tags, each max 50 chars |
| `growth` | No | `"seedling"` (default), `"budding"`, or `"evergreen"` |

**Response `201 Created`:** a [Document object](#document-object).

**Errors:**

| Status | Cause |
|--------|-------|
| `400` | Missing `title` or `bodyMarkdown`; invalid `slug` format; unknown `growth` value; body > 1 MB |
| `401` | Missing or invalid `X-Api-Key` |
| `403` | Token lacks `write` scope |
| `409` | A document with this slug already exists (`error.slug` identifies it) |

---

#### `GET /documents/{slug}`

Fetch a single document. Visibility applies (drafts visible only to owner or
admin). If the slug was renamed, responds with `301 Moved Permanently` and a
`Location: /documents/{new-slug}` header (provided the target is visible to
the caller).

The response includes an `ETag` header whose value is the document's version,
for use as `If-Match` on a conditional update.

```
ETag: "3"
```

**Response `200 OK`:** a [Document object](#document-object).

**Errors:**

| Status | Cause |
|--------|-------|
| `404` | Slug does not exist or is a draft the caller cannot see |

---

#### `PATCH /documents/{slug}` / `PUT /documents/{slug}`

Update a document. Both `PATCH` and `PUT` accept the same partial-update body
(only fields present in the request body are changed). Requires the `write`
scope. Non-owners receive `404` (ownership enforced atomically in the DB write;
no TOCTOU window).

**Request body** (at least one field required):

```json
{
  "title": "Updated Title",
  "bodyMarkdown": "Updated body.",
  "tags": ["rust"],
  "growth": "budding",
  "slug": "new-slug"
}
```

| Field | Notes |
|-------|-------|
| `title` | Max 500 characters |
| `bodyMarkdown` | Max 262,144 characters |
| `tags` | Replaces the full tag list |
| `growth` | `"seedling"`, `"budding"`, or `"evergreen"` |
| `slug` | Rename the document; old slug auto-registers as a `301` alias |

**Optimistic concurrency (`If-Match`):**

Send the document's `version` as `If-Match` to guard against stale writes:

```
If-Match: "3"
```

If the server's current version differs, `409 Conflict` is returned with the
current version in the error message. Without `If-Match`, the update proceeds
unconditionally.

**Response `200 OK`:** the updated [Document object](#document-object).

**Errors:**

| Status | Cause |
|--------|-------|
| `400` | Empty body; no recognized fields; invalid field value; body > 1 MB |
| `401` | Missing or invalid `X-Api-Key` |
| `403` | Token lacks `write` scope |
| `404` | Slug does not exist or is not owned by the caller |
| `409` | `If-Match` version mismatch (includes current version in message); or duplicate slug on rename |

---

#### `DELETE /documents/{slug}`

Delete a document permanently. Requires the `write` scope. Non-owners receive
`404`. Backlinks from other documents are automatically downgraded to stubs.

**Response `204 No Content`:** no body.

**Errors:**

| Status | Cause |
|--------|-------|
| `401` | Missing or invalid `X-Api-Key` |
| `403` | Token lacks `write` scope |
| `404` | Slug does not exist or is not owned by the caller |

---

### Documents — state transitions

#### `POST /documents/{slug}/publish`

Publish a draft document, making it publicly visible. Requires the `publish`
scope. Non-owners receive `404`.

**Request body:** empty (no body required).

**Response `200 OK`:** the updated [Document object](#document-object) with
`"status": "published"`.

**Errors:**

| Status | Cause |
|--------|-------|
| `401` | Missing or invalid `X-Api-Key` |
| `403` | Token lacks `publish` scope |
| `404` | Slug does not exist or is not owned by the caller |

---

#### `POST /documents/{slug}/unpublish`

Return a published document to draft status. Requires the `publish` scope.
Non-owners receive `404`. Backlinks pointing at the note are downgraded to
stubs.

**Request body:** empty.

**Response `200 OK`:** the updated [Document object](#document-object) with
`"status": "draft"`.

**Errors:** same as `/publish`.

---

### Documents — preview tokens

Preview tokens allow sharing a rendered draft with anyone who has the token URL,
without exposing the full API. Any failure (expired, revoked, unknown) returns
`401` — the draft's existence is never leaked.

Token format: `pvw_<prefix>_<secret>`. Migration 0022 (`preview_tokens` table).

---

#### `POST /documents/{slug}/preview-tokens`

Mint a new preview token tied to this document. Requires the `admin` scope or
ownership with the `write` scope.

**Request body:** empty (`{}`) or omitted. Optional fields:

```json
{ "expiresAt": "2026-12-31T23:59:59Z" }
```

| Field | Required | Notes |
|-------|----------|-------|
| `expiresAt` | No | ISO 8601 UTC expiry. Omit for a non-expiring token. |

**Response `201 Created`:**

```json
{
  "token": "pvw_abc123_secretvalue",
  "prefix": "abc123",
  "slug": "my-draft",
  "expiresAt": null,
  "createdAt": "2026-06-27T10:00:00Z"
}
```

Store the `token` value — the secret is shown exactly once. Use the `prefix` to
list or revoke.

**Errors:**

| Status | Cause |
|--------|-------|
| `401` | Missing or invalid `X-Api-Key` |
| `403` | Token lacks required scope |
| `404` | Document not found or not visible to caller |

---

#### `GET /documents/{slug}/preview-tokens`

List all non-revoked preview tokens for this document. Secrets are never returned.

**Response `200 OK`:**

```json
{
  "tokens": [
    {
      "prefix": "abc123",
      "slug": "my-draft",
      "expiresAt": null,
      "createdAt": "2026-06-27T10:00:00Z",
      "revokedAt": null
    }
  ]
}
```

**Errors:** same as mint (401/403/404).

---

#### `DELETE /documents/{slug}/preview-tokens/{prefix}`

Revoke a preview token by its prefix. Revoked tokens return `401` immediately on
any preview attempt.

**Response `204 No Content`:** no body.

**Errors:**

| Status | Cause |
|--------|-------|
| `401` | Missing or invalid `X-Api-Key` |
| `403` | Token lacks required scope |
| `404` | No token with this prefix for this document |

---

#### `GET /documents/{slug}/preview?token=<pvw_...>`

Render a draft document for any bearer of a valid, non-expired, non-revoked
preview token. **No `X-Api-Key` header required.** Returns an HTML page
identical to the published document page.

Any failure (token invalid, expired, revoked, document not found) returns `401
Unauthorized` — the draft's existence is not revealed to anonymous callers.

**Response `200 OK`:** HTML page (same design as published document pages).

**Errors:**

| Status | Cause |
|--------|-------|
| `401` | Token missing, invalid, expired, or revoked; document not found |

---

### Documents — linked surfaces

#### `GET /documents/{slug}/backlinks`

Return the "linked from" set for a document: internal backlinks (other notes
with a `[[wikilink]]` to this one) and verified inbound Webmentions. Visibility
applies — a public caller never sees a draft source or a mention targeting a
draft.

**Response `200 OK`:**

```json
{
  "backlinks": [
    {
      "slug": "other-note",
      "title": "Other Note",
      "snippet": "…context around the link…"
    }
  ],
  "mentions": [
    {
      "sourceUrl": "https://external-site.example/post"
    }
  ]
}
```

**Errors:**

| Status | Cause |
|--------|-------|
| `404` | Target document not found or not visible to caller |

---

#### `GET /documents/{slug}/graph`

Return the one-hop neighborhood link graph around a note: nodes (slug + title)
and directed edges. Visibility-filtered; a public caller never sees a draft
neighbor. Returns `404` when the target is not visible.

**Response `200 OK`:**

```json
{
  "nodes": [
    { "slug": "hello-world", "title": "Hello World" },
    { "slug": "rust-notes", "title": "Rust Notes" }
  ],
  "edges": [
    { "sourceSlug": "hello-world", "targetSlug": "rust-notes" }
  ]
}
```

---

### Garden graph

#### `GET /graph`

Return the whole-garden link graph: all nodes and directed edges within the
bounded cap. Visibility-filtered.

**Response `200 OK`:** same shape as `/documents/{slug}/graph`.

---

### AI — question answering

#### `GET /ask?q=<query>`
#### `POST /ask`

Retrieve semantically relevant content chunks (via pgvector) and synthesize a
grounded answer using the configured LLM. Falls back to full-text search when
no vector index exists. Visibility-filtered — a public caller's answer is
never grounded in a draft.

Throttled by the write rate limiter (counts toward the per-principal bucket).

**GET:** pass the question as `?q=<encoded query>`.

**POST request body:**

```json
{ "q": "What is a digital garden?" }
```

The `q` parameter may also be supplied as a query string on POST; the body
field takes precedence.

**Constraints:**
- Max query length: 1,000 characters
- An empty `q` returns `400`

**Response `200 OK`:**

```json
{
  "query": "What is a digital garden?",
  "answer": "A digital garden is an evolving collection of interconnected notes…",
  "citations": [
    { "slug": "digital-garden", "title": "Digital Garden" },
    { "slug": "evergreen-notes", "title": "Evergreen Notes" }
  ]
}
```

When `ANTHROPIC_API_KEY` is not configured, `answer` is
`"AI features are not configured on this site."` and `citations` is empty.
The status is still `200`.

**Errors:**

| Status | Cause |
|--------|-------|
| `400` | Empty `q`; `q` > 1,000 characters; POST body is not valid JSON |
| `405` | Method other than GET or POST |
| `429` | Rate limit exceeded |

---

### AI — related notes

#### `GET /documents/{slug}/related`

Return the top 5 notes most semantically similar to this one (by embedding
cosine distance). Visibility-filtered; a public caller never sees a draft
neighbor. Returns an empty list when no embedding index exists for the note.

**Response `200 OK`:**

```json
{
  "slug": "hello-world",
  "related": [
    { "slug": "rust-notes", "title": "Rust Notes", "distance": 0.12 },
    { "slug": "async-intro", "title": "Async Introduction", "distance": 0.24 }
  ]
}
```

| Field | Notes |
|-------|-------|
| `slug` | The origin note's slug |
| `related[].slug` | Related note's slug |
| `related[].title` | Related note's title |
| `related[].distance` | Cosine distance (0 = identical, lower = more similar) |

**Errors:**

| Status | Cause |
|--------|-------|
| `404` | Origin slug not found or not visible to caller |
| `405` | Method other than GET |

---

### Search

#### `GET /search?q=<query>&format=json`

Full-text search across documents. Without `?format=json` the endpoint returns
an HTML page (the public site's search UI). Pass `?format=json` to receive a
JSON payload instead.

The JSON path is owner-aware (authenticated owners find their own drafts).
The HTML path is always public-only so shared caches never serve private results.

**Query parameters:**

| Parameter | Notes |
|-----------|-------|
| `q` | Search query; empty returns zero results |
| `format` | `"json"` for JSON; omit for HTML |
| `page` | Page number (1-based, default `1`) |

Page size is fixed at the server default (currently 20 results per page).

**Response `200 OK` (`?format=json`):**

```json
{
  "query": "digital garden",
  "page": 1,
  "pageSize": 20,
  "total": 5,
  "results": [
    {
      "slug": "digital-garden",
      "title": "Digital Garden",
      "excerpt": "A digital garden is…",
      "tags": ["writing", "notes"],
      "createdAt": "2026-01-10T12:00:00Z",
      "updatedAt": "2026-06-01T08:00:00Z"
    }
  ]
}
```

---

### Media

#### `POST /media`

Upload a binary image. Requires the `write` scope. The file is stored
server-side and served at a stable `/media/{id}` URL.

**Request headers:**
- `Content-Type`: one of `image/png`, `image/jpeg`, `image/gif`, `image/webp`
  (SVG excluded for security; parameters like `; charset=…` are stripped)

**Request body:** raw image bytes (max 5 MiB).

**Response `201 Created`:**

```json
{
  "id": "550e8400-e29b-41d4-a716-446655440000",
  "url": "/media/550e8400-e29b-41d4-a716-446655440000"
}
```

**Errors:**

| Status | Cause |
|--------|-------|
| `400` | Unsupported `Content-Type` |
| `401` | Missing or invalid `X-Api-Key` |
| `403` | Token lacks `write` scope |
| `413` | Body exceeds 5 MiB |
| `429` | Rate limit exceeded |

---

#### `GET /media/{id}`

Serve a stored media file. Public (no auth). Also responds to `HEAD`.

**Response `200 OK`:**
- `Content-Type`: original MIME type from upload
- `Cache-Control: public, max-age=31536000, immutable`
- Body: raw image bytes

**Errors:**

| Status | Cause |
|--------|-------|
| `404` | No media with this ID |

---

### Admin — token management

All `/admin/*` endpoints require the `admin` scope (shared `INKWELL_API_KEY`
or a token minted with `"scopes": ["admin"]`).

---

#### `GET /admin/tokens`

List token metadata. The secret is never recoverable after creation.

**Query parameters:**

| Parameter | Default | Notes |
|-----------|---------|-------|
| `all` | `false` | Set `true` to include revoked tokens |

**Response `200 OK`:**

```json
{
  "tokens": [
    {
      "prefix": "abc123",
      "authorName": "Alice",
      "scopes": ["write", "publish"],
      "createdAt": "2026-01-15T10:30:00Z",
      "lastUsedAt": "2026-06-25T14:00:00Z",
      "revokedAt": null
    }
  ]
}
```

| Field | Notes |
|-------|-------|
| `prefix` | Public prefix; pass to revoke |
| `authorName` | Author this token belongs to |
| `scopes` | Declared scopes |
| `createdAt` | Mint timestamp |
| `lastUsedAt` | Last successful authentication, or `null` |
| `revokedAt` | Revocation timestamp, or `null` if still live |

---

#### `POST /admin/tokens`

Mint a new scoped token for an author. If no author with the given `name`
exists, one is created. **The full token secret is returned exactly once**; it
cannot be recovered later.

**Request body:**

```json
{
  "name": "Alice",
  "scopes": ["write", "publish"]
}
```

| Field | Required | Notes |
|-------|----------|-------|
| `name` | Yes | Author name; non-empty; max 200 characters; creates the author on first use |
| `scopes` | Yes | Non-empty array; values: `"read"`, `"write"`, `"publish"`, `"admin"` |

**Response `201 Created`:**

```json
{
  "token": "ink_abc123_secretvalue",
  "prefix": "abc123",
  "author": "Alice",
  "scopes": ["write", "publish"]
}
```

Store the `token` value securely — it is shown once. Use the `prefix` to
identify the token in future list/revoke operations.

**Errors:**

| Status | Cause |
|--------|-------|
| `400` | Missing `name`; empty `scopes`; unknown scope string |
| `401` | Missing or invalid `X-Api-Key` |
| `403` | Token lacks `admin` scope |

---

#### `POST /admin/tokens/{prefix}/revoke`

Revoke a token immediately by its prefix. Revoked tokens never authenticate.

**Request body:** empty.

**Response `200 OK`:**

```json
{ "prefix": "abc123", "revoked": true }
```

**Errors:**

| Status | Cause |
|--------|-------|
| `401` | Missing or invalid `X-Api-Key` |
| `403` | Token lacks `admin` scope |
| `404` | No live token with this prefix |

---

#### `POST /admin/tokens/prune`

Hard-delete all revoked tokens from the database. Live tokens are untouched.

**Request body:** empty.

**Response `200 OK`:**

```json
{ "pruned": 3 }
```

---

### Feed and sitemaps

These endpoints are public (no auth) and return XML.

| Route | Description |
|-------|-------------|
| `GET /feed.xml` | Atom/RSS feed of published documents |
| `GET /sitemap.xml` | Root sitemap index |
| `GET /sitemap-static.xml` | Sitemap for static pages |
| `GET /sitemaps/documents/{page}` | Paginated document sitemap |
| `GET /sitemaps/tags/{page}` | Paginated tag sitemap |

---

## Public HTML routes

These routes serve the public HTML site (not JSON). They are not part of the
programmatic API but are included here for completeness.

| Route | Description |
|-------|-------------|
| `GET /` | Garden index (published notes) |
| `GET /page/{page}` | Paginated index |
| `GET /{slug}` | Rendered document page |
| `GET /tags` | Tag index |
| `GET /tags/{tag}` | Notes tagged with `{tag}` |
| `GET /tags/{tag}/page/{page}` | Paginated tag page |
| `GET /search` | Full-text search HTML page |
| `GET /archive` | Archive index — year/month buckets |
| `GET /archive/{year}/{month}` | Monthly archive page |
| `GET /archive/{year}/{month}/page/{page}` | Paginated monthly archive |

### Archive navigation

#### `GET /archive`

Lists all year/month buckets that contain at least one published document. Returns
an HTML page with one entry per non-empty month, ordered newest first. Included
in the sitemap.

**Response `200 OK`:** HTML page.

---

#### `GET /archive/{year}/{month}`

Shows a paginated list of published documents for a given month (e.g.
`/archive/2026/01`). Page size matches the site default.

**Response `200 OK`:** HTML page. Returns `404` if no documents exist for
that month.

---

#### `GET /archive/{year}/{month}/page/{page}`

Additional pages of the archive month listing. `page` is 1-based.

**Response `200 OK`:** HTML page. Returns `404` if the page number is out of
range.

---

All archive routes carry canonical metadata and `Cache-Control` headers.
Document pages carry a `<nav class="doc-nav">` prev/next bar linking to the
immediately older and newer published documents (omitted gracefully if the
query fails).

---

Optional (when `INKWELL_BROWSER_LOGIN=true`):

| Route | Description |
|-------|-------------|
| `POST /auth/login` | Browser session login |
| `POST /auth/logout` | Browser session logout |

When `INKWELL_BROWSER_LOGIN` is off (the default), `/auth/*` routes return
`404` — the routes do not exist and no auth surface is exposed.
