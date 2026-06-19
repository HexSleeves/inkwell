/**
 * Typed data-access layer for the `documents` table.
 *
 * The on-disk schema uses `snake_case` columns; this module maps each row to a
 * `camelCase` {@link Document} domain object so the rest of the codebase never
 * deals in raw SQL column names. All functions take a {@link Queryable}, so the
 * same code runs against a real pool, a transaction client, or a test double.
 */

import type { Queryable } from './pool.js';

/**
 * Publication lifecycle of a document. `draft` is private (author-only);
 * `published` is publicly visible. Stored as a CHECK-constrained text column
 * (see migration `0002`).
 */
export type DocumentStatus = 'draft' | 'published';

/** The full set of valid statuses, for validation at the edges. */
export const DOCUMENT_STATUSES: readonly DocumentStatus[] = ['draft', 'published'];

/** Narrow an arbitrary value to a {@link DocumentStatus}, or `null` if it isn't one. */
export function asDocumentStatus(value: unknown): DocumentStatus | null {
  return DOCUMENT_STATUSES.includes(value as DocumentStatus) ? (value as DocumentStatus) : null;
}

/** A document as stored in Postgres, mapped to domain shape. */
export interface Document {
  readonly id: string;
  readonly slug: string;
  readonly title: string;
  readonly bodyMarkdown: string;
  readonly renderedHtml: string;
  readonly status: DocumentStatus;
  readonly createdAt: Date;
  readonly updatedAt: Date;
}

/** Fields required to create a document. The id and timestamps are assigned by the database. */
export interface NewDocument {
  readonly slug: string;
  readonly title: string;
  readonly bodyMarkdown: string;
  readonly renderedHtml: string;
  /**
   * Initial publication status. Defaults to `draft` (the database default) when
   * omitted, so newly authored documents stay private until explicitly
   * published.
   */
  readonly status?: DocumentStatus;
}

/** Mutable fields when updating an existing document. */
export interface DocumentPatch {
  readonly title?: string;
  readonly bodyMarkdown?: string;
  readonly renderedHtml?: string;
}

/**
 * Optional status filter shared by the read functions. Omit `status` to match
 * documents of any status — the right default for authenticated callers that
 * want everything; public callers should pass `'published'`.
 */
export interface StatusFilter {
  readonly status?: DocumentStatus;
}

/** Raw row shape returned by `SELECT * FROM documents`. */
interface DocumentRow {
  id: string;
  slug: string;
  title: string;
  body_markdown: string;
  rendered_html: string;
  status: DocumentStatus;
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
    status: row.status,
    createdAt: row.created_at,
    updatedAt: row.updated_at,
  };
}

const RETURNING = `id, slug, title, body_markdown, rendered_html, status, created_at, updated_at`;

/**
 * Insert a new document and return the persisted row. When `status` is omitted
 * the database default (`draft`) applies.
 *
 * @throws {DuplicateSlugError} if `slug` is already taken.
 */
export async function createDocument(db: Queryable, input: NewDocument): Promise<Document> {
  try {
    const result = await db.query<DocumentRow>(
      `INSERT INTO documents (slug, title, body_markdown, rendered_html, status)
       VALUES ($1, $2, $3, $4, COALESCE($5, 'draft'))
       RETURNING ${RETURNING}`,
      [input.slug, input.title, input.bodyMarkdown, input.renderedHtml, input.status ?? null],
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

/**
 * Fetch a document by its unique slug, or `null` if none exists. When
 * `filter.status` is supplied, a document whose status differs is treated as
 * not found (returns `null`) — this is how public reads avoid leaking the
 * existence of a draft.
 */
export async function getDocumentBySlug(
  db: Queryable,
  slug: string,
  filter: StatusFilter = {},
): Promise<Document | null> {
  const result = filter.status
    ? await db.query<DocumentRow>(
        `SELECT ${RETURNING} FROM documents WHERE slug = $1 AND status = $2`,
        [slug, filter.status],
      )
    : await db.query<DocumentRow>(`SELECT ${RETURNING} FROM documents WHERE slug = $1`, [slug]);
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

/** Optional paging window and status filter for {@link listDocuments}. */
export interface ListOptions {
  /** Maximum number of rows to return. Omitted means "no limit". */
  readonly limit?: number;
  /** Number of rows to skip from the start of the ordering. Defaults to 0. */
  readonly offset?: number;
  /**
   * Restrict to a single status. Omit to list documents of any status (the
   * default for authenticated callers); public callers pass `'published'`.
   */
  readonly status?: DocumentStatus;
}

/**
 * List documents, newest first.
 *
 * `status` filters by publication state in SQL (before any `LIMIT`, so a page of
 * N rows is N matching documents). `limit`/`offset` page the stable ordering
 * (`created_at`, then `id`) so paging never skips or repeats a row. Use
 * {@link countDocuments} for the unpaged total under the same status filter.
 */
export async function listDocuments(db: Queryable, options: ListOptions = {}): Promise<Document[]> {
  const params: unknown[] = [];
  let sql = `SELECT ${RETURNING} FROM documents`;
  if (options.status) {
    params.push(options.status);
    sql += ` WHERE status = $${params.length}`;
  }
  sql += ` ORDER BY created_at DESC, id DESC`;
  if (options.limit !== undefined) {
    params.push(options.limit);
    sql += ` LIMIT $${params.length}`;
  }
  if (options.offset !== undefined) {
    params.push(options.offset);
    sql += ` OFFSET $${params.length}`;
  }
  const result = await db.query<DocumentRow>(sql, params);
  return result.rows.map(toDocument);
}

/** Options for {@link listPublishedDocuments}. */
export interface ListPublishedOptions {
  /** Maximum number of rows to return. Omitted means "no limit". */
  readonly limit?: number;
}

/**
 * List only `published` documents, newest first.
 *
 * The publication-aware read for public surfaces such as the Atom feed: drafts
 * are never returned. Filtering happens in SQL (before any `LIMIT`) so a page of
 * N rows is N *published* documents, not N rows that may include hidden drafts.
 */
export async function listPublishedDocuments(
  db: Queryable,
  options: ListPublishedOptions = {},
): Promise<Document[]> {
  const params: unknown[] = ['published'];
  let sql = `SELECT ${RETURNING} FROM documents WHERE status = $1 ORDER BY created_at DESC, id DESC`;
  if (options.limit !== undefined) {
    params.push(options.limit);
    sql += ` LIMIT $${params.length}`;
  }
  const result = await db.query<DocumentRow>(sql, params);
  return result.rows.map(toDocument);
}

/**
 * Count documents, optionally restricted to a single status. Kept alongside
 * {@link listDocuments} so paginated reads can report a total consistent with
 * the page — pass the same `status` used for the listing.
 */
export async function countDocuments(db: Queryable, filter: StatusFilter = {}): Promise<number> {
  const result = filter.status
    ? await db.query<{ count: number | string }>(
        `SELECT count(*)::int AS count FROM documents WHERE status = $1`,
        [filter.status],
      )
    : await db.query<{ count: number | string }>(`SELECT count(*)::int AS count FROM documents`);
  // `count(*)` comes back as a number with the `::int` cast, but coerce
  // defensively in case a driver hands it back as a string.
  return Number(result.rows[0]?.count ?? 0);
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

/**
 * Set a document's publication status and return the updated row, or `null` if
 * no such document exists. Idempotent: setting the status a document already has
 * is a successful no-op that returns the row unchanged. `updated_at` is
 * deliberately left untouched so repeated publish/unpublish calls don't churn
 * the displayed "updated" date.
 */
export async function setDocumentStatus(
  db: Queryable,
  slug: string,
  status: DocumentStatus,
): Promise<Document | null> {
  const result = await db.query<DocumentRow>(
    `UPDATE documents SET status = $2 WHERE slug = $1 RETURNING ${RETURNING}`,
    [slug, status],
  );
  const row = result.rows[0];
  return row ? toDocument(row) : null;
}

/** Delete a document by slug. Returns `true` if a row was removed. */
export async function deleteDocumentBySlug(db: Queryable, slug: string): Promise<boolean> {
  const result = await db.query(`DELETE FROM documents WHERE slug = $1`, [slug]);
  return (result.rowCount ?? 0) > 0;
}
