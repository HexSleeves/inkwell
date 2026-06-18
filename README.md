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

| Concern     | Choice                                     |
| ----------- | ------------------------------------------ |
| Language    | TypeScript (ESM, `NodeNext`)               |
| Runtime     | Node.js ≥ 20                               |
| Persistence | PostgreSQL _(planned)_                     |
| Test runner | [Vitest](https://vitest.dev)               |
| Lint        | ESLint 9 (flat config) + typescript-eslint |
| Format      | Prettier                                   |
| CI          | GitHub Actions                             |

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
| `npm run ci`            | Lint + format check + typecheck + test + build   |

## Project layout

```
.
├── src/                 # TypeScript source (entry: src/index.ts)
│   ├── index.ts         # Public API surface
│   └── index.test.ts    # Co-located tests
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

| Export            | Description                                  |
| ----------------- | -------------------------------------------- |
| `NAME`, `VERSION` | Package metadata constants                   |
| `slugify(title)`  | Derive a URL-safe slug from a document title |

## Contributing

1. Branch from `main`.
2. Keep the core small and tested — add tests alongside features.
3. Ensure `npm run ci` passes before opening a pull request.

## License

[MIT](LICENSE) © Cypress Ink Labs LLC
