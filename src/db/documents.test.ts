import { beforeEach, describe, expect, it } from 'vitest';

import {
  DuplicateSlugError,
  createDocument,
  deleteDocumentBySlug,
  getDocumentById,
  getDocumentBySlug,
  listDocuments,
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
});
