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
  createDocument,
  deleteDocumentBySlug,
  getDocumentBySlug,
  listDocuments,
  updateDocumentBySlug,
  type Document,
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

function errorResponse(
  status: number,
  message: string,
  details?: Record<string, unknown>,
): ApiResponse {
  return { status, body: { error: { message, ...details } } };
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
 * Enforce the shared-secret API key on a mutating request. The client must send
 * the configured secret in the `X-API-Key` header. A missing, malformed, or
 * non-matching key — or an unconfigured server secret — results in a 401.
 */
function requireApiKey(req: ApiRequest, configuredKey: string | undefined): void {
  const header = req.headers?.['x-api-key'];
  // A repeated header arrives as an array; reject the ambiguity outright.
  const provided = typeof header === 'string' ? header : undefined;
  if (!configuredKey || !provided || !secretsMatch(provided, configuredKey)) {
    throw new ApiError(401, 'Missing or invalid API key.');
  }
}

/** POST /documents — create a document from `{ title, bodyMarkdown, slug? }`. */
async function handleCreate(db: Queryable, body: unknown): Promise<ApiResponse> {
  const input = asObject(body);
  const title = requireString(input.title, 'title', MAX_TITLE_LENGTH);
  const bodyMarkdown = requireString(input.bodyMarkdown, 'bodyMarkdown', Number.MAX_SAFE_INTEGER);
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

/** GET /documents/:slug — fetch a single document. */
async function handleGet(db: Queryable, slug: string): Promise<ApiResponse> {
  const doc = await getDocumentBySlug(db, slug);
  if (!doc) {
    throw new ApiError(404, `No document with slug "${slug}".`);
  }
  return { status: 200, body: doc };
}

/** GET /documents — list documents, newest first. */
async function handleList(db: Queryable): Promise<ApiResponse> {
  const docs = await listDocuments(db);
  return { status: 200, body: docs };
}

/** PATCH /documents/:slug — partial update of `{ title?, bodyMarkdown? }`. */
async function handleUpdate(db: Queryable, slug: string, body: unknown): Promise<ApiResponse> {
  const input = asObject(body);
  const patch: { title?: string; bodyMarkdown?: string; renderedHtml?: string } = {};

  if (input.title !== undefined) {
    patch.title = requireString(input.title, 'title', MAX_TITLE_LENGTH);
  }
  if (input.bodyMarkdown !== undefined) {
    const bodyMarkdown = requireString(input.bodyMarkdown, 'bodyMarkdown', Number.MAX_SAFE_INTEGER);
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
 *   - `GET    /documents`        -> list
 *   - `POST   /documents`        -> create
 *   - `GET    /documents/:slug`  -> fetch
 *   - `PATCH  /documents/:slug`  -> update (PUT is accepted as an alias)
 *   - `DELETE /documents/:slug`  -> delete
 *
 * Also serves `GET /health` for liveness checks. Unknown paths return 404;
 * known paths with an unsupported method return 405. Any {@link ApiError}
 * thrown by a handler is mapped to its status; anything else surfaces as a 500
 * without leaking internal detail to the client.
 *
 * Mutating routes (`POST`/`PATCH`/`PUT`/`DELETE` under `/documents`) require the
 * shared secret in `options.apiKey` to be presented via the `X-API-Key` header;
 * reads and the health check stay open. See {@link requireApiKey}.
 */
export async function handleApiRequest(
  db: Queryable,
  req: ApiRequest,
  options: ApiOptions = {},
): Promise<ApiResponse> {
  try {
    const [resource, slug, ...rest] = req.segments;

    // Lightweight health check, handy for liveness probes.
    if (resource === 'health' && req.segments.length === 1) {
      if (req.method !== 'GET') return methodNotAllowed(['GET']);
      return { status: 200, body: { status: 'ok' } };
    }

    if (resource !== 'documents' || rest.length > 0) {
      throw new ApiError(404, 'Not found.');
    }

    // Collection route: /documents
    if (slug === undefined) {
      switch (req.method) {
        case 'GET':
          return await handleList(db);
        case 'POST':
          requireApiKey(req, options.apiKey);
          return await handleCreate(db, req.body);
        default:
          return methodNotAllowed(['GET', 'POST']);
      }
    }

    // Item route: /documents/:slug
    switch (req.method) {
      case 'GET':
        return await handleGet(db, slug);
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
