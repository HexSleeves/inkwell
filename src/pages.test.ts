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

import { PAGE_SIZE, deriveExcerpt, handlePageRequest } from './pages.js';
import { createServer } from './server.js';
import { handleApiRequest } from './api.js';
import { migrate } from './db/migrate.js';
import { createMemoryDatabase } from './db/test-helpers.js';
import type { Queryable } from './db/pool.js';

/** Shared secret used to authorize the seed writes below. */
const SEED_API_KEY = 'pages-test-key';

/** Create a document through the real API path (renders + persists HTML). */
async function create(
  db: Queryable,
  body: { title: string; bodyMarkdown: string; slug?: string },
): Promise<string> {
  const res = await handleApiRequest(
    db,
    { method: 'POST', segments: ['documents'], body, headers: { 'x-api-key': SEED_API_KEY } },
    { apiKey: SEED_API_KEY },
  );
  expect(res.status).toBe(201);
  return (res.body as { slug: string }).slug;
}

/**
 * Seed a *published* document. Documents are created as drafts (which the public
 * frontend hides), so the public-page tests below publish their fixtures.
 */
async function seed(
  db: Queryable,
  body: { title: string; bodyMarkdown: string; slug?: string },
): Promise<void> {
  const slug = await create(db, body);
  const res = await handleApiRequest(
    db,
    {
      method: 'POST',
      segments: ['documents', slug, 'publish'],
      headers: { 'x-api-key': SEED_API_KEY },
    },
    { apiKey: SEED_API_KEY },
  );
  expect(res.status).toBe(200);
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
      expect(res.html).toContain('<title>Hello World — Inkwell</title>');
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

    it('hides a draft: not on the index and its page 404s', async () => {
      // Created but never published -> draft.
      await create(db, { title: 'Secret Draft', slug: 'secret-draft', bodyMarkdown: '# wip' });

      const index = await handlePageRequest(db, { method: 'GET', segments: [] });
      expect(index.status).toBe(200);
      expect(index.html).not.toContain('secret-draft');
      expect(index.html).not.toContain('Secret Draft');

      const page = await handlePageRequest(db, { method: 'GET', segments: ['secret-draft'] });
      expect(page.status).toBe(404);
      expect(page.html).toContain('Not found');
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

describe('deriveExcerpt', () => {
  it('returns empty string for empty/whitespace-only input', () => {
    expect(deriveExcerpt('')).toBe('');
    expect(deriveExcerpt('   \n  ')).toBe('');
  });

  it('strips common Markdown syntax to plain text', () => {
    const md = '# Title\n\nSome **bold** and _italic_ and `code` and [a link](https://x).';
    expect(deriveExcerpt(md)).toBe('Title Some bold and italic and code and a link.');
  });

  it('drops fenced code blocks and images', () => {
    const md = 'Intro text.\n\n```ts\nconst x = 1;\n```\n\n![alt](img.png) tail.';
    expect(deriveExcerpt(md)).toBe('Intro text. tail.');
  });

  it('truncates on a word boundary and appends an ellipsis', () => {
    const md = 'alpha bravo charlie delta echo foxtrot golf';
    const out = deriveExcerpt(md, 20);
    expect(out.endsWith('…')).toBe(true);
    // Trimmed at a space within the 20-char budget, no partial word.
    expect(out.length).toBeLessThanOrEqual(21);
    expect(out).toBe('alpha bravo charlie…');
  });
});

describe('SEO metadata', () => {
  let db: Queryable;

  beforeEach(async () => {
    db = createMemoryDatabase().db;
    await migrate(db);
  });

  it('emits canonical, OpenGraph, Twitter, and JSON-LD on a document page', async () => {
    await seed(db, { title: 'Hello World', bodyMarkdown: 'A short body about cats.' });

    const res = await handlePageRequest(
      db,
      { method: 'GET', segments: ['hello-world'] },
      { siteUrl: 'https://blog.example.com' },
    );
    expect(res.status).toBe(200);
    expect(res.html).toContain(
      '<link rel="canonical" href="https://blog.example.com/hello-world" />',
    );
    expect(res.html).toContain('<meta property="og:type" content="article" />');
    expect(res.html).toContain(
      '<meta property="og:url" content="https://blog.example.com/hello-world" />',
    );
    expect(res.html).toContain('<meta name="twitter:card" content="summary" />');
    expect(res.html).toContain('<meta name="description" content="A short body about cats." />');
    // JSON-LD BlogPosting block with the canonical URL.
    expect(res.html).toContain('<script type="application/ld+json">');
    expect(res.html).toContain('"@type":"BlogPosting"');
    expect(res.html).toContain('"headline":"Hello World"');
    expect(res.html).toContain('"url":"https://blog.example.com/hello-world"');
  });

  it('escapes JSON-LD so a title cannot break out of the script element', async () => {
    await seed(db, {
      title: 'Evil </script> title',
      slug: 'evil',
      bodyMarkdown: 'body',
    });
    const res = await handlePageRequest(db, { method: 'GET', segments: ['evil'] });
    // The literal closing tag must not appear inside the JSON-LD payload.
    expect(res.html).not.toContain('Evil </script> title');
    expect(res.html).toContain('\\u003c/script\\u003e');
  });

  it('marks the index as og:type website with a canonical root URL', async () => {
    const res = await handlePageRequest(
      db,
      { method: 'GET', segments: [] },
      { siteUrl: 'https://blog.example.com' },
    );
    expect(res.html).toContain('<meta property="og:type" content="website" />');
    expect(res.html).toContain('<link rel="canonical" href="https://blog.example.com/" />');
    // Discovery: every page advertises the Atom feed.
    expect(res.html).toContain('<link rel="alternate" type="application/atom+xml"');
  });
});

describe('index pagination & excerpts', () => {
  let db: Queryable;

  beforeEach(async () => {
    db = createMemoryDatabase().db;
    await migrate(db);
  });

  it('shows an excerpt under each index entry', async () => {
    await seed(db, { title: 'Cats', bodyMarkdown: '# Cats\n\nAll about cats and how they nap.' });
    const res = await handlePageRequest(db, { method: 'GET', segments: [] });
    expect(res.html).toContain('class="excerpt"');
    expect(res.html).toContain('All about cats and how they nap.');
  });

  it('does not paginate when there is only a single page of documents', async () => {
    await seed(db, { title: 'Only One', bodyMarkdown: 'hi' });
    const res = await handlePageRequest(db, { method: 'GET', segments: [] });
    expect(res.html).not.toContain('class="pager"');
  });

  it('paginates across multiple pages with prev/next links', async () => {
    // One more than a full page so a second page exists.
    for (let i = 0; i < PAGE_SIZE + 1; i++) {
      await seed(db, { title: `Post ${i}`, bodyMarkdown: `Body ${i}`, slug: `post-${i}` });
    }

    const page1 = await handlePageRequest(db, { method: 'GET', segments: [] });
    expect(page1.status).toBe(200);
    // Newest-first: the most recent post (PAGE_SIZE) is on page 1; the oldest is not.
    expect(page1.html).toContain('href="/post-10"');
    expect(page1.html).not.toContain('href="/post-0"');
    expect(page1.html).toContain('class="pager"');
    expect(page1.html).toContain('href="/page/2"');

    const page2 = await handlePageRequest(db, { method: 'GET', segments: ['page', '2'] });
    expect(page2.status).toBe(200);
    // The oldest post falls onto page 2, with a link back to page 1 (root).
    expect(page2.html).toContain('href="/post-0"');
    expect(page2.html).toContain('rel="prev" href="/"');
    expect(page2.html).toContain('<link rel="canonical" href="http://localhost/page/2" />');
  });

  it('404s a page number past the end and a malformed page number', async () => {
    await seed(db, { title: 'Lonely', bodyMarkdown: 'hi' });
    expect((await handlePageRequest(db, { method: 'GET', segments: ['page', '2'] })).status).toBe(
      404,
    );
    expect((await handlePageRequest(db, { method: 'GET', segments: ['page', '0'] })).status).toBe(
      404,
    );
    expect((await handlePageRequest(db, { method: 'GET', segments: ['page', 'x'] })).status).toBe(
      404,
    );
  });

  it('serves page 1 (empty) on a site with no documents', async () => {
    const res = await handlePageRequest(db, { method: 'GET', segments: [] });
    expect(res.status).toBe(200);
    expect(res.html).toContain('No documents published yet.');
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
    expect(html).toContain('<title>Over The Wire — Inkwell</title>');
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
