# ADR 0003: Postgres persistence and migrations

- **Status:** Accepted
- **Date:** 2026-06-18

## Context

ADR 0001 named PostgreSQL as the planned persistence layer but deliberately kept
it out of the founding scaffold. The document model now needs durable storage:
a `documents` table, a way to evolve the schema over time, and a typed
data-access layer the rest of the codebase can build on without writing raw SQL
everywhere.

## Decision

- **Driver: [`pg`](https://node-postgres.com) (node-postgres).** The boring,
  ubiquitous Postgres client for Node. No ORM — Inkwell's core is small and a
  thin, typed query layer keeps the SQL legible and the dependency surface low.
- **`documents` table.** Columns: `id uuid` (PK, `gen_random_uuid()` default),
  `slug text` (unique — the public URL key), `title text`, `body_markdown text`
  (authored source), `rendered_html text` (sanitized HTML projection),
  `created_at`/`updated_at timestamptz` (default `now()`). `gen_random_uuid()`
  is built into Postgres 13+, so no extension is required.
- **Home-grown migration runner** over a heavier framework. Migrations are
  ordered, immutable `{ id, name, up, down }` records applied in id order and
  recorded in a `schema_migrations` ledger, so runs are idempotent and a partial
  failure is resumable. Rollback runs the reverse SQL newest-first. A small CLI
  (`pnpm run db:migrate` / `db:rollback` / `db:status`) operates it against
  `DATABASE_URL`.
- **Driver-agnostic data-access layer.** All persistence functions take a small
  `Queryable` interface (anything with a compatible `query` method), not a
  concrete pool. `snake_case` rows are mapped to `camelCase` domain objects, and
  a unique-slug collision surfaces as a typed `DuplicateSlugError`.
- **Tests run on [`pg-mem`](https://github.com/oguimbal/pg-mem),** an in-memory
  Postgres, so the migration + CRUD suite runs in CI with no external service.
  Each test gets a fresh database, which is exactly the "applies cleanly to a
  fresh DB" success condition.

## Consequences

- `pnpm test` exercises migrate + insert/select/update/delete with zero infra.
- pg-mem implements only a subset of Postgres, so the test harness registers
  `gen_random_uuid()` and enables `noAstCoverageCheck`. Two real-Postgres
  behaviours pg-mem cannot model (re-creating a table after `DROP` in the same
  instance) are documented in the tests rather than asserted. Validating against
  a real Postgres is a `pnpm run db:migrate` away and a good candidate for a
  future CI service-container job.
- No ORM means schema changes are explicit SQL migrations — more typing, but no
  hidden magic and full control over generated DDL.
