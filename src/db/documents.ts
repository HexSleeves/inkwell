/**
 * Typed data-access layer for the `documents` table.
 *
 * The on-disk schema uses `snake_case` columns; this module maps each row to a
 * `camelCase` {@link Document} domain object so the rest of the codebase never
 * deals in raw SQL column names. All functions take a {@link Queryable}, so the
 * same code runs against a real pool, a transaction client, or a test double.
 */

import type { Queryable } from './pool.js';

/** A published document as stored in Postgres, mapped to domain shape. */
export interface Document {
  readonly id: string;
  readonly slug: string;
  readonly title: string;
  readonly bodyMarkdown: string;
  readonly renderedHtml: string;
  readonly createdAt: Date;
  readonly updatedAt: Date;
}

/** Fields required to create a document. The id and timestamps are assigned by the database. */
export interface NewDocument {
  readonly slug: string;
  readonly title: string;
  readonly bodyMarkdown: string;
  readonly renderedHtml: string;
}

/** Mutable fields when updating an existing document. */
export interface DocumentPatch {
  readonly title?: string;
  readonly bodyMarkdown?: string;
  readonly renderedHtml?: string;
}

/** Raw row shape returned by `SELECT * FROM documents`. */
interface DocumentRow {
  id: string;
  slug: string;
  title: string;
  body_markdown: string;
  rendered_html: string;
  created_at: Date;
  updated_at: Date;
}

/** Postgres unique-violation error code. */
const UNIQUE_VIOLATION = '23505';

/**
 * Thrown when an insert/update would collide with an existing document slug.
 * Callers (e.g. the HTTP layer) can map this to a 409 without sniffing driver
 * error codes themselves.
 */
export class DuplicateSlugError extends Error {
  readonly slug: string;
  constructor(slug: string) {
    super(`A document with slug "${slug}" already exists.`);
    this.name = 'DuplicateSlugError';
    this.slug = slug;
  }
}

function isUniqueViolation(error: unknown): boolean {
  return (
    typeof error === 'object' &&
    error !== null &&
    'code' in error &&
    (error as { code?: unknown }).code === UNIQUE_VIOLATION
  );
}

function toDocument(row: DocumentRow): Document {
  return {
    id: row.id,
    slug: row.slug,
    title: row.title,
    bodyMarkdown: row.body_markdown,
    renderedHtml: row.rendered_html,
    createdAt: row.created_at,
    updatedAt: row.updated_at,
  };
}

const RETURNING = `id, slug, title, body_markdown, rendered_html, created_at, updated_at`;

/**
 * Insert a new document and return the persisted row.
 *
 * @throws {DuplicateSlugError} if `slug` is already taken.
 */
export async function createDocument(db: Queryable, input: NewDocument): Promise<Document> {
  try {
    const result = await db.query<DocumentRow>(
      `INSERT INTO documents (slug, title, body_markdown, rendered_html)
       VALUES ($1, $2, $3, $4)
       RETURNING ${RETURNING}`,
      [input.slug, input.title, input.bodyMarkdown, input.renderedHtml],
    );
    // INSERT ... RETURNING always yields exactly one row on success.
    return toDocument(result.rows[0] as DocumentRow);
  } catch (error) {
    if (isUniqueViolation(error)) {
      throw new DuplicateSlugError(input.slug);
    }
    throw error;
  }
}

/** Fetch a document by its unique slug, or `null` if none exists. */
export async function getDocumentBySlug(db: Queryable, slug: string): Promise<Document | null> {
  const result = await db.query<DocumentRow>(`SELECT ${RETURNING} FROM documents WHERE slug = $1`, [
    slug,
  ]);
  const row = result.rows[0];
  return row ? toDocument(row) : null;
}

/** Fetch a document by id, or `null` if none exists. */
export async function getDocumentById(db: Queryable, id: string): Promise<Document | null> {
  const result = await db.query<DocumentRow>(`SELECT ${RETURNING} FROM documents WHERE id = $1`, [
    id,
  ]);
  const row = result.rows[0];
  return row ? toDocument(row) : null;
}

/** List documents, newest first. */
export async function listDocuments(db: Queryable): Promise<Document[]> {
  const result = await db.query<DocumentRow>(
    `SELECT ${RETURNING} FROM documents ORDER BY created_at DESC, id DESC`,
  );
  return result.rows.map(toDocument);
}

/**
 * Apply a partial update to the document with the given slug and return the
 * updated row, or `null` if no such document exists. Touches `updated_at`.
 * Passing an empty patch is a no-op read of the current row.
 */
export async function updateDocumentBySlug(
  db: Queryable,
  slug: string,
  patch: DocumentPatch,
): Promise<Document | null> {
  const result = await db.query<DocumentRow>(
    `UPDATE documents
        SET title = COALESCE($2, title),
            body_markdown = COALESCE($3, body_markdown),
            rendered_html = COALESCE($4, rendered_html),
            updated_at = now()
      WHERE slug = $1
      RETURNING ${RETURNING}`,
    [slug, patch.title ?? null, patch.bodyMarkdown ?? null, patch.renderedHtml ?? null],
  );
  const row = result.rows[0];
  return row ? toDocument(row) : null;
}

/** Delete a document by slug. Returns `true` if a row was removed. */
export async function deleteDocumentBySlug(db: Queryable, slug: string): Promise<boolean> {
  const result = await db.query(`DELETE FROM documents WHERE slug = $1`, [slug]);
  return (result.rowCount ?? 0) > 0;
}
