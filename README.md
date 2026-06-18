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

## npm scripts

| Script                  | What it does                                     |
| ----------------------- | ------------------------------------------------ |
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
│   ├── rendering.ts     # Markdown → sanitized HTML pipeline
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

The HTTP API is not yet implemented. As endpoints land they will be documented
here as an endpoint table (method, path, purpose). The current public module
surface is:

| Export                     | Description                                                 |
| -------------------------- | ----------------------------------------------------------- |
| `NAME`, `VERSION`          | Package metadata constants                                  |
| `slugify(title)`           | Derive a URL-safe slug from a document title                |
| `renderMarkdown(markdown)` | Render Markdown to sanitized, XSS-safe HTML                 |
| `renderDocumentHtml(body)` | Produce a document's `rendered_html` from its Markdown body |

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

## License

[MIT](LICENSE) © Cypress Ink Labs LLC
