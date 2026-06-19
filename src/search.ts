/**
 * Full-text search over published documents — a JSON API and a public HTML
 * results page sharing one query path.
 *
 * Like `src/feed.ts` and `src/sitemap.ts`, this module is framework-free:
 * {@link handleSearchRequest} takes a normalized {@link SearchRequest} plus a
 * {@link Queryable} and returns a {@link SearchResponse} (status + content-type
 * + body string), so it is integration-tested directly against the data-access
 * layer. The thin `node:http` adapter in `src/server.ts` dispatches `GET
 * /search` here and writes the body with the returned content type.
 *
 * One path serves two shapes: `?format=json` returns the JSON API payload;
 * anything else returns the styled HTML results page (rendered by `src/pages.ts`
 * so all templating stays in one place). Only `published` documents are ever
 * searched — drafts never appear in results. See ADR 0006 for why the match is
 * `ILIKE`-based rather than `tsvector` (the `pg-mem` test harness can't model
 * `tsvector`).
 */

import {
  countSearchPublishedDocuments,
  searchPublishedDocuments,
  type Document,
} from './db/documents.js';
import type { Queryable } from './db/pool.js';
import { PAGE_SIZE, deriveExcerpt, renderSearchPage, type PageOptions } from './pages.js';

/** Content types for the two response shapes. */
const JSON_CONTENT_TYPE = 'application/json; charset=utf-8';
const HTML_CONTENT_TYPE = 'text/html; charset=utf-8';

/** A normalized inbound search request, independent of any HTTP framework. */
export interface SearchRequest {
  /** Upper-case HTTP method, e.g. `GET`. */
  readonly method: string;
  /** Parsed query parameters (`q`, `page`, `format`). */
  readonly query?: Readonly<Record<string, string | undefined>>;
}

/** A search response to be written by the transport adapter. */
export interface SearchResponse {
  readonly status: number;
  /** Value for the `Content-Type` header. */
  readonly contentType: string;
  /** The serialized body (JSON or HTML), or empty for non-200s. */
  readonly body: string;
}

/** One result entry in the JSON payload. */
interface SearchResultJson {
  readonly slug: string;
  readonly title: string;
  readonly excerpt: string;
  readonly tags: readonly string[];
  readonly createdAt: string;
  readonly updatedAt: string;
}

/** Parse a 1-based page number from the query (defaults to 1; junk -> 1). */
function parsePage(raw: string | undefined): number {
  if (raw === undefined || !/^[1-9][0-9]*$/.test(raw)) return 1;
  const n = Number.parseInt(raw, 10);
  return Number.isSafeInteger(n) ? n : 1;
}

function toResultJson(doc: Document): SearchResultJson {
  return {
    slug: doc.slug,
    title: doc.title,
    excerpt: deriveExcerpt(doc.bodyMarkdown),
    tags: doc.tags,
    createdAt: doc.createdAt.toISOString(),
    updatedAt: doc.updatedAt.toISOString(),
  };
}

/**
 * Route and render a single `/search` request.
 *
 * `GET`/`HEAD` only (other methods -> 405). An empty/whitespace `q` is valid: the
 * JSON shape returns zero results and the HTML shape returns just the search
 * form. Results are paginated ({@link PAGE_SIZE} per page) via `?page=N`.
 */
export async function handleSearchRequest(
  db: Queryable,
  req: SearchRequest,
  options: PageOptions = {},
): Promise<SearchResponse> {
  const wantsJson = req.query?.format === 'json';
  const contentType = wantsJson ? JSON_CONTENT_TYPE : HTML_CONTENT_TYPE;

  if (req.method !== 'GET' && req.method !== 'HEAD') {
    return { status: 405, contentType, body: '' };
  }

  const query = req.query?.q ?? '';
  const trimmed = query.trim();
  const page = parsePage(req.query?.page);

  // An empty query short-circuits the DB: nothing to match.
  const total = trimmed === '' ? 0 : await countSearchPublishedDocuments(db, trimmed);
  const totalPages = Math.max(1, Math.ceil(total / PAGE_SIZE));
  const documents =
    trimmed === ''
      ? []
      : await searchPublishedDocuments(db, trimmed, {
          limit: PAGE_SIZE,
          offset: (page - 1) * PAGE_SIZE,
        });

  if (wantsJson) {
    const payload = {
      query: trimmed,
      page,
      pageSize: PAGE_SIZE,
      total,
      results: documents.map(toResultJson),
    };
    return { status: 200, contentType, body: JSON.stringify(payload) };
  }

  const html = renderSearchPage(query, documents, { page, totalPages }, options);
  return { status: 200, contentType, body: html };
}
