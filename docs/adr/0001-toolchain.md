# ADR 0001: Founding toolchain

- **Status:** Accepted
- **Date:** 2026-06-18

## Context

Inkwell needs a founding TypeScript/Node toolchain that a small team can trust:
fast feedback, typed code, consistent formatting, and a green test run as part
of "done". Choices should be boring and well-supported so they do not become a
maintenance burden.

## Decision

- **TypeScript, ESM, `NodeNext`** module resolution. ESM is the modern default
  and `NodeNext` matches how Node actually resolves modules, avoiding
  bundler-specific surprises. `strict` plus extra safety flags
  (`noUncheckedIndexedAccess`, `exactOptionalPropertyTypes`,
  `verbatimModuleSyntax`) are on from day one — cheap to adopt now, painful to
  retrofit later.
- **Node ≥ 20.** Current LTS baseline; CI also exercises 22.x.
- **Vitest** as the test runner. First-class TypeScript/ESM support with no
  extra transform config, fast watch mode, and built-in V8 coverage.
- **ESLint 9 flat config + typescript-eslint** for linting, with
  **Prettier** owning formatting (`eslint-config-prettier` disables stylistic
  ESLint rules so the two never fight).
- **GitHub Actions** for CI: lint → format check → typecheck → test → build,
  across Node 20 and 22.

## Consequences

- A single `npm run ci` reproduces the CI gate locally.
- Build output (`dist/`) ships compiled JS plus `.d.ts` declarations, so the
  package is consumable as a library while the HTTP service is built out.
- Postgres is the planned persistence layer (typed schemas + migrations) but is
  intentionally **not** part of this scaffold; it will arrive with the document
  model work.
