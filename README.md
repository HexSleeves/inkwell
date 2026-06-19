# Inkwell

> An open, API-first Markdown publishing platform.

Inkwell lets authors write in **Markdown**, persists documents through a clean
HTTP **API**, and renders them to fast, accessible public web pages. It is
**self-hostable** and built around a small, well-tested core rather than a
sprawling feature set.

This repository is the founding scaffold for Inkwell **v0.1**. The product
surface (document model, Postgres persistence, Markdown → HTML rendering, and
the public web frontend) is being built on top of this toolchain.

## Why Inkwell

- **Markdown-native.** Authors write plain Markdown; no proprietary editor lock-in.
- **API-first.** Every capability is reachable over a documented HTTP API, so
  Inkwell can back a CLI, a CMS UI, or someone else's tooling.
- **Open & self-hostable.** MIT-licensed, runs on your own infrastructure.
- **Small, trustworthy core.** Typed schemas, migrations, and tests over breadth.

## Tech stack

| Concern     | Choice                                           |
| ----------- | ------------------------------------------------ |
| Language    | TypeScript (ESM, `NodeNext`)                     |
| Runtime     | Node.js ≥ 20                                     |
| Persistence | PostgreSQL via [`pg`](https://node-postgres.com) |
| Test runner | [Vitest](https://vitest.dev)                     |
| Lint        | ESLint 9 (flat config) + typescript-eslint       |
| Format      | Prettier                                         |
| CI          | GitHub Actions                                   |

See [`docs/adr/0001-toolchain.md`](docs/adr/0001-toolchain.md) for the rationale
behind these choices.

## Getting started

Requires Node.js ≥ 20 and npm.

```bash
# Install dependencies
npm install

# Run the test suite once
npm test

# Type-check, lint, and check formatting
npm run typecheck
npm run lint
npm run format:check

# Build the library to ./dist
npm run build
```

## Environment variables

Inkwell is configured entirely through the environment. Only `DATABASE_URL` is
required; the rest have sensible defaults.

| Variable       | Required | Default   | Used by                | Description                                                                                               |
| -------------- | -------- | --------- | ---------------------- | --------------------------------------------------------------------------------------------------------- |
| `DATABASE_URL` | yes      | —         | server, `db:*` scripts | Postgres connection string, e.g. `postgres://user:pass@host:5432/inkwell`. Startup fails loudly if unset. |
| `PORT`         | no       | `3000`    | server                 | TCP port the HTTP server listens on.                                                                      |
| `HOST`         | no       | `0.0.0.0` | server                 | Address to bind. Use `127.0.0.1` to restrict to localhost.                                                |
| `INKWELL_API_KEY` | no    | —         | server                 | Shared secret required on mutating requests via the `X-API-Key` header (see below). Unset locks down all writes. |

### Write authentication

Mutating API requests — `POST`, `PATCH`/`PUT`, and `DELETE` on `/documents` —
require the shared secret `INKWELL_API_KEY` to be presented in an `X-API-Key`
header; missing or wrong keys get `401`. Reads (`GET /documents`,
`GET /documents/:slug`) and the public HTML frontend stay open. If
`INKWELL_API_KEY` is unset or empty the server fails closed: no key can match,
so every write is rejected. Set it before allowing writes:

```bash
export INKWELL_API_KEY=$(openssl rand -hex 32)
curl -X POST http://localhost:3000/documents \
  -H "content-type: application/json" \
  -H "x-api-key: $INKWELL_API_KEY" \
  -d '{"title":"Hello","bodyMarkdown":"# Hi"}'
```

## Run Inkwell

Follow these steps from a fresh clone to publish your first Markdown page. They
assume a reachable PostgreSQL instance.

```bash
# 1. Install dependencies and compile to ./dist
npm install
npm run build

# 2. Point Inkwell at your database
export DATABASE_URL=postgres://user:pass@localhost:5432/inkwell

# 3. Create the schema
npm run db:migrate

# 4. Start the server (defaults to http://0.0.0.0:3000)
npm start
```

You should see `Inkwell listening on http://0.0.0.0:3000`. Leave it running and,
in a second terminal, **publish a document** by POSTing Markdown to the API:

```bash
curl -sS -X POST http://localhost:3000/documents \
  -H 'content-type: application/json' \
  -d '{"title":"Hello World","bodyMarkdown":"# Hello World\n\nMy first **Inkwell** page."}'
```

The response echoes the stored document, including its derived `slug`
(`hello-world`) and the sanitized `renderedHtml`. Your page is now live:

- Open <http://localhost:3000/hello-world> to read the published page.
- Open <http://localhost:3000/> to see it listed on the index.

To update it, `PATCH` the same slug; to remove it, `DELETE` it:

```bash
curl -sS -X PATCH http://localhost:3000/documents/hello-world \
  -H 'content-type: application/json' \
  -d '{"bodyMarkdown":"# Hello World\n\nNow with an edit."}'

curl -sS -X DELETE http://localhost:3000/documents/hello-world -o /dev/null -w '%{http_code}\n'
```

See [API](#api) below for the full endpoint reference.

## npm scripts

| Script                  | What it does                                     |
| ----------------------- | ------------------------------------------------ |
| `npm start`             | Run the compiled server (`dist/main.js`)         |
| `npm run build`         | Compile `src/` to `dist/` with type declarations |
| `npm run typecheck`     | Type-check without emitting                      |
| `npm run lint`          | Lint with ESLint                                 |
| `npm run lint:fix`      | Lint and auto-fix                                |
| `npm run format`        | Format the repo with Prettier                    |
| `npm run format:check`  | Verify formatting (used in CI)                   |
| `npm test`              | Run the test suite once                          |
| `npm run test:watch`    | Run tests in watch mode                          |
| `npm run test:coverage` | Run tests with V8 coverage                       |
| `npm run db:migrate`    | Apply pending migrations (needs `DATABASE_URL`)  |
| `npm run db:rollback`   | Roll back the last migration (`[n]` for more)    |
| `npm run db:status`     | List applied migration ids                       |
| `npm run ci`            | Lint + format check + typecheck + test + build   |

The `db:*` scripts run against compiled output, so `npm run build` first.

## Project layout

```
.
├── src/                 # TypeScript source (entry: src/index.ts)
│   ├── index.ts         # Public API surface
│   ├── slug.ts          # URL-safe slug derivation
│   ├── rendering.ts     # Markdown → sanitized HTML pipeline
│   ├── api.ts           # Framework-free HTTP request handler (routing/validation)
│   ├── pages.ts         # Public HTML frontend (index + document pages)
│   ├── server.ts        # node:http transport adapter (routes API vs. pages)
│   ├── main.ts          # Server entrypoint (npm start): pool + server + listen
│   ├── db/              # Postgres schema, migrations, data-access layer
│   └── *.test.ts        # Co-located tests
├── docs/adr/            # Architecture Decision Records
├── .github/workflows/   # CI pipeline
├── tsconfig*.json       # TypeScript configuration
├── eslint.config.js     # ESLint flat config
└── vitest.config.ts     # Test runner configuration
```

Tests are co-located with source as `*.test.ts`.

## API

Documents are managed over a small JSON REST API. The routing/validation core
lives in [`src/api.ts`](src/api.ts) and is framework-free; a thin `node:http`
adapter in [`src/server.ts`](src/server.ts) binds it to a port. Start a server
backed by a Postgres pool:

```ts
import { createPool } from 'inkwell/db';
import { createServer } from 'inkwell';

createServer(createPool(process.env.DATABASE_URL)).listen(3000);
```

All request and response bodies are JSON. Errors share one shape:
`{ "error": { "message": string, ... } }`.

### Endpoints

| Method   | Path               | Description                        | Success | Errors                         |
| -------- | ------------------ | ---------------------------------- | ------- | ------------------------------ |
| `GET`    | `/health`          | Liveness check                     | `200`   | —                              |
| `POST`   | `/documents`       | Create a document                  | `201`   | `400` invalid, `409` dup slug  |
| `GET`    | `/documents`       | List documents (newest first, paged) | `200` | `400` invalid paging           |
| `GET`    | `/documents/:slug` | Fetch one document by slug         | `200`   | `404` not found                |
| `PATCH`  | `/documents/:slug` | Partial update (`PUT` is an alias) | `200`   | `400` invalid, `404` not found |
| `DELETE` | `/documents/:slug` | Delete a document                  | `204`   | `404` not found                |

Unknown paths return `404`; a known path with an unsupported method returns
`405` with an `Allow` hint.

**Request bodies.**

- **Create** (`POST /documents`): `{ "title": string, "bodyMarkdown": string, "slug"?: string }`.
  `title` and `bodyMarkdown` are required; `slug` is optional and derived from
  the title when omitted. An explicit `slug` must be lowercase alphanumerics
  separated by single hyphens.
- **Update** (`PATCH /documents/:slug`): `{ "title"?: string, "bodyMarkdown"?: string }`.
  At least one field is required.

**Listing & pagination.** `GET /documents` accepts `?limit=N` (default `20`,
clamped to a max of `100`) and `?offset=N` (default `0`). Both must be
non-negative integers and `limit` must be at least `1`; anything else is a
`400`. The response wraps the page with the unpaged total so clients can page
through the full set:

```json
{
  "documents": [ /* document objects, newest first */ ],
  "total": 42,
  "limit": 20,
  "offset": 0
}
```

`renderedHtml` is always derived server-side from `bodyMarkdown` via the
[rendering pipeline](#rendering-pipeline) — clients never supply HTML. A
document is returned as:

```json
{
  "id": "uuid",
  "slug": "hello-world",
  "title": "Hello World",
  "bodyMarkdown": "# Hello",
  "renderedHtml": "<h1>Hello</h1>",
  "createdAt": "2026-01-01T00:00:00.000Z",
  "updatedAt": "2026-01-01T00:00:00.000Z"
}
```

The API is integration-tested end to end against the data-access layer (and a
real `node:http` server) using the in-memory Postgres double, so `npm test`
needs no database. See
[`docs/adr/0004-http-api.md`](docs/adr/0004-http-api.md) for the design
rationale.

## Public web frontend

The same server also serves a minimal, styled public website rendered from the
documents' sanitized HTML. The frontend lives in [`src/pages.ts`](src/pages.ts)
and is framework-free in the same spirit as the API: a `handlePageRequest`
handler returns complete HTML pages with an inlined stylesheet (no static-asset
pipeline needed for v0.1). The transport adapter routes any path outside the
reserved API prefixes (`/documents`, `/health`) to the frontend.

| Method | Path     | Description                                    | Success | Errors            |
| ------ | -------- | ---------------------------------------------- | ------- | ----------------- |
| `GET`  | `/`      | Index of published documents (newest first)    | `200`   | —                 |
| `GET`  | `/:slug` | A document's public reading page (styled HTML) | `200`   | `404` styled page |

A document's `renderedHtml` is sanitized at write time by the
[rendering pipeline](#rendering-pipeline), so it is embedded verbatim; every
other interpolated value (titles, etc.) is HTML-escaped. Because the frontend
shares the root with the API, a document whose slug is exactly `documents` or
`health` is unreachable as a public page — those words are reserved.

The current public module surface is:

| Export                      | Description                                                 |
| --------------------------- | ----------------------------------------------------------- |
| `NAME`, `VERSION`           | Package metadata constants                                  |
| `slugify(title)`            | Derive a URL-safe slug from a document title                |
| `renderMarkdown(markdown)`  | Render Markdown to sanitized, XSS-safe HTML                 |
| `renderDocumentHtml(body)`  | Produce a document's `rendered_html` from its Markdown body |
| `createServer(db)`          | Build a `node:http` server for the documents API            |
| `createRequestListener(db)` | Bare request listener, for mounting on an existing server   |
| `handleApiRequest(db, req)` | Framework-free request handler (routing + validation)       |
| `ApiError`                  | Error type carrying an HTTP status                          |
| `handlePageRequest(db,req)` | Framework-free HTML page handler (index + document pages)   |
| `renderIndexPage(docs)`     | Render the index page from a list of documents              |
| `renderDocumentPage(doc)`   | Render a single document's public reading page              |
| `escapeHtml(value)`         | HTML-escape a plain-text value for safe interpolation       |

### Rendering pipeline

Markdown is rendered with [`markdown-it`](https://github.com/markdown-it/markdown-it)
and the output is sanitized with [`sanitize-html`](https://github.com/apostrophecms/sanitize-html)
using a strict allowlist. Authors may use safe inline HTML; anything that can
execute script (`<script>`, `<iframe>`, `on*` handlers, `javascript:` URLs) is
stripped. The document create/update path calls `renderDocumentHtml` to populate
the stored `rendered_html`. See
[`docs/adr/0002-markdown-rendering.md`](docs/adr/0002-markdown-rendering.md).

### Persistence

Documents are stored in PostgreSQL. The schema and a small migration runner live
in [`src/db/`](src/db); the data-access layer is importable from `inkwell/db`.
See [`docs/adr/0003-postgres-persistence.md`](docs/adr/0003-postgres-persistence.md).

`documents` table:

| Column          | Type          | Notes                            |
| --------------- | ------------- | -------------------------------- |
| `id`            | `uuid`        | Primary key, `gen_random_uuid()` |
| `slug`          | `text`        | Unique — the public URL key      |
| `title`         | `text`        |                                  |
| `body_markdown` | `text`        | Authored Markdown source         |
| `rendered_html` | `text`        | Sanitized HTML projection        |
| `created_at`    | `timestamptz` | Default `now()`                  |
| `updated_at`    | `timestamptz` | Default `now()`                  |

Migrations are applied in id order and tracked in a `schema_migrations` ledger.
Point the runner at a database and apply them:

```bash
export DATABASE_URL=postgres://user:pass@localhost:5432/inkwell
npm run build && npm run db:migrate     # apply pending migrations
npm run db:status                       # list applied migration ids
npm run db:rollback                     # roll back the most recent migration
```

The data-access layer maps rows to `camelCase` domain objects:

| Export                        | Description                                     |
| ----------------------------- | ----------------------------------------------- |
| `createPool(url?)`            | Build a `pg` pool from arg or `DATABASE_URL`    |
| `migrate` / `rollback`        | Apply / revert migrations                       |
| `createDocument(db, input)`   | Insert a document (throws `DuplicateSlugError`) |
| `getDocumentBySlug(db, …)`    | Fetch by slug, or `null`                        |
| `getDocumentById(db, …)`      | Fetch by id, or `null`                          |
| `listDocuments(db)`           | List documents, newest first                    |
| `updateDocumentBySlug(db, …)` | Partial update by slug                          |
| `deleteDocumentBySlug(db, …)` | Delete by slug                                  |

The automated test suite runs the migration + CRUD coverage against an in-memory
Postgres ([`pg-mem`](https://github.com/oguimbal/pg-mem)), so no database server
is needed for `npm test`.

## Contributing

1. Branch from `main`.
2. Keep the core small and tested — add tests alongside features.
3. Ensure `npm run ci` passes before opening a pull request.

Releases follow the [v0.1 release checklist](docs/RELEASE-CHECKLIST.md).

## License

[MIT](LICENSE) © Cypress Ink Labs LLC
