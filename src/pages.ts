/**
 * Public web frontend — server-rendered HTML pages for published documents.
 *
 * Where {@link handleApiRequest} (see `src/api.ts`) speaks JSON for programmatic
 * clients, this module renders the *human-facing* surface: a styled index of
 * published documents and a clean reading page per document. Like the API
 * handler it is framework-free — {@link handlePageRequest} takes a normalized
 * {@link PageRequest} plus a {@link Queryable} and returns a {@link PageResponse}
 * (status + HTML string), so the routing and templating are integration-tested
 * directly against the data-access layer without binding a socket. The thin
 * `node:http` adapter in `src/server.ts` decides whether a path is an API route
 * or a page route and dispatches accordingly.
 *
 * Safety note: a document's `renderedHtml` is already sanitized at write time by
 * the rendering pipeline (see `src/rendering.ts`), so it is embedded verbatim.
 * Every *other* value interpolated into a template — titles, the requested slug
 * echoed back on a 404 — is plain text and is HTML-escaped via {@link escapeHtml}
 * before insertion.
 */

import { getDocumentBySlug, listDocuments, type Document } from './db/documents.js';
import type { Queryable } from './db/pool.js';

/** A normalized inbound page request, independent of any HTTP framework. */
export interface PageRequest {
  /** Upper-case HTTP method, e.g. `GET`. */
  readonly method: string;
  /** Path split into non-empty segments, e.g. `/hello-world` -> `['hello-world']`. */
  readonly segments: readonly string[];
}

/** An HTML response to be written by the transport adapter. */
export interface PageResponse {
  readonly status: number;
  /** A complete HTML document. */
  readonly html: string;
}

/** Escape the five characters that are unsafe in HTML text/attribute contexts. */
export function escapeHtml(value: string): string {
  return value
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
    .replace(/'/g, '&#39;');
}

/**
 * Minimal, dependency-free stylesheet. Inlined into every page so the frontend
 * needs no static-asset pipeline for v0.1 — one request renders a styled page.
 * A readable measure, system font stack, and gentle defaults for the kind of
 * prose elements the rendering allowlist can emit (headings, code, tables,
 * blockquotes, images).
 */
const STYLES = `
  :root { color-scheme: light dark; }
  * { box-sizing: border-box; }
  body {
    margin: 0;
    font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, Helvetica, Arial, sans-serif;
    line-height: 1.6;
    color: #1a1a1a;
    background: #fdfdfd;
  }
  .wrap { max-width: 42rem; margin: 0 auto; padding: 3rem 1.25rem 5rem; }
  header.site { margin-bottom: 2.5rem; }
  header.site a.brand { font-weight: 700; font-size: 1.1rem; color: inherit; text-decoration: none; }
  h1, h2, h3, h4, h5, h6 { line-height: 1.25; margin: 2rem 0 0.75rem; }
  h1 { font-size: 2rem; }
  p, ul, ol, blockquote, table, pre, figure { margin: 0 0 1.1rem; }
  a { color: #0b5fff; }
  img { max-width: 100%; height: auto; }
  pre { background: #f4f4f6; padding: 1rem; border-radius: 6px; overflow-x: auto; }
  code { background: #f4f4f6; padding: 0.15em 0.35em; border-radius: 4px; font-size: 0.9em; }
  pre code { background: none; padding: 0; }
  blockquote { border-left: 3px solid #d0d0d8; margin-left: 0; padding-left: 1rem; color: #555; }
  table { border-collapse: collapse; width: 100%; }
  th, td { border: 1px solid #e0e0e6; padding: 0.4rem 0.6rem; text-align: left; }
  .meta { color: #777; font-size: 0.875rem; }
  ul.index { list-style: none; padding: 0; }
  ul.index li { margin: 0 0 1.25rem; }
  ul.index a { font-size: 1.15rem; font-weight: 600; text-decoration: none; }
  ul.index a:hover { text-decoration: underline; }
  .empty { color: #777; font-style: italic; }
  footer.site { margin-top: 4rem; color: #aaa; font-size: 0.8rem; }
  @media (prefers-color-scheme: dark) {
    body { color: #e6e6e6; background: #16161a; }
    a { color: #6ea8ff; }
    pre, code { background: #24242b; }
    blockquote { border-left-color: #44444f; color: #aaa; }
    th, td { border-color: #33333b; }
  }
`;

/** Wrap page-specific `<main>` markup in the shared HTML shell + styles. */
function layout(title: string, main: string): string {
  return `<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <title>${escapeHtml(title)}</title>
    <style>${STYLES}</style>
  </head>
  <body>
    <div class="wrap">
      <header class="site"><a class="brand" href="/">Inkwell</a></header>
      <main>
${main}
      </main>
      <footer class="site">Published with Inkwell.</footer>
    </div>
  </body>
</html>
`;
}

/** Format a timestamp as a machine-readable + human-readable date pair. */
function dateLine(label: string, date: Date): string {
  const iso = date.toISOString();
  const human = iso.slice(0, 10); // YYYY-MM-DD — deterministic, locale-independent
  return `<time datetime="${iso}">${label} ${human}</time>`;
}

/** Render the index: a list of published documents, newest first. */
export function renderIndexPage(documents: readonly Document[]): string {
  const main =
    documents.length === 0
      ? `<p class="empty">No documents published yet.</p>`
      : `<ul class="index">
${documents
  .map(
    (doc) => `          <li>
            <a href="/${encodeURIComponent(doc.slug)}">${escapeHtml(doc.title)}</a>
            <div class="meta">${dateLine('Published', doc.createdAt)}</div>
          </li>`,
  )
  .join('\n')}
        </ul>`;
  return layout('Inkwell', main);
}

/** Render a single document's public reading page. */
export function renderDocumentPage(document: Document): string {
  const updated =
    document.updatedAt.getTime() !== document.createdAt.getTime()
      ? ` &middot; ${dateLine('Updated', document.updatedAt)}`
      : '';
  const main = `<article>
          <h1>${escapeHtml(document.title)}</h1>
          <div class="meta">${dateLine('Published', document.createdAt)}${updated}</div>
${document.renderedHtml}
        </article>`;
  return layout(document.title, main);
}

/** Render a styled 404 page for an unknown path/slug. */
export function renderNotFoundPage(): string {
  const main = `<h1>Not found</h1>
        <p>That page does not exist. <a href="/">Back to the index.</a></p>`;
  return layout('Not found — Inkwell', main);
}

/**
 * Route and render a single public page request.
 *
 * Recognized routes (GET/HEAD only):
 *   - `GET /`        -> index of published documents
 *   - `GET /:slug`   -> a document's public page (404 page if the slug is unknown)
 *
 * Non-GET methods yield 405; any deeper path yields a 404 page. The `/documents`
 * and `/health` prefixes are reserved for the JSON API and never reach here —
 * the transport adapter dispatches those to {@link handleApiRequest} instead.
 */
export async function handlePageRequest(db: Queryable, req: PageRequest): Promise<PageResponse> {
  if (req.method !== 'GET' && req.method !== 'HEAD') {
    return { status: 405, html: renderNotFoundPage() };
  }

  // Index — the public frontend only ever surfaces published documents.
  if (req.segments.length === 0) {
    const docs = await listDocuments(db, { status: 'published' });
    return { status: 200, html: renderIndexPage(docs) };
  }

  // Single document page. A draft (or unknown slug) renders the 404 page so a
  // draft's existence isn't leaked through the public surface.
  if (req.segments.length === 1) {
    const doc = await getDocumentBySlug(db, req.segments[0] as string, { status: 'published' });
    if (doc) {
      return { status: 200, html: renderDocumentPage(doc) };
    }
  }

  // Unknown slug or a path too deep to be a public page.
  return { status: 404, html: renderNotFoundPage() };
}
