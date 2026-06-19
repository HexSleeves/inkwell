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

const API_KEY = 'test-secret-key';

describe('documents HTTP API (handler)', () => {
  let db: Queryable;

  beforeEach(async () => {
    db = createMemoryDatabase().db;
    await migrate(db);
  });

  // The configured shared secret is supplied on every handler call, and the
  // valid key is presented by default so the CRUD suites exercise authorized
  // requests. The dedicated auth suite overrides the header to test rejection.
  const call = (req: ApiRequest): Promise<ApiResponse> =>
    handleApiRequest(
      db,
      { headers: { 'x-api-key': API_KEY }, ...req },
      { apiKey: API_KEY },
    );

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

  describe('API key authentication', () => {
    const auth = (req: ApiRequest): Promise<ApiResponse> =>
      handleApiRequest(db, req, { apiKey: API_KEY });

    const sample = { title: 'Auth Doc', bodyMarkdown: '# Auth' };

    it('rejects POST with a missing key (401)', async () => {
      const res = await auth({ method: 'POST', segments: ['documents'], body: sample });
      expect(res.status).toBe(401);
      expect((res.body as { error: { message: string } }).error.message).toMatch(/api key/i);
    });

    it('rejects POST with a wrong key (401)', async () => {
      const res = await auth({
        method: 'POST',
        segments: ['documents'],
        body: sample,
        headers: { 'x-api-key': 'wrong' },
      });
      expect(res.status).toBe(401);
    });

    it('accepts POST with the correct key (201)', async () => {
      const res = await auth({
        method: 'POST',
        segments: ['documents'],
        body: sample,
        headers: { 'x-api-key': API_KEY },
      });
      expect(res.status).toBe(201);
    });

    it('rejects PATCH without a key even when the document exists (401)', async () => {
      await createSample();
      const res = await auth({
        method: 'PATCH',
        segments: ['documents', 'hello-world'],
        body: { title: 'Nope' },
      });
      expect(res.status).toBe(401);
    });

    it('rejects DELETE without a key (401)', async () => {
      await createSample();
      const res = await auth({ method: 'DELETE', segments: ['documents', 'hello-world'] });
      expect(res.status).toBe(401);
    });

    it('leaves GET (list and item) open without a key', async () => {
      await createSample();
      const list = await auth({ method: 'GET', segments: ['documents'] });
      expect(list.status).toBe(200);
      const item = await auth({ method: 'GET', segments: ['documents', 'hello-world'] });
      expect(item.status).toBe(200);
    });

    it('rejects an array-valued x-api-key header (401)', async () => {
      const res = await auth({
        method: 'POST',
        segments: ['documents'],
        body: sample,
        headers: { 'x-api-key': [API_KEY, API_KEY] },
      });
      expect(res.status).toBe(401);
    });

    it('rejects all mutations when no server key is configured (401)', async () => {
      const res = await handleApiRequest(
        db,
        { method: 'POST', segments: ['documents'], body: sample, headers: { 'x-api-key': '' } },
        {},
      );
      expect(res.status).toBe(401);
    });
  });
});

describe('documents HTTP API (node:http transport)', () => {
  let server: Server;
  let baseUrl: string;
  let previousApiKey: string | undefined;

  beforeEach(async () => {
    // The transport adapter reads the secret from the environment.
    previousApiKey = process.env.INKWELL_API_KEY;
    process.env.INKWELL_API_KEY = API_KEY;
    const db = createMemoryDatabase().db;
    await migrate(db);
    server = createServer(db);
    await new Promise<void>((resolve) => server.listen(0, '127.0.0.1', resolve));
    const { port } = server.address() as AddressInfo;
    baseUrl = `http://127.0.0.1:${port}`;
  });

  afterEach(async () => {
    if (previousApiKey === undefined) {
      delete process.env.INKWELL_API_KEY;
    } else {
      process.env.INKWELL_API_KEY = previousApiKey;
    }
    await new Promise<void>((resolve, reject) =>
      server.close((err) => (err ? reject(err) : resolve())),
    );
  });

  it('round-trips create -> fetch over HTTP', async () => {
    const createRes = await fetch(`${baseUrl}/documents`, {
      method: 'POST',
      headers: { 'content-type': 'application/json', 'x-api-key': API_KEY },
      body: JSON.stringify({ title: 'Over The Wire', bodyMarkdown: '# Wire' }),
    });
    expect(createRes.status).toBe(201);
    const created = (await createRes.json()) as DocumentBody;
    expect(created.slug).toBe('over-the-wire');

    const getRes = await fetch(`${baseUrl}/documents/over-the-wire`);
    expect(getRes.status).toBe(200);
    expect(((await getRes.json()) as DocumentBody).title).toBe('Over The Wire');
  });

  it('rejects an unauthenticated POST over HTTP with 401', async () => {
    const res = await fetch(`${baseUrl}/documents`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ title: 'No Key', bodyMarkdown: '# Nope' }),
    });
    expect(res.status).toBe(401);
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
