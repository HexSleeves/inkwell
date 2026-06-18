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

/**
 * Build an HTTP request listener bound to a given database. Exposed separately
 * from {@link createServer} so it can be mounted on an existing server or
 * tested in isolation.
 */
export function createRequestListener(db: Queryable) {
  return async (req: IncomingMessage, res: ServerResponse): Promise<void> => {
    try {
      const segments = splitPath(req.url ?? '/');

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

      const response = await handleApiRequest(db, {
        method: (req.method ?? 'GET').toUpperCase(),
        segments,
        body,
      });
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
