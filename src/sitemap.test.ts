/**
 * Tests for `sitemap.xml`.
 *
 * `buildSitemap` is unit-tested against synthetic {@link Document} values for
 * structure and escaping. `handleSitemapRequest` is integration-tested against
 * the real data-access layer (pg-mem with migrations applied), and a `node:http`
 * suite proves the transport adapter routes `GET /sitemap.xml` and serves it as
 * `application/xml`. Mirrors `src/feed.test.ts`.
 */

import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import type { AddressInfo } from 'node:net';
import type { Server } from 'node:http';

import { SITEMAP_CONTENT_TYPE, buildSitemap, escapeXml, handleSitemapRequest } from './sitemap.js';
import { handleApiRequest } from './api.js';
import { createServer } from './server.js';
import { migrate } from './db/migrate.js';
import { createMemoryDatabase } from './db/test-helpers.js';
import type { Document } from './db/documents.js';
import type { Queryable } from './db/pool.js';

/** Shared secret used to authorize the seed writes below. */
const SEED_API_KEY = 'sitemap-test-key';

/** Seed a document through the real API path; publish it unless told otherwise. */
async function seed(
  db: Queryable,
  body: { title: string; bodyMarkdown: string; slug?: string; tags?: string[] },
  publish = true,
): Promise<Document> {
  const res = await handleApiRequest(
    db,
    { method: 'POST', segments: ['documents'], body, headers: { 'x-api-key': SEED_API_KEY } },
    { apiKey: SEED_API_KEY },
  );
  expect(res.status).toBe(201);
  const doc = res.body as Document;
  if (publish) {
    await db.query(`UPDATE documents SET status = 'published' WHERE slug = $1`, [doc.slug]);
  }
  return doc;
}

/** Count `<url>` elements in a serialized sitemap. */
function urlCount(xml: string): number {
  return (xml.match(/<url>/g) ?? []).length;
}

/** A synthetic Document for unit tests that don't need a database. */
function makeDoc(overrides: Partial<Document> = {}): Document {
  return {
    id: 'id-1',
    slug: 'hello-world',
    title: 'Hello World',
    bodyMarkdown: '# Hello',
    renderedHtml: '<h1>Hello</h1>',
    status: 'published',
    tags: [],
    createdAt: new Date('2026-01-01T00:00:00.000Z'),
    updatedAt: new Date('2026-01-02T00:00:00.000Z'),
    ...overrides,
  };
}

describe('escapeXml', () => {
  it('escapes the five XML special characters', () => {
    expect(escapeXml(`& < > " '`)).toBe('&amp; &lt; &gt; &quot; &apos;');
  });
});

describe('buildSitemap', () => {
  it('emits a well-formed urlset with only the home page when empty', () => {
    const xml = buildSitemap([], { siteUrl: 'https://blog.example.com' });
    expect(xml.startsWith('<?xml version="1.0" encoding="utf-8"?>')).toBe(true);
    expect(xml).toContain('<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">');
    expect(xml).toContain('<loc>https://blog.example.com/</loc>');
    expect(urlCount(xml)).toBe(1);
    // No documents -> the home entry carries no lastmod.
    expect(xml).not.toContain('<lastmod>');
    expect((xml.match(/<urlset\b/g) ?? []).length).toBe(1);
    expect((xml.match(/<\/urlset>/g) ?? []).length).toBe(1);
  });

  it('lists the home page plus one url per document with lastmod', () => {
    const xml = buildSitemap([makeDoc()], { siteUrl: 'https://blog.example.com' });
    expect(urlCount(xml)).toBe(2);
    expect(xml).toContain('<loc>https://blog.example.com/hello-world</loc>');
    expect(xml).toContain('<lastmod>2026-01-02T00:00:00.000Z</lastmod>');
  });

  it('percent-encodes slugs in locs', () => {
    const xml = buildSitemap([makeDoc({ slug: 'a b&c' })], { siteUrl: 'https://x.example' });
    // encodeURIComponent handles both the space (%20) and the ampersand (%26),
    // so the loc is already URL-safe before XML escaping has anything to do.
    expect(xml).toContain('<loc>https://x.example/a%20b%26c</loc>');
  });

  it('falls back to the default origin when no siteUrl is configured', () => {
    const xml = buildSitemap([makeDoc()]);
    expect(xml).toContain('<loc>http://localhost/hello-world</loc>');
  });

  it('tolerates and trims a trailing slash on the configured siteUrl', () => {
    const xml = buildSitemap([makeDoc()], { siteUrl: 'https://blog.example.com/' });
    expect(xml).toContain('<loc>https://blog.example.com/</loc>');
    expect(xml).toContain('<loc>https://blog.example.com/hello-world</loc>');
  });
});

describe('sitemap handler', () => {
  let db: Queryable;

  beforeEach(async () => {
    db = createMemoryDatabase().db;
    await migrate(db);
  });

  it('serves a home-only sitemap when there are no documents', async () => {
    const res = await handleSitemapRequest(db, { method: 'GET' });
    expect(res.status).toBe(200);
    expect(res.contentType).toBe(SITEMAP_CONTENT_TYPE);
    expect(urlCount(res.xml)).toBe(1);
  });

  it('lists published documents and excludes drafts', async () => {
    await seed(db, { title: 'Published One', bodyMarkdown: 'live' }, true);
    await seed(db, { title: 'Secret Draft', bodyMarkdown: 'wip' }, false);

    const res = await handleSitemapRequest(db, { method: 'GET' }, { siteUrl: 'https://x.example' });
    // Home + one published document.
    expect(urlCount(res.xml)).toBe(2);
    expect(res.xml).toContain('<loc>https://x.example/published-one</loc>');
    expect(res.xml).not.toContain('secret-draft');
  });

  it('rejects non-GET methods with 405 and no body', async () => {
    const res = await handleSitemapRequest(db, { method: 'POST' });
    expect(res.status).toBe(405);
    expect(res.xml).toBe('');
  });
});

describe('sitemap (node:http transport)', () => {
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

  it('serves /sitemap.xml as application/xml with a urlset', async () => {
    await seed(db, { title: 'Wired Post', bodyMarkdown: '# Wire\n\nbody' });

    const res = await fetch(`${baseUrl}/sitemap.xml`);
    expect(res.status).toBe(200);
    expect(res.headers.get('content-type')).toContain('application/xml');
    const xml = await res.text();
    expect(xml).toContain('<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">');
    expect(xml).toContain('/wired-post');
  });

  it('does not let /sitemap.xml fall through to the HTML 404 page', async () => {
    const res = await fetch(`${baseUrl}/sitemap.xml`);
    expect(res.status).toBe(200);
    expect(res.headers.get('content-type')).not.toContain('text/html');
  });
});

describe('sitemap tag URLs', () => {
  it('emits the /tags index and one URL per published tag', () => {
    const xml = buildSitemap([], { siteUrl: 'https://blog.test' }, [
      { tag: 'rust', count: 2 },
      { tag: 'sql', count: 1 },
    ]);
    expect(xml).toContain('<loc>https://blog.test/tags</loc>');
    expect(xml).toContain('<loc>https://blog.test/tags/rust</loc>');
    expect(xml).toContain('<loc>https://blog.test/tags/sql</loc>');
  });

  it('omits tag URLs entirely when there are none', () => {
    const xml = buildSitemap([], { siteUrl: 'https://blog.test' }, []);
    expect(xml).not.toContain('/tags');
  });

  it('includes published tags from the database', async () => {
    const db = createMemoryDatabase().db;
    await migrate(db);
    await seed(db, { title: 'Tagged', bodyMarkdown: 'x', tags: ['rust'] });
    const res = await handleSitemapRequest(db, { method: 'GET' }, { siteUrl: 'https://blog.test' });
    expect(res.xml).toContain('https://blog.test/tags/rust');
  });
});
