/**
 * HTTP API for documents — a small, framework-free request handler.
 *
 * The core of the API is {@link handleApiRequest}: a pure-ish async function
 * that takes a normalized {@link ApiRequest} (method, path segments, parsed
 * body) plus a {@link Queryable} and returns an {@link ApiResponse} (status +
 * JSON body). Keeping the routing/validation logic decoupled from Node's
 * `http` server means it can be integration-tested directly against the
 * pg-mem-backed data-access layer without binding a socket — see
 * `src/api.test.ts`. The thin `node:http` adapter that wires real requests to
 * this handler lives in `src/server.ts`.
 *
 * Conventions:
 *   - Success bodies are the resource itself (or an array for the list route).
 *   - Error bodies are always `{ error: { message, ... } }` so clients have a
 *     single shape to parse regardless of status.
 *   - `slug` is the public URL key. Callers may supply one explicitly or let
 *     the API derive it from the title via {@link slugify}.
 */

import { createHash, timingSafeEqual } from 'node:crypto';

import {
  DuplicateSlugError,
  asDocumentStatus,
  countDocuments,
  createDocument,
  deleteDocumentBySlug,
  getDocumentBySlug,
  listDocuments,
  setDocumentStatus,
  updateDocumentBySlug,
  type Document,
  type DocumentStatus,
  type StatusFilter,
} from './db/documents.js';
import type { Queryable } from './db/pool.js';
import { renderDocumentHtml } from './rendering.js';
import { slugify } from './slug.js';

/** A normalized inbound request, independent of any HTTP framework. */
export interface ApiRequest {
  /** Upper-case HTTP method, e.g. `GET`, `POST`. */
  readonly method: string;
  /** Path split into non-empty segments, e.g. `/documents/x` -> `['documents','x']`. */
  readonly segments: readonly string[];
  /**
   * Parsed JSON request body, or `undefined` when there is no body. Body
   * parsing/`Content-Type` checks happen in the transport adapter; by the time
   * a request reaches the handler the body is already an `unknown` value.
   */
  readonly body?: unknown;
  /**
   * Request headers with lower-cased names, matching Node's
   * `IncomingHttpHeaders` shape. Used for the shared-secret API key check; a
   * repeated header may arrive as a `string[]`. `undefined` when the transport
   * supplies no headers (e.g. direct handler tests).
   */
  readonly headers?: Readonly<Record<string, string | string[] | undefined>>;
  /**
   * Parsed URL query parameters (first value seen per key). `undefined` when the
   * transport supplies none. Used by the list route for `limit`/`offset` paging
   * and the `?status=` visibility filter; the transport adapter parses the
   * query string.
   */
  readonly query?: Readonly<Record<string, string | undefined>>;
}

/** Server-side options for {@link handleApiRequest}. */
export interface ApiOptions {
  /**
   * The shared secret required on mutating requests (POST/PATCH/PUT/DELETE to
   * `/documents`), read from `INKWELL_API_KEY`. When `undefined` or empty, no
   * key can match and all mutations are rejected with 401 — a misconfigured
   * server fails closed rather than serving writes unauthenticated.
   */
  readonly apiKey?: string | undefined;
}

/** A response to be serialized as JSON by the transport adapter. */
export interface ApiResponse {
  readonly status: number;
  /** `undefined` body means "no content" (e.g. a 204). */
  readonly body?: unknown;
}

/**
 * An error carrying the HTTP status it should map to. Thrown inside route
 * handlers and caught centrally in {@link handleApiRequest}, so handlers can
 * `throw new ApiError(400, ...)` instead of threading response shapes around.
 */
export class ApiError extends Error {
  readonly status: number;
  readonly details: Record<string, unknown> | undefined;
  constructor(status: number, message: string, details?: Record<string, unknown>) {
    super(message);
    this.name = 'ApiError';
    this.status = status;
    this.details = details;
  }
}

/** Slugs must be lowercase alphanumerics separated by single hyphens. */
const SLUG_PATTERN = /^[a-z0-9]+(?:-[a-z0-9]+)*$/;
const MAX_SLUG_LENGTH = 200;
const MAX_TITLE_LENGTH = 500;

/**
 * Hard cap on `bodyMarkdown` length. Rendering (markdown-it + highlight.js +
 * sanitize-html) runs synchronously on the event loop, so an oversize or
 * pathological document would block every other request while it renders.
 * Reject oversize input at the API edge with a 400 before it ever reaches the
 * renderer. 256 KiB of Markdown is well beyond any legitimate single document.
 */
const MAX_BODY_MARKDOWN_LENGTH = 256 * 1024; // 262_144 chars (~256 KB)

/** How long the health check waits on the DB before reporting it unreachable. */
const HEALTH_DB_TIMEOUT_MS = 1000;

/** Pagination defaults/bounds for the list route. */
const DEFAULT_LIMIT = 20;
const MAX_LIMIT = 100;

function errorResponse(
  status: number,
  message: string,
  details?: Record<string, unknown>,
): ApiResponse {
  return { status, body: { error: { message, ...details } } };
}

/** Reject with a timeout error if `promise` doesn't settle within `ms`. */
function withTimeout<T>(promise: Promise<T>, ms: number): Promise<T> {
  return new Promise<T>((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error('Operation timed out.')), ms);
    promise.then(
      (value) => {
        clearTimeout(timer);
        resolve(value);
      },
      (error: unknown) => {
        clearTimeout(timer);
        reject(error instanceof Error ? error : new Error(String(error)));
      },
    );
  });
}

/**
 * GET /health — readiness probe. Pings Postgres with `SELECT 1` under a short
 * timeout so the check reflects actual database reachability instead of mere
 * process liveness. Returns 200 `{ status: 'ok', db: 'up' }` when reachable and
 * 503 `{ status: 'error', db: 'down' }` otherwise, letting load balancers and
 * orchestrators route around an instance that can't serve requests.
 */
async function handleHealth(db: Queryable): Promise<ApiResponse> {
  try {
    await withTimeout(db.query('SELECT 1'), HEALTH_DB_TIMEOUT_MS);
    return { status: 200, body: { status: 'ok', db: 'up' } };
  } catch {
    return { status: 503, body: { status: 'error', db: 'down' } };
  }
}

/** Require a value to be a non-empty (after trim) string, else 400. */
function requireString(value: unknown, field: string, maxLength: number): string {
  if (typeof value !== 'string' || value.trim() === '') {
    throw new ApiError(400, `Field "${field}" is required and must be a non-empty string.`);
  }
  if (value.length > maxLength) {
    throw new ApiError(400, `Field "${field}" must be at most ${maxLength} characters.`);
  }
  return value;
}

/** Validate an explicit slug or derive one from the title; throws 400 if unusable. */
function resolveSlug(rawSlug: unknown, title: string): string {
  if (rawSlug === undefined || rawSlug === null) {
    const derived = slugify(title);
    if (derived === '') {
      throw new ApiError(
        400,
        'Could not derive a slug from the title; provide an explicit "slug".',
      );
    }
    return derived;
  }
  if (
    typeof rawSlug !== 'string' ||
    !SLUG_PATTERN.test(rawSlug) ||
    rawSlug.length > MAX_SLUG_LENGTH
  ) {
    throw new ApiError(
      400,
      'Field "slug" must be lowercase alphanumerics separated by single hyphens.',
    );
  }
  return rawSlug;
}

/**
 * Parse a query param that must be a non-negative integer. Only digit strings
 * are accepted, so negatives, floats, and junk like `"12abc"` are rejected with
 * a 400 — keeping numeric validation consistent with the strict field checks
 * used by the write routes.
 */
function parseNonNegativeInt(value: string | undefined, field: string): number | undefined {
  if (value === undefined) {
    return undefined;
  }
  if (!/^\d+$/.test(value)) {
    throw new ApiError(400, `Query param "${field}" must be a non-negative integer.`);
  }
  return Number(value);
}

/**
 * Resolve `limit`/`offset` from the query string into a validated window.
 *   - `limit` defaults to {@link DEFAULT_LIMIT}, must be >= 1, and is clamped
 *     down to {@link MAX_LIMIT} rather than rejected when too large.
 *   - `offset` defaults to 0 and rejects negative values.
 */
function parsePagination(query: ApiRequest['query']): { limit: number; offset: number } {
  let limit = DEFAULT_LIMIT;
  const rawLimit = parseNonNegativeInt(query?.limit, 'limit');
  if (rawLimit !== undefined) {
    if (rawLimit < 1) {
      throw new ApiError(400, 'Query param "limit" must be at least 1.');
    }
    limit = Math.min(rawLimit, MAX_LIMIT);
  }
  const offset = parseNonNegativeInt(query?.offset, 'offset') ?? 0;
  return { limit, offset };
}

function asObject(body: unknown): Record<string, unknown> {
  if (typeof body !== 'object' || body === null || Array.isArray(body)) {
    throw new ApiError(400, 'Request body must be a JSON object.');
  }
  return body as Record<string, unknown>;
}

/**
 * Constant-time comparison of two secrets. Both are SHA-256 hashed first so the
 * comparison runs over fixed-length digests — this avoids leaking the secret's
 * length and keeps `timingSafeEqual` (which throws on length mismatch) safe to
 * call with attacker-controlled input.
 */
function secretsMatch(provided: string, expected: string): boolean {
  const a = createHash('sha256').update(provided).digest();
  const b = createHash('sha256').update(expected).digest();
  return timingSafeEqual(a, b);
}

/**
 * Whether a request carries the valid shared secret in `X-API-Key`. A missing,
 * malformed (array-valued), or non-matching key — or an unconfigured server
 * secret — is unauthenticated. Used both to gate mutations (via
 * {@link requireApiKey}) and to decide whether reads may see drafts.
 */
function isAuthenticated(req: ApiRequest, configuredKey: string | undefined): boolean {
  const header = req.headers?.['x-api-key'];
  // A repeated header arrives as an array; reject the ambiguity outright.
  const provided = typeof header === 'string' ? header : undefined;
  return Boolean(configuredKey && provided && secretsMatch(provided, configuredKey));
}

/**
 * Enforce the shared-secret API key on a mutating request. The client must send
 * the configured secret in the `X-API-Key` header. A missing, malformed, or
 * non-matching key — or an unconfigured server secret — results in a 401.
 */
function requireApiKey(req: ApiRequest, configuredKey: string | undefined): void {
  if (!isAuthenticated(req, configuredKey)) {
    throw new ApiError(401, 'Missing or invalid API key.');
  }
}

/** POST /documents — create a document from `{ title, bodyMarkdown, slug? }`. */
async function handleCreate(db: Queryable, body: unknown): Promise<ApiResponse> {
  const input = asObject(body);
  const title = requireString(input.title, 'title', MAX_TITLE_LENGTH);
  const bodyMarkdown = requireString(input.bodyMarkdown, 'bodyMarkdown', MAX_BODY_MARKDOWN_LENGTH);
  const slug = resolveSlug(input.slug, title);
  try {
    const doc = await createDocument(db, {
      slug,
      title,
      bodyMarkdown,
      renderedHtml: renderDocumentHtml(bodyMarkdown),
    });
    return { status: 201, body: doc };
  } catch (error) {
    if (error instanceof DuplicateSlugError) {
      throw new ApiError(409, error.message, { slug: error.slug });
    }
    throw error;
  }
}

/**
 * GET /documents/:slug — fetch a single document.
 *
 * Unauthenticated callers only ever see `published` documents; a draft is
 * reported as 404 so its existence isn't leaked. Authenticated callers (valid
 * API key) see documents of any status.
 */
async function handleGet(db: Queryable, slug: string, authed: boolean): Promise<ApiResponse> {
  const filter: StatusFilter = authed ? {} : { status: 'published' };
  const doc = await getDocumentBySlug(db, slug, filter);
  if (!doc) {
    throw new ApiError(404, `No document with slug "${slug}".`);
  }
  return { status: 200, body: doc };
}

/**
 * GET /documents — list documents, newest first, one page at a time.
 *
 * Reads `?limit=N&offset=N` (see {@link parsePagination}) and returns the page
 * alongside the unpaged `total` for the same filter:
 * `{ documents, total, limit, offset }`.
 *
 * Visibility depends on authentication:
 *   - Unauthenticated: only `published` documents, always (the `status` query
 *     param is ignored so drafts can never leak).
 *   - Authenticated: every status by default (drafts included); narrow with
 *     `?status=published`, `?status=draft`, or the explicit `?status=all`. An
 *     unrecognized value is a 400.
 */
async function handleList(
  db: Queryable,
  req: ApiRequest,
  configuredKey: string | undefined,
): Promise<ApiResponse> {
  const status = resolveListStatus(req, configuredKey);
  const { limit, offset } = parsePagination(req.query);
  const filter: StatusFilter = status ? { status } : {};
  const [documents, total] = await Promise.all([
    listDocuments(db, { ...filter, limit, offset }),
    countDocuments(db, filter),
  ]);
  return { status: 200, body: { documents, total, limit, offset } };
}

/**
 * Resolve which status an authenticated list request is asking for. Returns the
 * status to filter by, or `undefined` for "all statuses". Unauthenticated
 * callers are pinned to `published` and the `?status` param is ignored.
 */
function resolveListStatus(
  req: ApiRequest,
  configuredKey: string | undefined,
): DocumentStatus | undefined {
  if (!isAuthenticated(req, configuredKey)) {
    return 'published';
  }
  const raw = req.query?.status;
  if (raw === undefined || raw === 'all') {
    return undefined;
  }
  const status = asDocumentStatus(raw);
  if (!status) {
    throw new ApiError(400, 'Query param "status" must be one of: draft, published, all.');
  }
  return status;
}

/**
 * POST /documents/:slug/publish and .../unpublish — flip a document's status.
 * Requires the API key (enforced by the caller). Idempotent: re-publishing an
 * already-published document (or vice versa) returns 200 with the row.
 */
async function handleSetStatus(
  db: Queryable,
  slug: string,
  status: 'draft' | 'published',
): Promise<ApiResponse> {
  const doc = await setDocumentStatus(db, slug, status);
  if (!doc) {
    throw new ApiError(404, `No document with slug "${slug}".`);
  }
  return { status: 200, body: doc };
}

/** PATCH /documents/:slug — partial update of `{ title?, bodyMarkdown? }`. */
async function handleUpdate(db: Queryable, slug: string, body: unknown): Promise<ApiResponse> {
  const input = asObject(body);
  const patch: { title?: string; bodyMarkdown?: string; renderedHtml?: string } = {};

  if (input.title !== undefined) {
    patch.title = requireString(input.title, 'title', MAX_TITLE_LENGTH);
  }
  if (input.bodyMarkdown !== undefined) {
    const bodyMarkdown = requireString(
      input.bodyMarkdown,
      'bodyMarkdown',
      MAX_BODY_MARKDOWN_LENGTH,
    );
    patch.bodyMarkdown = bodyMarkdown;
    // Re-render so the stored HTML never drifts from the Markdown source.
    patch.renderedHtml = renderDocumentHtml(bodyMarkdown);
  }
  if (patch.title === undefined && patch.bodyMarkdown === undefined) {
    throw new ApiError(400, 'Provide at least one of "title" or "bodyMarkdown" to update.');
  }

  const updated = await updateDocumentBySlug(db, slug, patch);
  if (!updated) {
    throw new ApiError(404, `No document with slug "${slug}".`);
  }
  return { status: 200, body: updated };
}

/** DELETE /documents/:slug — remove a document. */
async function handleDelete(db: Queryable, slug: string): Promise<ApiResponse> {
  const removed = await deleteDocumentBySlug(db, slug);
  if (!removed) {
    throw new ApiError(404, `No document with slug "${slug}".`);
  }
  return { status: 204 };
}

function methodNotAllowed(allowed: string[]): ApiResponse {
  return {
    status: 405,
    body: { error: { message: 'Method not allowed.', allow: allowed.join(', ') } },
  };
}

/**
 * Route and execute a single request against the documents resource.
 *
 * Recognized routes (all under `/documents`):
 *   - `GET    /documents`                 -> list (published-only unless authed)
 *   - `POST   /documents`                 -> create (defaults to draft)
 *   - `GET    /documents/:slug`           -> fetch (draft 404s unless authed)
 *   - `PATCH  /documents/:slug`           -> update (PUT is accepted as an alias)
 *   - `DELETE /documents/:slug`           -> delete
 *   - `POST   /documents/:slug/publish`   -> mark published (idempotent)
 *   - `POST   /documents/:slug/unpublish` -> mark draft (idempotent)
 *
 * Also serves `GET /health` as a DB-aware readiness check (503 if Postgres is
 * unreachable). Unknown paths return 404;
 * known paths with an unsupported method return 405. Any {@link ApiError}
 * thrown by a handler is mapped to its status; anything else surfaces as a 500
 * without leaking internal detail to the client.
 *
 * Mutating routes (`POST`/`PATCH`/`PUT`/`DELETE` and the publish/unpublish
 * actions under `/documents`) require the shared secret in `options.apiKey` to
 * be presented via the `X-API-Key` header. Reads stay open, but an
 * unauthenticated reader only ever sees `published` documents — drafts are
 * invisible (list) or 404 (single get) so their existence isn't leaked. A valid
 * key on a read unlocks draft visibility. See {@link requireApiKey} and
 * {@link isAuthenticated}.
 */
export async function handleApiRequest(
  db: Queryable,
  req: ApiRequest,
  options: ApiOptions = {},
): Promise<ApiResponse> {
  try {
    const [resource, slug, action, ...extra] = req.segments;

    // DB-aware readiness check: pings Postgres so the probe fails when the
    // database is unreachable rather than reporting healthy on liveness alone.
    if (resource === 'health' && req.segments.length === 1) {
      if (req.method !== 'GET') return methodNotAllowed(['GET']);
      return await handleHealth(db);
    }

    if (resource !== 'documents') {
      throw new ApiError(404, 'Not found.');
    }

    // Collection route: /documents
    if (slug === undefined) {
      switch (req.method) {
        case 'GET':
          return await handleList(db, req, options.apiKey);
        case 'POST':
          requireApiKey(req, options.apiKey);
          return await handleCreate(db, req.body);
        default:
          return methodNotAllowed(['GET', 'POST']);
      }
    }

    // Action route: /documents/:slug/(publish|unpublish)
    if (action !== undefined && extra.length === 0) {
      if (action !== 'publish' && action !== 'unpublish') {
        throw new ApiError(404, 'Not found.');
      }
      if (req.method !== 'POST') {
        return methodNotAllowed(['POST']);
      }
      requireApiKey(req, options.apiKey);
      return await handleSetStatus(db, slug, action === 'publish' ? 'published' : 'draft');
    }

    // Any deeper path is not a route.
    if (extra.length > 0) {
      throw new ApiError(404, 'Not found.');
    }

    // Item route: /documents/:slug
    switch (req.method) {
      case 'GET':
        return await handleGet(db, slug, isAuthenticated(req, options.apiKey));
      case 'PATCH':
      case 'PUT':
        requireApiKey(req, options.apiKey);
        return await handleUpdate(db, slug, req.body);
      case 'DELETE':
        requireApiKey(req, options.apiKey);
        return await handleDelete(db, slug);
      default:
        return methodNotAllowed(['GET', 'PATCH', 'PUT', 'DELETE']);
    }
  } catch (error) {
    if (error instanceof ApiError) {
      return errorResponse(error.status, error.message, error.details);
    }
    // Unexpected failure: don't leak internals, but make it a real 500.
    return errorResponse(500, 'Internal server error.');
  }
}

/** Re-export the domain type for API consumers. */
export type { Document };
