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
  status: 'draft' | 'published';
  tags: string[];
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
    handleApiRequest(db, { headers: { 'x-api-key': API_KEY }, ...req }, { apiKey: API_KEY });

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

    // DoS guard: rendering is synchronous, so bodyMarkdown is capped at 256 KiB
    // (262_144 chars) at the API edge. Exercise both sides of the boundary.
    const MAX_BODY_MARKDOWN_LENGTH = 256 * 1024;

    it('accepts bodyMarkdown exactly at the cap', async () => {
      const res = await createSample({ bodyMarkdown: 'a'.repeat(MAX_BODY_MARKDOWN_LENGTH) });
      expect(res.status).toBe(201);
    });

    it('rejects bodyMarkdown one char over the cap with 400', async () => {
      const res = await createSample({ bodyMarkdown: 'a'.repeat(MAX_BODY_MARKDOWN_LENGTH + 1) });
      expect(res.status).toBe(400);
      expect((res.body as { error: { message: string } }).error.message).toMatch(/at most/i);
    });
  });

  interface ListBody {
    documents: DocumentBody[];
    total: number;
    limit: number;
    offset: number;
  }

  describe('tags', () => {
    it('accepts, normalizes, and returns tags on create', async () => {
      const res = await createSample({ slug: 'tagged', tags: ['Rust', 'rust', ' Postgres '] });
      expect(res.status).toBe(201);
      // Lower-cased, trimmed, de-duplicated, order preserved.
      expect((res.body as DocumentBody).tags).toEqual(['rust', 'postgres']);
    });

    it('defaults tags to an empty array when omitted', async () => {
      const res = await createSample({ slug: 'untagged' });
      expect((res.body as DocumentBody).tags).toEqual([]);
    });

    it('rejects malformed tags with 400', async () => {
      const bad = await createSample({ slug: 'x', tags: ['has space'] });
      expect(bad.status).toBe(400);
      const notArray = await createSample({ slug: 'y', tags: 'rust' });
      expect(notArray.status).toBe(400);
      const tooMany = await createSample({
        slug: 'z',
        tags: Array.from({ length: 21 }, (_, i) => `t${i}`),
      });
      expect(tooMany.status).toBe(400);
    });

    it('replaces tags on PATCH and leaves them untouched when omitted', async () => {
      await createSample({ slug: 'doc', tags: ['a', 'b'] });
      const patched = await call({
        method: 'PATCH',
        segments: ['documents', 'doc'],
        body: { tags: ['c'] },
      });
      expect(patched.status).toBe(200);
      expect((patched.body as DocumentBody).tags).toEqual(['c']);

      const titleOnly = await call({
        method: 'PATCH',
        segments: ['documents', 'doc'],
        body: { title: 'Renamed' },
      });
      expect((titleOnly.body as DocumentBody).tags).toEqual(['c']);
    });
  });

  describe('GET /documents', () => {
    it('lists documents in a paginated envelope', async () => {
      await createSample({ slug: 'a' });
      await createSample({ slug: 'b' });
      const res = await call({ method: 'GET', segments: ['documents'] });
      expect(res.status).toBe(200);
      const page = res.body as ListBody;
      expect(page.documents.map((d) => d.slug).sort()).toEqual(['a', 'b']);
      expect(page.total).toBe(2);
      expect(page.limit).toBe(20);
      expect(page.offset).toBe(0);
    });

    it('returns an empty page when there are no documents', async () => {
      const res = await call({ method: 'GET', segments: ['documents'] });
      expect(res.status).toBe(200);
      expect(res.body).toEqual({ documents: [], total: 0, limit: 20, offset: 0 });
    });

    describe('pagination', () => {
      // Seed N documents; created_at ordering is newest-first, so the most
      // recently inserted slug lands at index 0 of the unpaged list.
      const seed = async (count: number): Promise<void> => {
        for (let i = 0; i < count; i += 1) {
          // Pad so lexical and insertion order agree for readable assertions.
          const res = await createSample({ slug: `doc-${String(i).padStart(2, '0')}` });
          expect(res.status).toBe(201);
        }
      };

      it('applies a custom limit and reports the full total', async () => {
        await seed(5);
        const res = await call({ method: 'GET', segments: ['documents'], query: { limit: '2' } });
        expect(res.status).toBe(200);
        const page = res.body as ListBody;
        expect(page.documents).toHaveLength(2);
        expect(page.total).toBe(5);
        expect(page.limit).toBe(2);
        expect(page.offset).toBe(0);
      });

      it('walks pages with limit + offset without overlap', async () => {
        await seed(5);
        const first = (
          await call({ method: 'GET', segments: ['documents'], query: { limit: '2', offset: '0' } })
        ).body as ListBody;
        const second = (
          await call({ method: 'GET', segments: ['documents'], query: { limit: '2', offset: '2' } })
        ).body as ListBody;
        const firstSlugs = first.documents.map((d) => d.slug);
        const secondSlugs = second.documents.map((d) => d.slug);
        expect(firstSlugs).toHaveLength(2);
        expect(secondSlugs).toHaveLength(2);
        // Disjoint pages, and each reports the same unpaged total.
        expect(firstSlugs.some((s) => secondSlugs.includes(s))).toBe(false);
        expect(first.total).toBe(5);
        expect(second.total).toBe(5);
      });

      it('returns an empty page past the end while keeping the total', async () => {
        await seed(3);
        const res = await call({
          method: 'GET',
          segments: ['documents'],
          query: { offset: '10' },
        });
        const page = res.body as ListBody;
        expect(page.documents).toEqual([]);
        expect(page.total).toBe(3);
        expect(page.offset).toBe(10);
      });

      it('clamps a limit above the max to 100', async () => {
        const res = await call({
          method: 'GET',
          segments: ['documents'],
          query: { limit: '500' },
        });
        expect(res.status).toBe(200);
        expect((res.body as ListBody).limit).toBe(100);
      });

      it('rejects a non-numeric limit with 400', async () => {
        const res = await call({
          method: 'GET',
          segments: ['documents'],
          query: { limit: 'abc' },
        });
        expect(res.status).toBe(400);
      });

      it('rejects a zero limit with 400', async () => {
        const res = await call({ method: 'GET', segments: ['documents'], query: { limit: '0' } });
        expect(res.status).toBe(400);
      });

      it('rejects a negative offset with 400', async () => {
        const res = await call({
          method: 'GET',
          segments: ['documents'],
          query: { offset: '-1' },
        });
        expect(res.status).toBe(400);
      });
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

    it('rejects a bodyMarkdown patch over the 256 KiB cap with 400', async () => {
      await createSample();
      const res = await call({
        method: 'PATCH',
        segments: ['documents', 'hello-world'],
        body: { bodyMarkdown: 'a'.repeat(256 * 1024 + 1) },
      });
      expect(res.status).toBe(400);
      expect((res.body as { error: { message: string } }).error.message).toMatch(/at most/i);
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

  describe('draft/published state', () => {
    // Helpers scoped to this suite. `call` (authed) is used to create/transition;
    // `pub`/`unpub` hit the action routes; reads use either `call` (authed) or a
    // bare unauthenticated request to assert public gating.
    const pub = (slug: string): Promise<ApiResponse> =>
      call({ method: 'POST', segments: ['documents', slug, 'publish'] });
    const unpub = (slug: string): Promise<ApiResponse> =>
      call({ method: 'POST', segments: ['documents', slug, 'unpublish'] });
    const anon = (req: ApiRequest): Promise<ApiResponse> =>
      handleApiRequest(db, req, { apiKey: API_KEY });

    it('defaults new documents to draft', async () => {
      const res = await createSample();
      expect(res.status).toBe(201);
      expect((res.body as DocumentBody).status).toBe('draft');
    });

    it('publish then unpublish flips status and is idempotent', async () => {
      await createSample();
      const published = await pub('hello-world');
      expect(published.status).toBe(200);
      expect((published.body as DocumentBody).status).toBe('published');

      // Idempotent: publishing again stays published, still 200.
      const again = await pub('hello-world');
      expect(again.status).toBe(200);
      expect((again.body as DocumentBody).status).toBe('published');

      const draft = await unpub('hello-world');
      expect(draft.status).toBe(200);
      expect((draft.body as DocumentBody).status).toBe('draft');

      const againDraft = await unpub('hello-world');
      expect(againDraft.status).toBe(200);
      expect((againDraft.body as DocumentBody).status).toBe('draft');
    });

    it('publish/unpublish on a missing document is 404', async () => {
      expect((await pub('ghost')).status).toBe(404);
      expect((await unpub('ghost')).status).toBe(404);
    });

    it('publish/unpublish require an API key (401 without one)', async () => {
      await createSample();
      const noKeyPub = await anon({
        method: 'POST',
        segments: ['documents', 'hello-world', 'publish'],
      });
      expect(noKeyPub.status).toBe(401);
      const noKeyUnpub = await anon({
        method: 'POST',
        segments: ['documents', 'hello-world', 'unpublish'],
      });
      expect(noKeyUnpub.status).toBe(401);
      // The status was not changed by the rejected calls.
      expect(
        (await call({ method: 'GET', segments: ['documents', 'hello-world'] })).body,
      ).toMatchObject({ status: 'draft' });
    });

    it('rejects a non-POST method on the action route (405)', async () => {
      await createSample();
      const res = await call({ method: 'GET', segments: ['documents', 'hello-world', 'publish'] });
      expect(res.status).toBe(405);
    });

    it('public list returns only published docs', async () => {
      await createSample({ slug: 'draft-one' });
      await createSample({ slug: 'pub-one' });
      await pub('pub-one');

      const res = await anon({ method: 'GET', segments: ['documents'] });
      expect(res.status).toBe(200);
      expect((res.body as { documents: DocumentBody[] }).documents.map((d) => d.slug)).toEqual([
        'pub-one',
      ]);
    });

    it('public single GET 404s a draft but serves a published doc', async () => {
      await createSample();
      const draftGet = await anon({ method: 'GET', segments: ['documents', 'hello-world'] });
      expect(draftGet.status).toBe(404);

      await pub('hello-world');
      const pubGet = await anon({ method: 'GET', segments: ['documents', 'hello-world'] });
      expect(pubGet.status).toBe(200);
      expect((pubGet.body as DocumentBody).slug).toBe('hello-world');
    });

    it('authenticated list includes drafts by default', async () => {
      await createSample({ slug: 'draft-one' });
      await createSample({ slug: 'pub-one' });
      await pub('pub-one');

      const res = await call({ method: 'GET', segments: ['documents'] });
      expect(res.status).toBe(200);
      expect(
        (res.body as { documents: DocumentBody[] }).documents.map((d) => d.slug).sort(),
      ).toEqual(['draft-one', 'pub-one']);
    });

    it('authenticated list filters by ?status', async () => {
      await createSample({ slug: 'draft-one' });
      await createSample({ slug: 'pub-one' });
      await pub('pub-one');

      const drafts = await call({
        method: 'GET',
        segments: ['documents'],
        query: { status: 'draft' },
      });
      expect((drafts.body as { documents: DocumentBody[] }).documents.map((d) => d.slug)).toEqual([
        'draft-one',
      ]);

      const published = await call({
        method: 'GET',
        segments: ['documents'],
        query: { status: 'published' },
      });
      expect(
        (published.body as { documents: DocumentBody[] }).documents.map((d) => d.slug),
      ).toEqual(['pub-one']);

      const all = await call({
        method: 'GET',
        segments: ['documents'],
        query: { status: 'all' },
      });
      expect(
        (all.body as { documents: DocumentBody[] }).documents.map((d) => d.slug).sort(),
      ).toEqual(['draft-one', 'pub-one']);
    });

    it('rejects an unknown ?status value with 400 (authenticated)', async () => {
      const res = await call({
        method: 'GET',
        segments: ['documents'],
        query: { status: 'archived' },
      });
      expect(res.status).toBe(400);
    });

    it('ignores ?status for unauthenticated callers (drafts never leak)', async () => {
      await createSample({ slug: 'draft-one' });
      const res = await anon({
        method: 'GET',
        segments: ['documents'],
        query: { status: 'draft' },
      });
      expect(res.status).toBe(200);
      expect((res.body as { documents: DocumentBody[] }).documents).toEqual([]);
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

    it('serves a health check that reports the database as up', async () => {
      const res = await call({ method: 'GET', segments: ['health'] });
      expect(res.status).toBe(200);
      expect(res.body).toEqual({ status: 'ok', db: 'up' });
    });

    it('reports 503 when the database is unreachable', async () => {
      const brokenDb: Queryable = {
        query: () => Promise.reject(new Error('connection refused')),
      };
      const res = await handleApiRequest(
        brokenDb,
        { method: 'GET', segments: ['health'], headers: { 'x-api-key': API_KEY } },
        { apiKey: API_KEY },
      );
      expect(res.status).toBe(503);
      expect(res.body).toEqual({ status: 'error', db: 'down' });
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

    it('leaves GET (list and item) open without a key for published docs', async () => {
      await createSample();
      // Created docs are drafts; publish so an unauthenticated reader can see it.
      await call({ method: 'POST', segments: ['documents', 'hello-world', 'publish'] });
      const list = await auth({ method: 'GET', segments: ['documents'] });
      expect(list.status).toBe(200);
      expect((list.body as { documents: DocumentBody[] }).documents.map((d) => d.slug)).toEqual([
        'hello-world',
      ]);
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
    expect(created.status).toBe('draft');

    // A draft is hidden from unauthenticated readers (404, no existence leak).
    const draftGet = await fetch(`${baseUrl}/documents/over-the-wire`);
    expect(draftGet.status).toBe(404);

    // Publish, then the same unauthenticated fetch succeeds.
    const pubRes = await fetch(`${baseUrl}/documents/over-the-wire/publish`, {
      method: 'POST',
      headers: { 'x-api-key': API_KEY },
    });
    expect(pubRes.status).toBe(200);
    expect(((await pubRes.json()) as DocumentBody).status).toBe('published');

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
