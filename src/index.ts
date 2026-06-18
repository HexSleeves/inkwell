/**
 * Inkwell — an open, API-first Markdown publishing platform.
 *
 * This is the package entry point. As the core grows it will export the
 * public API surface (document model, rendering pipeline, HTTP server).
 * For now it exposes a small, tested utility so the scaffold has something
 * real to build, lint, and test against.
 */

export const NAME = 'inkwell';
export const VERSION = '0.1.0';

export { renderMarkdown, renderDocumentHtml } from './rendering.js';

/**
 * Derive a URL-safe slug from a document title.
 *
 * Slugs are used to build the public page path for a published document
 * (e.g. "Hello, World!" -> "hello-world"). Kept deliberately small and
 * dependency-free; it lowercases, strips diacritics, converts runs of
 * non-alphanumerics to hyphens, and trims leading/trailing hyphens.
 */
export function slugify(title: string): string {
  return title
    .normalize('NFKD')
    .replace(/[̀-ͯ]/g, '') // strip diacritics
    .toLowerCase()
    .trim()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '');
}
