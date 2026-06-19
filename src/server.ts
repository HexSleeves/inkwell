/**
 * `node:http` transport adapter for the documents API.
 *
 * This module is the only place that touches Node's HTTP types. It reads the
 * request body, normalizes the request into an {@link ApiRequest}, delegates
 * all routing/validation/business logic to {@link handleApiRequest}, and
 * serializes the {@link ApiResponse} back to the wire. Keeping the adapter this
 * thin means the interesting behavior is covered by the framework-free
 * integration tests in `src/api.test.ts`.
 */

import {
  createServer as createHttpServer,
  type IncomingMessage,
  type ServerResponse,
} from 'node:http';

import { handleApiRequest, type ApiResponse } from './api.js';
import type { Queryable } from './db/pool.js';
import { handleFeedRequest, type FeedResponse } from './feed.js';
import { handlePageRequest, type PageResponse } from './pages.js';
import { handleSitemapRequest, type SitemapResponse } from './sitemap.js';

/**
 * Path prefixes reserved for the JSON API. Any other path is served by the
 * public HTML frontend (see `src/pages.ts`), so `GET /:slug` renders a document
 * page and `GET /` renders the index. A consequence is that a document whose
 * slug is exactly `documents` or `health` is unreachable as a public page;
 * those words are reserved for the API.
 */
const API_PREFIXES = new Set(['documents', 'health']);

/** Cap request bodies so a client can't exhaust memory with an endless stream. */
const MAX_BODY_BYTES = 1_000_000; // 1 MB

/** Split a URL path into non-empty, decoded segments. */
function splitPath(rawUrl: string): string[] {
  // The host is irrelevant here; we only need the pathname.
  const path = rawUrl.split('?')[0] ?? '';
  return path
    .split('/')
    .filter((segment) => segment.length > 0)
    .map((segment) => decodeURIComponent(segment));
}

/**
 * Parse the query string into a flat map, keeping the first value seen for any
 * repeated key — matching the `ApiRequest.query` contract the handler expects.
 */
function parseQuery(rawUrl: string): Record<string, string> {
  const params = new URLSearchParams(rawUrl.split('?')[1] ?? '');
  const query: Record<string, string> = {};
  for (const [key, value] of params) {
    if (!(key in query)) {
      query[key] = value;
    }
  }
  return query;
}

/** Read the full request body as a string, rejecting if it exceeds the cap. */
function readBody(req: IncomingMessage): Promise<string> {
  return new Promise((resolve, reject) => {
    const chunks: Buffer[] = [];
    let total = 0;
    req.on('data', (chunk: Buffer) => {
      total += chunk.length;
      if (total > MAX_BODY_BYTES) {
        reject(new Error('Request body too large.'));
        req.destroy();
        return;
      }
      chunks.push(chunk);
    });
    req.on('end', () => resolve(Buffer.concat(chunks).toString('utf8')));
    req.on('error', reject);
  });
}

function writeResponse(res: ServerResponse, response: ApiResponse): void {
  if (response.body === undefined) {
    res.writeHead(response.status);
    res.end();
    return;
  }
  const payload = JSON.stringify(response.body);
  res.writeHead(response.status, {
    'content-type': 'application/json; charset=utf-8',
    'content-length': Buffer.byteLength(payload),
  });
  res.end(payload);
}

/** Serialize an HTML page response from the public frontend. */
function writeHtmlResponse(res: ServerResponse, method: string, response: PageResponse): void {
  const payload = Buffer.from(response.html, 'utf8');
  res.writeHead(response.status, {
    'content-type': 'text/html; charset=utf-8',
    'content-length': payload.length,
  });
  // HEAD requests get headers (including content-length) but no body.
  res.end(method === 'HEAD' ? undefined : payload);
}

/**
 * Serialize an XML response (Atom feed or sitemap) from one of the syndication
 * modules. Both responses share the same `{ status, contentType, xml }` shape.
 */
function writeXmlResponse(
  res: ServerResponse,
  method: string,
  response: FeedResponse | SitemapResponse,
): void {
  const payload = Buffer.from(response.xml, 'utf8');
  res.writeHead(response.status, {
    'content-type': response.contentType,
    'content-length': payload.length,
  });
  // HEAD requests get headers (including content-length) but no body.
  res.end(method === 'HEAD' ? undefined : payload);
}

/**
 * Build an HTTP request listener bound to a given database. Exposed separately
 * from {@link createServer} so it can be mounted on an existing server or
 * tested in isolation.
 */
export function createRequestListener(db: Queryable) {
  return async (req: IncomingMessage, res: ServerResponse): Promise<void> => {
    try {
      const segments = splitPath(req.url ?? '/');
      const method = (req.method ?? 'GET').toUpperCase();

      // The configured public origin, used to build absolute URLs across the
      // feed, sitemap, and the HTML pages' canonical/OpenGraph metadata.
      const siteUrl = process.env.INKWELL_SITE_URL;

      // Atom syndication feed. Lives at a fixed top-level path so it never
      // shadows a document slug; served with its own XML content type.
      if (segments.length === 1 && segments[0] === 'feed.xml') {
        const feedResponse = await handleFeedRequest(db, { method }, { siteUrl });
        writeXmlResponse(res, method, feedResponse);
        return;
      }

      // sitemap.xml — fixed top-level path, served as XML, listing public URLs.
      if (segments.length === 1 && segments[0] === 'sitemap.xml') {
        const sitemapResponse = await handleSitemapRequest(db, { method }, { siteUrl });
        writeXmlResponse(res, method, sitemapResponse);
        return;
      }

      // Anything outside the reserved API prefixes is a public HTML page.
      if (segments.length === 0 || !API_PREFIXES.has(segments[0] as string)) {
        const pageResponse = await handlePageRequest(db, { method, segments }, { siteUrl });
        writeHtmlResponse(res, method, pageResponse);
        return;
      }

      let body: unknown;
      const raw = await readBody(req);
      if (raw.length > 0) {
        try {
          body = JSON.parse(raw);
        } catch {
          writeResponse(res, {
            status: 400,
            body: { error: { message: 'Request body must be valid JSON.' } },
          });
          return;
        }
      }

      // The shared secret is read per request so it can be rotated by
      // restarting with a new env value (and so tests can set it freely).
      const apiKey = process.env.INKWELL_API_KEY;
      const response = await handleApiRequest(
        db,
        { method, segments, body, headers: req.headers, query: parseQuery(req.url ?? '/') },
        { apiKey },
      );
      writeResponse(res, response);
    } catch (error) {
      const tooLarge = error instanceof Error && error.message === 'Request body too large.';
      writeResponse(res, {
        status: tooLarge ? 413 : 500,
        body: {
          error: { message: tooLarge ? 'Request body too large.' : 'Internal server error.' },
        },
      });
    }
  };
}

/**
 * Create an HTTP server for the documents API backed by `db`.
 *
 * The caller owns the lifecycle: `createServer(db).listen(port)`. The database
 * pool is injected so the same wiring serves production (a real `pg` pool) and
 * any future smoke tests (a pg-mem adapter).
 */
export function createServer(db: Queryable) {
  return createHttpServer(createRequestListener(db));
}
