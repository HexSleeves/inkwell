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

/**
 * Documents carry a publication lifecycle: `draft` (private, author-only) or
 * `published` (publicly visible). New rows default to `draft` so nothing leaks
 * before an author opts in. Existing rows predate this column and were already
 * served publicly, so they are backfilled to `published` to preserve behavior.
 *
 * The status is a CHECK-constrained text column rather than a native enum so a
 * future value can be added with a plain migration (no `ALTER TYPE` dance).
 */
const addDocumentStatus: Migration = {
  id: '0002',
  name: 'add_document_status',
  up: `
    ALTER TABLE documents
      ADD COLUMN status text NOT NULL DEFAULT 'draft'
      CHECK (status IN ('draft', 'published'));
    UPDATE documents SET status = 'published';
  `,
  down: `ALTER TABLE documents DROP COLUMN status;`,
};

/**
 * Documents gain free-form **tags** for discovery: a `text[]` column rather than
 * a `document_tags` join table. Tags are a small, unordered set per document
 * that is always read and written alongside the document itself (never queried
 * independently of it), so an array column keeps reads single-row and writes
 * atomic without a join. The trade-off — no referential tag entity — is
 * acceptable for v0.x; a join table can be migrated to later if tags ever need
 * their own metadata (descriptions, colors, rename-with-history).
 *
 * A GIN index over the array backs the `tag = ANY(tags)` containment lookups the
 * tag listing pages and sitemap perform. Existing rows backfill to the empty
 * array via the `DEFAULT '{}'`, so no document is left with a NULL tag set.
 */
const addDocumentTags: Migration = {
  id: '0003',
  name: 'add_document_tags',
  up: `
    ALTER TABLE documents
      ADD COLUMN tags text[] NOT NULL DEFAULT '{}';
    CREATE INDEX documents_tags_idx ON documents USING gin (tags);
  `,
  down: `
    DROP INDEX documents_tags_idx;
    ALTER TABLE documents DROP COLUMN tags;
  `,
};

/** All migrations, in apply order. */
export const MIGRATIONS: readonly Migration[] = [
  createDocuments,
  addDocumentStatus,
  addDocumentTags,
];
