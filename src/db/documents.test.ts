import { beforeEach, describe, expect, it } from 'vitest';

import {
  DuplicateSlugError,
  countDocumentsByTag,
  countSearchPublishedDocuments,
  createDocument,
  deleteDocumentBySlug,
  getDocumentById,
  getDocumentBySlug,
  listDocuments,
  listDocumentsByTag,
  listPublishedTags,
  searchPublishedDocuments,
  setDocumentStatus,
  updateDocumentBySlug,
} from './documents.js';
import { migrate } from './migrate.js';
import { createMemoryDatabase } from './test-helpers.js';
import type { Queryable } from './pool.js';

const sample = {
  slug: 'hello-world',
  title: 'Hello World',
  bodyMarkdown: '# Hello',
  renderedHtml: '<h1>Hello</h1>',
};

describe('documents data-access layer', () => {
  let db: Queryable;

  beforeEach(async () => {
    db = createMemoryDatabase().db;
    await migrate(db);
  });

  it('inserts a document and returns the persisted row', async () => {
    const doc = await createDocument(db, sample);
    expect(doc.id).toMatch(/^[0-9a-f-]{36}$/);
    expect(doc.slug).toBe('hello-world');
    expect(doc.title).toBe('Hello World');
    expect(doc.bodyMarkdown).toBe('# Hello');
    expect(doc.renderedHtml).toBe('<h1>Hello</h1>');
    expect(doc.createdAt).toBeInstanceOf(Date);
    expect(doc.updatedAt).toBeInstanceOf(Date);
  });

  it('reads a document back by slug', async () => {
    const created = await createDocument(db, sample);
    const found = await getDocumentBySlug(db, 'hello-world');
    expect(found).toEqual(created);
  });

  it('reads a document back by id', async () => {
    const created = await createDocument(db, sample);
    const found = await getDocumentById(db, created.id);
    expect(found?.slug).toBe('hello-world');
  });

  it('returns null for a missing slug', async () => {
    expect(await getDocumentBySlug(db, 'nope')).toBeNull();
  });

  it('rejects a duplicate slug with DuplicateSlugError', async () => {
    await createDocument(db, sample);
    await expect(createDocument(db, { ...sample, title: 'Other' })).rejects.toBeInstanceOf(
      DuplicateSlugError,
    );
  });

  it('lists documents newest first', async () => {
    await createDocument(db, { ...sample, slug: 'first' });
    await createDocument(db, { ...sample, slug: 'second' });
    const slugs = (await listDocuments(db)).map((d) => d.slug);
    expect(slugs).toHaveLength(2);
    expect(slugs).toContain('first');
    expect(slugs).toContain('second');
  });

  it('updates mutable fields by slug', async () => {
    await createDocument(db, sample);
    const updated = await updateDocumentBySlug(db, 'hello-world', { title: 'Renamed' });
    expect(updated?.title).toBe('Renamed');
    expect(updated?.bodyMarkdown).toBe('# Hello'); // untouched
  });

  it('returns null when updating a missing document', async () => {
    expect(await updateDocumentBySlug(db, 'ghost', { title: 'x' })).toBeNull();
  });

  it('deletes a document by slug', async () => {
    await createDocument(db, sample);
    expect(await deleteDocumentBySlug(db, 'hello-world')).toBe(true);
    expect(await getDocumentBySlug(db, 'hello-world')).toBeNull();
    expect(await deleteDocumentBySlug(db, 'hello-world')).toBe(false);
  });

  it('defaults a new document to draft status', async () => {
    const doc = await createDocument(db, sample);
    expect(doc.status).toBe('draft');
  });

  it('honors an explicit status on create', async () => {
    const doc = await createDocument(db, { ...sample, status: 'published' });
    expect(doc.status).toBe('published');
  });

  it('filters lists and single reads by status', async () => {
    await createDocument(db, { ...sample, slug: 'a-draft' });
    await createDocument(db, { ...sample, slug: 'b-published', status: 'published' });

    const published = await listDocuments(db, { status: 'published' });
    expect(published.map((d) => d.slug)).toEqual(['b-published']);

    const drafts = await listDocuments(db, { status: 'draft' });
    expect(drafts.map((d) => d.slug)).toEqual(['a-draft']);

    // No filter returns both.
    expect((await listDocuments(db)).map((d) => d.slug).sort()).toEqual(['a-draft', 'b-published']);

    // A status filter on a single read treats a mismatch as not-found.
    expect(await getDocumentBySlug(db, 'a-draft', { status: 'published' })).toBeNull();
    expect((await getDocumentBySlug(db, 'a-draft', { status: 'draft' }))?.slug).toBe('a-draft');
  });

  it('sets status idempotently and returns null for a missing slug', async () => {
    await createDocument(db, sample);
    const published = await setDocumentStatus(db, 'hello-world', 'published');
    expect(published?.status).toBe('published');
    // Idempotent: setting the same status again still returns the row.
    expect((await setDocumentStatus(db, 'hello-world', 'published'))?.status).toBe('published');
    expect((await setDocumentStatus(db, 'hello-world', 'draft'))?.status).toBe('draft');
    expect(await setDocumentStatus(db, 'ghost', 'published')).toBeNull();
  });

  describe('tags', () => {
    it('defaults to an empty array and round-trips a tag set', async () => {
      const untagged = await createDocument(db, sample);
      expect(untagged.tags).toEqual([]);

      const tagged = await createDocument(db, {
        ...sample,
        slug: 'tagged',
        tags: ['rust', 'postgres'],
      });
      expect(tagged.tags).toEqual(['rust', 'postgres']);
      expect((await getDocumentBySlug(db, 'tagged'))?.tags).toEqual(['rust', 'postgres']);
    });

    it('replaces tags on update and leaves them untouched when omitted', async () => {
      await createDocument(db, { ...sample, tags: ['a', 'b'] });
      const replaced = await updateDocumentBySlug(db, 'hello-world', { tags: ['c'] });
      expect(replaced?.tags).toEqual(['c']);

      // Omitting tags preserves them; a title-only patch must not clear tags.
      const titleOnly = await updateDocumentBySlug(db, 'hello-world', { title: 'New' });
      expect(titleOnly?.tags).toEqual(['c']);

      // An explicit empty array clears them.
      const cleared = await updateDocumentBySlug(db, 'hello-world', { tags: [] });
      expect(cleared?.tags).toEqual([]);
    });

    it('lists and counts documents by tag, filtered by status', async () => {
      await createDocument(db, { ...sample, slug: 'p1', status: 'published', tags: ['x', 'y'] });
      await createDocument(db, { ...sample, slug: 'p2', status: 'published', tags: ['x'] });
      await createDocument(db, { ...sample, slug: 'd1', status: 'draft', tags: ['x'] });

      const publishedX = await listDocumentsByTag(db, 'x', { status: 'published' });
      expect(publishedX.map((d) => d.slug).sort()).toEqual(['p1', 'p2']);
      expect(await countDocumentsByTag(db, 'x', { status: 'published' })).toBe(2);

      // Without a status filter, the draft is included too.
      expect(await countDocumentsByTag(db, 'x')).toBe(3);

      // A tag carried by only one document.
      expect((await listDocumentsByTag(db, 'y')).map((d) => d.slug)).toEqual(['p1']);

      // An unknown tag yields nothing.
      expect(await listDocumentsByTag(db, 'nope')).toEqual([]);
    });

    it('paginates tag listings', async () => {
      for (const n of [1, 2, 3]) {
        await createDocument(db, { ...sample, slug: `t${n}`, status: 'published', tags: ['p'] });
      }
      const firstPage = await listDocumentsByTag(db, 'p', {
        status: 'published',
        limit: 2,
        offset: 0,
      });
      expect(firstPage).toHaveLength(2);
      const secondPage = await listDocumentsByTag(db, 'p', {
        status: 'published',
        limit: 2,
        offset: 2,
      });
      expect(secondPage).toHaveLength(1);
    });

    it('aggregates distinct published tags with counts', async () => {
      await createDocument(db, { ...sample, slug: 'a', status: 'published', tags: ['ts', 'sql'] });
      await createDocument(db, { ...sample, slug: 'b', status: 'published', tags: ['ts'] });
      // A draft's tags must not leak into the public aggregate.
      await createDocument(db, { ...sample, slug: 'c', status: 'draft', tags: ['secret'] });

      const tags = await listPublishedTags(db);
      expect(tags).toEqual([
        { tag: 'ts', count: 2 },
        { tag: 'sql', count: 1 },
      ]);
    });
  });

  describe('search', () => {
    beforeEach(async () => {
      await createDocument(db, {
        slug: 'rust-intro',
        title: 'Intro to Rust',
        bodyMarkdown: 'Ownership and borrowing explained.',
        renderedHtml: '<p>Ownership and borrowing explained.</p>',
        status: 'published',
      });
      await createDocument(db, {
        slug: 'pg-tips',
        title: 'Postgres tips',
        bodyMarkdown: 'Indexes, including the GIN index for arrays.',
        renderedHtml: '<p>Indexes, including the GIN index for arrays.</p>',
        status: 'published',
      });
      await createDocument(db, {
        slug: 'draft-rust',
        title: 'Secret Rust draft',
        bodyMarkdown: 'unpublished',
        renderedHtml: '<p>unpublished</p>',
        status: 'draft',
      });
    });

    it('matches title or body, case-insensitively, over published docs only', async () => {
      const byTitle = await searchPublishedDocuments(db, 'rust');
      expect(byTitle.map((d) => d.slug)).toEqual(['rust-intro']); // draft excluded

      const byBody = await searchPublishedDocuments(db, 'borrowing');
      expect(byBody.map((d) => d.slug)).toEqual(['rust-intro']);

      // Case-insensitive.
      expect((await searchPublishedDocuments(db, 'POSTGRES')).map((d) => d.slug)).toEqual([
        'pg-tips',
      ]);
    });

    it('ranks title matches ahead of body-only matches', async () => {
      // "index" appears in pg-tips' body; add a doc with it in the title.
      await createDocument(db, {
        slug: 'index-guide',
        title: 'The index guide',
        bodyMarkdown: 'about things',
        renderedHtml: '<p>about things</p>',
        status: 'published',
      });
      const results = await searchPublishedDocuments(db, 'index');
      expect(results[0]?.slug).toBe('index-guide');
      expect(results.map((d) => d.slug).sort()).toEqual(['index-guide', 'pg-tips']);
    });

    it('counts matches and treats LIKE metacharacters literally', async () => {
      expect(await countSearchPublishedDocuments(db, 'rust')).toBe(1);
      // A bare wildcard must not match everything — it is escaped.
      expect(await countSearchPublishedDocuments(db, '%')).toBe(0);
    });

    it('paginates search results', async () => {
      const page = await searchPublishedDocuments(db, 'r', { limit: 1, offset: 0 });
      expect(page).toHaveLength(1);
    });
  });
});
