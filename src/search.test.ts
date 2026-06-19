/**
 * Integration tests for full-text search.
 *
 * Mirrors the other surface tests: `handleSearchRequest` is exercised directly
 * against the real data-access layer (pg-mem with migrations applied), and a
 * separate suite binds a `node:http` server to prove the transport adapter
 * routes `/search` and serves JSON or HTML with the right content type.
 */

import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import type { AddressInfo } from 'node:net';
import type { Server } from 'node:http';

import { handleApiRequest } from './api.js';
import { handleSearchRequest } from './search.js';
import { createServer } from './server.js';
import { migrate } from './db/migrate.js';
import { createMemoryDatabase } from './db/test-helpers.js';
import type { Queryable } from './db/pool.js';

const SEED_API_KEY = 'search-test-key';

/** Seed a published document through the real API path. */
async function seed(
  db: Queryable,
  body: { title: string; bodyMarkdown: string; slug?: string; tags?: string[] },
): Promise<void> {
  const created = await handleApiRequest(
    db,
    { method: 'POST', segments: ['documents'], body, headers: { 'x-api-key': SEED_API_KEY } },
    { apiKey: SEED_API_KEY },
  );
  expect(created.status).toBe(201);
  const slug = (created.body as { slug: string }).slug;
  const pub = await handleApiRequest(
    db,
    {
      method: 'POST',
      segments: ['documents', slug, 'publish'],
      headers: { 'x-api-key': SEED_API_KEY },
    },
    { apiKey: SEED_API_KEY },
  );
  expect(pub.status).toBe(200);
}

interface SearchJson {
  query: string;
  page: number;
  pageSize: number;
  total: number;
  results: { slug: string; title: string; excerpt: string; tags: string[] }[];
}

describe('search (handler)', () => {
  let db: Queryable;

  beforeEach(async () => {
    db = createMemoryDatabase().db;
    await migrate(db);
    await seed(db, {
      title: 'Intro to Rust',
      bodyMarkdown: 'ownership and borrowing',
      tags: ['rust'],
    });
    await seed(db, { title: 'Postgres tips', bodyMarkdown: 'the GIN index is great' });
    // A draft mentioning rust must never appear in results.
    const draft = await handleApiRequest(
      db,
      {
        method: 'POST',
        segments: ['documents'],
        body: { title: 'Draft about rust', bodyMarkdown: 'secret', slug: 'draft-rust' },
        headers: { 'x-api-key': SEED_API_KEY },
      },
      { apiKey: SEED_API_KEY },
    );
    expect(draft.status).toBe(201);
  });

  describe('JSON (?format=json)', () => {
    it('returns matching published documents with metadata', async () => {
      const res = await handleSearchRequest(db, {
        method: 'GET',
        query: { q: 'rust', format: 'json' },
      });
      expect(res.status).toBe(200);
      expect(res.contentType).toContain('application/json');
      const json = JSON.parse(res.body) as SearchJson;
      expect(json.query).toBe('rust');
      expect(json.total).toBe(1); // draft excluded
      expect(json.results[0]?.slug).toBe('intro-to-rust');
      expect(json.results[0]?.tags).toEqual(['rust']);
    });

    it('matches the body, case-insensitively', async () => {
      const res = await handleSearchRequest(db, {
        method: 'GET',
        query: { q: 'GIN', format: 'json' },
      });
      const json = JSON.parse(res.body) as SearchJson;
      expect(json.results.map((r) => r.slug)).toEqual(['postgres-tips']);
    });

    it('returns an empty payload for a blank query without hitting matches', async () => {
      const res = await handleSearchRequest(db, {
        method: 'GET',
        query: { q: '   ', format: 'json' },
      });
      const json = JSON.parse(res.body) as SearchJson;
      expect(json.total).toBe(0);
      expect(json.results).toEqual([]);
    });
  });

  describe('HTML', () => {
    it('renders the search form and matching results', async () => {
      const res = await handleSearchRequest(db, { method: 'GET', query: { q: 'rust' } });
      expect(res.status).toBe(200);
      expect(res.contentType).toContain('text/html');
      expect(res.body).toContain('<form class="search"');
      expect(res.body).toContain('Intro to Rust');
      expect(res.body).not.toContain('Draft about rust');
    });

    it('shows an explicit empty message when nothing matches', async () => {
      const res = await handleSearchRequest(db, { method: 'GET', query: { q: 'nonexistentword' } });
      expect(res.body).toContain('No results');
    });

    it('renders just the form for a blank query', async () => {
      const res = await handleSearchRequest(db, { method: 'GET', query: {} });
      expect(res.status).toBe(200);
      expect(res.body).toContain('<form class="search"');
      expect(res.body).not.toContain('No results');
    });
  });

  it('rejects non-GET methods with 405', async () => {
    const res = await handleSearchRequest(db, { method: 'POST', query: { q: 'rust' } });
    expect(res.status).toBe(405);
  });
});

describe('search (node:http transport)', () => {
  let db: Queryable;
  let server: Server;
  let baseUrl: string;

  beforeEach(async () => {
    db = createMemoryDatabase().db;
    await migrate(db);
    await seed(db, { title: 'Rust Guide', bodyMarkdown: 'all about rust' });
    server = createServer(db);
    await new Promise<void>((resolve) => server.listen(0, resolve));
    const { port } = server.address() as AddressInfo;
    baseUrl = `http://127.0.0.1:${port}`;
  });

  afterEach(() => new Promise<void>((resolve) => server.close(() => resolve())));

  it('serves JSON at /search?q=…&format=json', async () => {
    const res = await fetch(`${baseUrl}/search?q=rust&format=json`);
    expect(res.status).toBe(200);
    expect(res.headers.get('content-type')).toContain('application/json');
    const json = (await res.json()) as SearchJson;
    expect(json.results[0]?.slug).toBe('rust-guide');
  });

  it('serves the HTML results page at /search?q=…', async () => {
    const res = await fetch(`${baseUrl}/search?q=rust`);
    expect(res.status).toBe(200);
    expect(res.headers.get('content-type')).toContain('text/html');
    expect(await res.text()).toContain('Rust Guide');
  });
});
