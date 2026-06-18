/**
 * Slug derivation.
 *
 * Slugs are the public URL key for a published document. This lives in its own
 * module so both the package entry point and the HTTP API can depend on it
 * without creating an import cycle between them.
 */

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
