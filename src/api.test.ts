/**
 * Integration tests for the documents HTTP API.
 *
 * These exercise the full request handler against the real data-access layer,
 * backed by an in-memory Postgres (`pg-mem`) with migrations applied — so the
 * routing, validation, rendering, and SQL all run together. No socket is bound;
 * `handleApiRequest` is called directly. A separate suite at the bottom binds a
 * real `node:http` server to prove the transport adapter wires up end to end.
 */

import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import type { AddressInfo } from 'node:net';
import type { Server } from 'node:http';

import { handleApiRequest, type ApiRequest, type ApiResponse } from './api.js';
import { createServer } from './server.js';
import { migrate } from './db/migrate.js';
import { createMemoryDatabase } from './db/test-helpers.js';
import type { Queryable } from './db/pool.js';

interface DocumentBody {
  id: string;
  slug: string;
  title: string;
  bodyMarkdown: string;
  renderedHtml: string;
  createdAt: string;
  updatedAt: string;
}

describe('documents HTTP API (handler)', () => {
  let db: Queryable;

  beforeEach(async () => {
    db = createMemoryDatabase().db;
    await migrate(db);
  });

  const call = (req: ApiRequest): Promise<ApiResponse> => handleApiRequest(db, req);

  const createSample = (overrides: Record<string, unknown> = {}): Promise<ApiResponse> =>
    call({
      method: 'POST',
      segments: ['documents'],
      body: { title: 'Hello World', bodyMarkdown: '# Hi', ...overrides },
    });

  describe('POST /documents', () => {
    it('creates a document, deriving the slug and rendering HTML', async () => {
      const res = await createSample();
      expect(res.status).toBe(201);
      const doc = res.body as DocumentBody;
      expect(doc.slug).toBe('hello-world');
      expect(doc.title).toBe('Hello World');
      expect(doc.bodyMarkdown).toBe('# Hi');
      expect(doc.renderedHtml).toContain('<h1>Hi</h1>');
      expect(doc.id).toMatch(/^[0-9a-f-]{36}$/);
    });

    it('accepts an explicit valid slug', async () => {
      const res = await createSample({ slug: 'custom-slug' });
      expect(res.status).toBe(201);
      expect((res.body as DocumentBody).slug).toBe('custom-slug');
    });

    it('sanitizes dangerous HTML in the rendered output', async () => {
      const res = await createSample({ bodyMarkdown: 'Hi <script>alert(1)</script>' });
      expect(res.status).toBe(201);
      expect((res.body as DocumentBody).renderedHtml).not.toContain('<script>');
    });

    it('rejects a missing title with 400', async () => {
      const res = await call({
        method: 'POST',
        segments: ['documents'],
        body: { bodyMarkdown: 'x' },
      });
      expect(res.status).toBe(400);
    });

    it('rejects a blank title with 400', async () => {
      const res = await createSample({ title: '   ' });
      expect(res.status).toBe(400);
    });

    it('rejects a non-object body with 400', async () => {
      const res = await call({ method: 'POST', segments: ['documents'], body: 'nope' });
      expect(res.status).toBe(400);
    });

    it('rejects an invalid explicit slug with 400', async () => {
      const res = await createSample({ slug: 'Not Valid!' });
      expect(res.status).toBe(400);
    });

    it('rejects a title that yields an empty slug when no slug is given', async () => {
      const res = await createSample({ title: '!!!' });
      expect(res.status).toBe(400);
    });

    it('rejects a duplicate slug with 409', async () => {
      await createSample();
      const res = await createSample();
      expect(res.status).toBe(409);
      expect((res.body as { error: { slug: string } }).error.slug).toBe('hello-world');
    });
  });

  describe('GET /documents', () => {
    it('lists documents', async () => {
      await createSample({ slug: 'a' });
      await createSample({ slug: 'b' });
      const res = await call({ method: 'GET', segments: ['documents'] });
      expect(res.status).toBe(200);
      expect((res.body as DocumentBody[]).map((d) => d.slug).sort()).toEqual(['a', 'b']);
    });

    it('returns an empty array when there are no documents', async () => {
      const res = await call({ method: 'GET', segments: ['documents'] });
      expect(res.status).toBe(200);
      expect(res.body).toEqual([]);
    });
  });

  describe('GET /documents/:slug', () => {
    it('fetches an existing document', async () => {
      await createSample();
      const res = await call({ method: 'GET', segments: ['documents', 'hello-world'] });
      expect(res.status).toBe(200);
      expect((res.body as DocumentBody).slug).toBe('hello-world');
    });

    it('returns 404 for a missing slug', async () => {
      const res = await call({ method: 'GET', segments: ['documents', 'ghost'] });
      expect(res.status).toBe(404);
    });
  });

  describe('PATCH /documents/:slug', () => {
    it('updates the title without touching the body', async () => {
      await createSample();
      const res = await call({
        method: 'PATCH',
        segments: ['documents', 'hello-world'],
        body: { title: 'Renamed' },
      });
      expect(res.status).toBe(200);
      const doc = res.body as DocumentBody;
      expect(doc.title).toBe('Renamed');
      expect(doc.bodyMarkdown).toBe('# Hi');
    });

    it('re-renders HTML when the body changes', async () => {
      await createSample();
      const res = await call({
        method: 'PATCH',
        segments: ['documents', 'hello-world'],
        body: { bodyMarkdown: '## Changed' },
      });
      expect(res.status).toBe(200);
      const doc = res.body as DocumentBody;
      expect(doc.bodyMarkdown).toBe('## Changed');
      expect(doc.renderedHtml).toContain('<h2>Changed</h2>');
    });

    it('rejects an empty patch with 400', async () => {
      await createSample();
      const res = await call({
        method: 'PATCH',
        segments: ['documents', 'hello-world'],
        body: {},
      });
      expect(res.status).toBe(400);
    });

    it('returns 404 when updating a missing document', async () => {
      const res = await call({
        method: 'PATCH',
        segments: ['documents', 'ghost'],
        body: { title: 'x' },
      });
      expect(res.status).toBe(404);
    });
  });

  describe('DELETE /documents/:slug', () => {
    it('deletes an existing document and is idempotent on a second call', async () => {
      await createSample();
      const first = await call({ method: 'DELETE', segments: ['documents', 'hello-world'] });
      expect(first.status).toBe(204);
      expect(first.body).toBeUndefined();
      const second = await call({ method: 'DELETE', segments: ['documents', 'hello-world'] });
      expect(second.status).toBe(404);
    });
  });

  describe('routing and method handling', () => {
    it('returns 404 for unknown routes', async () => {
      const res = await call({ method: 'GET', segments: ['widgets'] });
      expect(res.status).toBe(404);
    });

    it('returns 405 for an unsupported method on the collection', async () => {
      const res = await call({ method: 'DELETE', segments: ['documents'] });
      expect(res.status).toBe(405);
    });

    it('serves a health check', async () => {
      const res = await call({ method: 'GET', segments: ['health'] });
      expect(res.status).toBe(200);
      expect(res.body).toEqual({ status: 'ok' });
    });
  });
});

describe('documents HTTP API (node:http transport)', () => {
  let server: Server;
  let baseUrl: string;

  beforeEach(async () => {
    const db = createMemoryDatabase().db;
    await migrate(db);
    server = createServer(db);
    await new Promise<void>((resolve) => server.listen(0, '127.0.0.1', resolve));
    const { port } = server.address() as AddressInfo;
    baseUrl = `http://127.0.0.1:${port}`;
  });

  afterEach(async () => {
    await new Promise<void>((resolve, reject) =>
      server.close((err) => (err ? reject(err) : resolve())),
    );
  });

  it('round-trips create -> fetch over HTTP', async () => {
    const createRes = await fetch(`${baseUrl}/documents`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ title: 'Over The Wire', bodyMarkdown: '# Wire' }),
    });
    expect(createRes.status).toBe(201);
    const created = (await createRes.json()) as DocumentBody;
    expect(created.slug).toBe('over-the-wire');

    const getRes = await fetch(`${baseUrl}/documents/over-the-wire`);
    expect(getRes.status).toBe(200);
    expect(((await getRes.json()) as DocumentBody).title).toBe('Over The Wire');
  });

  it('returns 400 for malformed JSON', async () => {
    const res = await fetch(`${baseUrl}/documents`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: '{ not json',
    });
    expect(res.status).toBe(400);
  });

  it('returns 404 with a JSON error body for a missing document', async () => {
    const res = await fetch(`${baseUrl}/documents/ghost`);
    expect(res.status).toBe(404);
    const payload = (await res.json()) as { error: { message: string } };
    expect(payload.error.message).toBeTypeOf('string');
  });
});
