/**
 * Tests for the Atom 1.0 syndication feed.
 *
 * `buildAtomFeed` is unit-tested against synthetic {@link Document} values for
 * structure and escaping. `handleFeedRequest` is integration-tested against the
 * real data-access layer (pg-mem with migrations applied), and a `node:http`
 * suite proves the transport adapter routes `GET /feed.xml` to the feed and
 * serves it as `application/atom+xml`. Mirrors `src/api.test.ts` / `src/pages.test.ts`.
 */

import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import type { AddressInfo } from 'node:net';
import type { Server } from 'node:http';

import {
  ATOM_CONTENT_TYPE,
  FEED_MAX_ENTRIES,
  buildAtomFeed,
  escapeXml,
  handleFeedRequest,
} from './feed.js';
import { handleApiRequest } from './api.js';
import { createServer } from './server.js';
import { migrate } from './db/migrate.js';
import { createMemoryDatabase } from './db/test-helpers.js';
import type { Document } from './db/documents.js';
import type { Queryable } from './db/pool.js';

/** Shared secret used to authorize the seed writes below. */
const SEED_API_KEY = 'feed-test-key';

/**
 * Seed a document through the real API create path (renders + persists HTML).
 * Documents are created as `draft` by default; the optional `publish` flag flips
 * the row to `published` directly in the database so feed tests can control
 * which documents are publicly visible.
 */
async function seed(
  db: Queryable,
  body: { title: string; bodyMarkdown: string; slug?: string },
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

/** Count `<entry>` elements in a serialized feed. */
function entryCount(xml: string): number {
  return (xml.match(/<entry>/g) ?? []).length;
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

  it('escapes ampersands before the entities it introduces (no double-escaping)', () => {
    expect(escapeXml('<a>')).toBe('&lt;a&gt;');
    expect(escapeXml('a & b')).toBe('a &amp; b');
  });
});

describe('buildAtomFeed', () => {
  it('emits a well-formed empty feed with the required feed-level elements', () => {
    const xml = buildAtomFeed([], { siteUrl: 'https://blog.example.com' });
    expect(xml.startsWith('<?xml version="1.0" encoding="utf-8"?>')).toBe(true);
    expect(xml).toContain('<feed xmlns="http://www.w3.org/2005/Atom">');
    expect(xml).toContain('<title>Inkwell</title>');
    expect(xml).toContain('<id>https://blog.example.com/</id>');
    expect(xml).toContain(
      '<link rel="self" type="application/atom+xml" href="https://blog.example.com/feed.xml" />',
    );
    // A valid Atom feed must carry an <updated>; empty feed falls back to epoch.
    expect(xml).toContain('<updated>1970-01-01T00:00:00.000Z</updated>');
    expect(entryCount(xml)).toBe(0);
    // Single root: exactly one opening and closing <feed>.
    expect((xml.match(/<feed\b/g) ?? []).length).toBe(1);
    expect((xml.match(/<\/feed>/g) ?? []).length).toBe(1);
  });

  it('renders one entry per document with title, link, dates, and content', () => {
    const xml = buildAtomFeed([makeDoc()], { siteUrl: 'https://blog.example.com' });
    expect(entryCount(xml)).toBe(1);
    expect(xml).toContain('<title>Hello World</title>');
    expect(xml).toContain('<id>https://blog.example.com/hello-world</id>');
    expect(xml).toContain(
      '<link rel="alternate" type="text/html" href="https://blog.example.com/hello-world" />',
    );
    // RFC3339 published/updated derived from the document timestamps.
    expect(xml).toContain('<published>2026-01-01T00:00:00.000Z</published>');
    expect(xml).toContain('<updated>2026-01-02T00:00:00.000Z</updated>');
  });

  it('entity-escapes the rendered HTML so it is content text, not live markup', () => {
    const xml = buildAtomFeed([makeDoc({ renderedHtml: '<h1>Hi</h1><p>x &amp; y</p>' })]);
    expect(xml).toContain('<content type="html">&lt;h1&gt;Hi&lt;/h1&gt;');
    // The body's own markup must not appear as real child elements of <content>.
    expect(xml).not.toContain('<content type="html"><h1>');
  });

  it('escapes XML special characters in interpolated titles', () => {
    const xml = buildAtomFeed([
      makeDoc({ title: 'A < B & "Q"', slug: 'a-b', renderedHtml: '<p>ok</p>' }),
    ]);
    expect(xml).toContain('<title>A &lt; B &amp; &quot;Q&quot;</title>');
    expect(xml).not.toContain('<title>A < B & "Q"</title>');
  });

  it('falls back to the default origin when no siteUrl is configured', () => {
    const xml = buildAtomFeed([makeDoc()]);
    expect(xml).toContain('<id>http://localhost/hello-world</id>');
  });

  it('tolerates and trims a trailing slash on the configured siteUrl', () => {
    const xml = buildAtomFeed([makeDoc()], { siteUrl: 'https://blog.example.com/' });
    expect(xml).toContain('<id>https://blog.example.com/</id>');
    expect(xml).toContain('<id>https://blog.example.com/hello-world</id>');
  });
});

describe('feed handler', () => {
  let db: Queryable;

  beforeEach(async () => {
    db = createMemoryDatabase().db;
    await migrate(db);
  });

  it('serves an empty feed when there are no documents', async () => {
    const res = await handleFeedRequest(db, { method: 'GET' });
    expect(res.status).toBe(200);
    expect(res.contentType).toBe(ATOM_CONTENT_TYPE);
    expect(entryCount(res.xml)).toBe(0);
  });

  it('serves recent documents newest-first with one entry each', async () => {
    await seed(db, { title: 'First Post', bodyMarkdown: '# Hello' });
    await seed(db, { title: 'Second Post', bodyMarkdown: '# World' });

    const res = await handleFeedRequest(db, { method: 'GET' }, { siteUrl: 'https://x.example' });
    expect(res.status).toBe(200);
    expect(entryCount(res.xml)).toBe(2);
    expect(res.xml).toContain('<title>First Post</title>');
    expect(res.xml).toContain('<title>Second Post</title>');
    expect(res.xml).toContain('href="https://x.example/first-post"');
    // Newest first: Second Post was created last, so it precedes First Post.
    expect(res.xml.indexOf('Second Post')).toBeLessThan(res.xml.indexOf('First Post'));
  });

  it('excludes draft documents, listing only published ones', async () => {
    await seed(db, { title: 'Published One', bodyMarkdown: 'live' }, true);
    await seed(db, { title: 'Secret Draft', bodyMarkdown: 'wip' }, false);

    const res = await handleFeedRequest(db, { method: 'GET' });
    expect(entryCount(res.xml)).toBe(1);
    expect(res.xml).toContain('<title>Published One</title>');
    expect(res.xml).not.toContain('Secret Draft');
  });

  it(`caps the feed at ${FEED_MAX_ENTRIES} entries`, async () => {
    for (let i = 0; i < FEED_MAX_ENTRIES + 5; i++) {
      await seed(db, { title: `Post ${i}`, bodyMarkdown: `Body ${i}`, slug: `post-${i}` });
    }
    const res = await handleFeedRequest(db, { method: 'GET' });
    expect(entryCount(res.xml)).toBe(FEED_MAX_ENTRIES);
  });

  it('rejects non-GET methods with 405 and no body', async () => {
    const res = await handleFeedRequest(db, { method: 'POST' });
    expect(res.status).toBe(405);
    expect(res.xml).toBe('');
  });
});

describe('feed (node:http transport)', () => {
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

  it('serves /feed.xml as application/atom+xml with entries', async () => {
    await seed(db, { title: 'Wired Post', bodyMarkdown: '# Wire\n\nbody' });

    const res = await fetch(`${baseUrl}/feed.xml`);
    expect(res.status).toBe(200);
    expect(res.headers.get('content-type')).toContain('application/atom+xml');
    const xml = await res.text();
    expect(xml).toContain('<feed xmlns="http://www.w3.org/2005/Atom">');
    expect(xml).toContain('<title>Wired Post</title>');
    expect(entryCount(xml)).toBe(1);
  });

  it('does not let /feed.xml fall through to the HTML 404 page', async () => {
    const res = await fetch(`${baseUrl}/feed.xml`);
    expect(res.status).toBe(200);
    expect(res.headers.get('content-type')).not.toContain('text/html');
  });
});
