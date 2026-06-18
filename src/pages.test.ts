/**
 * Integration tests for the public HTML frontend.
 *
 * Mirrors `src/api.test.ts`: the page handler is exercised directly against the
 * real data-access layer (pg-mem with migrations applied), and a separate suite
 * binds a `node:http` server to prove the transport adapter routes page paths
 * (everything outside the `/documents` and `/health` API prefixes) to the HTML
 * frontend and serves them with the right content type.
 */

import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import type { AddressInfo } from 'node:net';
import type { Server } from 'node:http';

import { handlePageRequest } from './pages.js';
import { createServer } from './server.js';
import { handleApiRequest } from './api.js';
import { migrate } from './db/migrate.js';
import { createMemoryDatabase } from './db/test-helpers.js';
import type { Queryable } from './db/pool.js';

/** Seed a document through the real API create path (renders + persists HTML). */
async function seed(
  db: Queryable,
  body: { title: string; bodyMarkdown: string; slug?: string },
): Promise<void> {
  const res = await handleApiRequest(db, { method: 'POST', segments: ['documents'], body });
  expect(res.status).toBe(201);
}

describe('public pages (handler)', () => {
  let db: Queryable;

  beforeEach(async () => {
    db = createMemoryDatabase().db;
    await migrate(db);
  });

  describe('GET / (index)', () => {
    it('renders an empty-state index when there are no documents', async () => {
      const res = await handlePageRequest(db, { method: 'GET', segments: [] });
      expect(res.status).toBe(200);
      expect(res.html).toContain('<!doctype html>');
      expect(res.html).toContain('No documents published yet.');
    });

    it('lists published documents with links to their slugs', async () => {
      await seed(db, { title: 'First Post', bodyMarkdown: '# Hello' });
      await seed(db, { title: 'Second Post', bodyMarkdown: '# World' });

      const res = await handlePageRequest(db, { method: 'GET', segments: [] });
      expect(res.status).toBe(200);
      expect(res.html).toContain('href="/first-post"');
      expect(res.html).toContain('First Post');
      expect(res.html).toContain('href="/second-post"');
      expect(res.html).toContain('Second Post');
    });
  });

  describe('GET /:slug (document page)', () => {
    it('renders a documents sanitized HTML inside a styled page', async () => {
      await seed(db, { title: 'Hello World', bodyMarkdown: '# Hi\n\nSome **bold** text.' });

      const res = await handlePageRequest(db, { method: 'GET', segments: ['hello-world'] });
      expect(res.status).toBe(200);
      // Page shell + styling present.
      expect(res.html).toContain('<!doctype html>');
      expect(res.html).toContain('<style>');
      expect(res.html).toContain('<title>Hello World</title>');
      // The document's rendered HTML is embedded.
      expect(res.html).toContain('<h1>Hi</h1>');
      expect(res.html).toContain('<strong>bold</strong>');
    });

    it('escapes the title where it is text but embeds the sanitized body verbatim', async () => {
      // Title with HTML-significant characters must be escaped where it is text.
      await seed(db, {
        title: 'A < B & "quotes"',
        slug: 'edgy-title',
        bodyMarkdown: 'Plain body.',
      });

      const res = await handlePageRequest(db, { method: 'GET', segments: ['edgy-title'] });
      expect(res.status).toBe(200);
      expect(res.html).toContain('A &lt; B &amp; &quot;quotes&quot;');
      expect(res.html).not.toContain('A < B & "quotes"');
    });

    it('does not emit script even if the source Markdown contained one', async () => {
      // The rendering pipeline strips <script>; the page must not reintroduce it.
      await seed(db, {
        title: 'XSS Attempt',
        slug: 'xss',
        bodyMarkdown: 'Hi <script>alert(1)</script> there',
      });

      const res = await handlePageRequest(db, { method: 'GET', segments: ['xss'] });
      expect(res.status).toBe(200);
      expect(res.html).not.toContain('<script>alert(1)</script>');
    });

    it('returns a 404 page for an unknown slug', async () => {
      const res = await handlePageRequest(db, { method: 'GET', segments: ['ghost'] });
      expect(res.status).toBe(404);
      expect(res.html).toContain('Not found');
    });
  });

  describe('method and path handling', () => {
    it('returns 405 for non-GET methods', async () => {
      const res = await handlePageRequest(db, { method: 'POST', segments: [] });
      expect(res.status).toBe(405);
    });

    it('returns a 404 page for a path too deep to be a page', async () => {
      const res = await handlePageRequest(db, { method: 'GET', segments: ['a', 'b'] });
      expect(res.status).toBe(404);
    });
  });
});

describe('public pages (node:http transport)', () => {
  let server: Server;
  let baseUrl: string;
  let db: Queryable;

  beforeEach(async () => {
    db = createMemoryDatabase().db;
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

  it('serves a document page as text/html with embedded rendered HTML', async () => {
    await seed(db, { title: 'Over The Wire', bodyMarkdown: '# Wire\n\nbody' });

    const res = await fetch(`${baseUrl}/over-the-wire`);
    expect(res.status).toBe(200);
    expect(res.headers.get('content-type')).toContain('text/html');
    const html = await res.text();
    expect(html).toContain('<title>Over The Wire</title>');
    expect(html).toContain('<h1>Wire</h1>');
  });

  it('serves the index at /', async () => {
    await seed(db, { title: 'Indexed', bodyMarkdown: 'hi' });
    const res = await fetch(`${baseUrl}/`);
    expect(res.status).toBe(200);
    expect(res.headers.get('content-type')).toContain('text/html');
    expect(await res.text()).toContain('href="/indexed"');
  });

  it('returns a 404 HTML page for an unknown slug', async () => {
    const res = await fetch(`${baseUrl}/nope`);
    expect(res.status).toBe(404);
    expect(res.headers.get('content-type')).toContain('text/html');
    expect(await res.text()).toContain('Not found');
  });

  it('still serves the JSON API under /documents', async () => {
    await seed(db, { title: 'Api Doc', bodyMarkdown: 'hi' });
    const res = await fetch(`${baseUrl}/documents/api-doc`);
    expect(res.status).toBe(200);
    expect(res.headers.get('content-type')).toContain('application/json');
    expect(((await res.json()) as { slug: string }).slug).toBe('api-doc');
  });

  it('still serves the JSON health check', async () => {
    const res = await fetch(`${baseUrl}/health`);
    expect(res.status).toBe(200);
    expect(res.headers.get('content-type')).toContain('application/json');
  });
});
