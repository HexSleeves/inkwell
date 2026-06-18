/**
 * Ordered schema migrations.
 *
 * Each migration is an immutable record with forward (`up`) and reverse
 * (`down`) SQL. Migrations are applied in ascending `id` order and recorded in
 * the `schema_migrations` table (see `migrate.ts`). Never edit a migration that
 * has shipped — add a new one instead.
 */

export interface Migration {
  /** Zero-padded, monotonically increasing id used for ordering, e.g. `0001`. */
  readonly id: string;
  /** Human-readable name describing the change. */
  readonly name: string;
  /** SQL applied when migrating forward. */
  readonly up: string;
  /** SQL applied when rolling back. */
  readonly down: string;
}

/**
 * The `documents` table is the heart of Inkwell: one row per published
 * document. `slug` is the public URL key and is unique; `body_markdown` is the
 * authored source and `rendered_html` its sanitized HTML projection.
 *
 * `gen_random_uuid()` is built into Postgres 13+ (no extension required).
 */
const createDocuments: Migration = {
  id: '0001',
  name: 'create_documents',
  up: `
    CREATE TABLE documents (
      id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
      slug text NOT NULL UNIQUE,
      title text NOT NULL,
      body_markdown text NOT NULL,
      rendered_html text NOT NULL,
      created_at timestamptz NOT NULL DEFAULT now(),
      updated_at timestamptz NOT NULL DEFAULT now()
    );
  `,
  down: `DROP TABLE documents;`,
};

/** All migrations, in apply order. */
export const MIGRATIONS: readonly Migration[] = [createDocuments];
