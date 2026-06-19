/**
 * Public web frontend — server-rendered HTML pages for published documents.
 *
 * Where {@link handleApiRequest} (see `src/api.ts`) speaks JSON for programmatic
 * clients, this module renders the *human-facing* surface: a styled, paginated
 * index of published documents and a clean reading page per document. Like the
 * API handler it is framework-free — {@link handlePageRequest} takes a normalized
 * {@link PageRequest} plus a {@link Queryable} and returns a {@link PageResponse}
 * (status + HTML string), so the routing and templating are integration-tested
 * directly against the data-access layer without binding a socket. The thin
 * `node:http` adapter in `src/server.ts` decides whether a path is an API route
 * or a page route and dispatches accordingly.
 *
 * Discovery & SEO: every page carries a canonical link plus OpenGraph, Twitter
 * Card, and (for document pages) JSON-LD `BlogPosting` metadata, so published
 * content is rich and rankable when shared or crawled. Absolute URLs are built
 * from the configured site origin (see `src/site-url.ts`).
 *
 * Safety note: a document's `renderedHtml` is already sanitized at write time by
 * the rendering pipeline (see `src/rendering.ts`), so it is embedded verbatim.
 * Every *other* value interpolated into a template — titles, excerpts, meta
 * descriptions, the JSON-LD payload — is plain text and is escaped for its
 * context (HTML, attribute, or JSON) before insertion.
 */

import { countDocuments, getDocumentBySlug, listDocuments, type Document } from './db/documents.js';
import type { Queryable } from './db/pool.js';
import { normalizeSiteUrl } from './site-url.js';

/** Number of documents shown per index page. */
export const PAGE_SIZE = 10;

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

/** Server-side options for the page handler (absolute-URL construction). */
export interface PageOptions {
  /**
   * The public origin the site is served from, e.g. `https://blog.example.com`.
   * Used to build canonical/OpenGraph/JSON-LD absolute URLs. Read from
   * `INKWELL_SITE_URL`; falls back to a localhost origin. Trailing slash trimmed.
   */
  readonly siteUrl?: string | undefined;
}

/** The site/brand name surfaced in titles, OpenGraph `og:site_name`, and JSON-LD. */
const SITE_NAME = 'Inkwell';

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
 * Serialize a value as JSON safe to embed inside a `<script>` element. JSON is
 * not HTML-escaped by browsers, so the only injection risk is a literal `</`
 * sequence (notably `</script>`); escaping `<`, `>`, and `&` to their unicode
 * escapes neutralizes that while keeping the JSON valid.
 */
function jsonForScript(value: unknown): string {
  return JSON.stringify(value)
    .replace(/</g, '\\u003c')
    .replace(/>/g, '\\u003e')
    .replace(/&/g, '\\u0026');
}

/**
 * Derive a short plain-text excerpt from a document's Markdown body, suitable
 * for an index summary and a `<meta name="description">`. This is intentionally
 * lightweight — strip the most common Markdown syntax, collapse whitespace, and
 * truncate on a word boundary — rather than re-parsing to an AST. Returns an
 * empty string for empty/whitespace-only input.
 */
export function deriveExcerpt(markdown: string, maxLength = 160): string {
  const text = markdown
    .replace(/```[\s\S]*?```/g, ' ') // fenced code blocks
    .replace(/`([^`]*)`/g, '$1') // inline code -> its text
    .replace(/!\[[^\]]*\]\([^)]*\)/g, ' ') // images -> drop
    .replace(/\[([^\]]*)\]\([^)]*\)/g, '$1') // links -> link text
    .replace(/^\s{0,3}#{1,6}\s+/gm, '') // ATX heading markers
    .replace(/^\s{0,3}>\s?/gm, '') // blockquote markers
    .replace(/^\s{0,3}(?:[-*+]|\d+\.)\s+/gm, '') // list markers
    .replace(/[*_~]/g, '') // emphasis/strikethrough
    .replace(/<[^>]+>/g, ' ') // any inline HTML tags
    .replace(/\s+/g, ' ')
    .trim();

  if (text.length <= maxLength) return text;
  // Truncate at the last word boundary within the budget, then add an ellipsis.
  const clipped = text.slice(0, maxLength);
  const lastSpace = clipped.lastIndexOf(' ');
  const head = lastSpace > 0 ? clipped.slice(0, lastSpace) : clipped;
  return `${head.trimEnd()}…`;
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
  /* highlight.js theme — colors the <span class="hljs-…"> tokens emitted by the
     server-side syntax highlighter (see src/rendering.ts). GitHub-light palette. */
  .hljs { color: #1a1a1a; }
  .hljs-comment, .hljs-quote { color: #6a737d; font-style: italic; }
  .hljs-keyword, .hljs-selector-tag, .hljs-literal, .hljs-doctag { color: #d73a49; }
  .hljs-string, .hljs-regexp, .hljs-meta .hljs-string { color: #032f62; }
  .hljs-number, .hljs-symbol, .hljs-bullet { color: #005cc5; }
  .hljs-title, .hljs-section, .hljs-function .hljs-title, .hljs-name { color: #6f42c1; }
  .hljs-type, .hljs-class .hljs-title, .hljs-built_in, .hljs-builtin-name { color: #e36209; }
  .hljs-attr, .hljs-attribute, .hljs-variable, .hljs-template-variable { color: #005cc5; }
  .hljs-tag { color: #22863a; }
  .hljs-meta { color: #6a737d; }
  .hljs-deletion { color: #b31d28; background: #ffeef0; }
  .hljs-addition { color: #22863a; background: #f0fff4; }
  .hljs-emphasis { font-style: italic; }
  .hljs-strong { font-weight: 700; }
  blockquote { border-left: 3px solid #d0d0d8; margin-left: 0; padding-left: 1rem; color: #555; }
  table { border-collapse: collapse; width: 100%; }
  th, td { border: 1px solid #e0e0e6; padding: 0.4rem 0.6rem; text-align: left; }
  .meta { color: #777; font-size: 0.875rem; }
  ul.index { list-style: none; padding: 0; }
  ul.index li { margin: 0 0 1.75rem; }
  ul.index a.title { font-size: 1.15rem; font-weight: 600; text-decoration: none; }
  ul.index a.title:hover { text-decoration: underline; }
  ul.index .excerpt { margin: 0.35rem 0 0; color: #444; }
  nav.pager { display: flex; justify-content: space-between; margin-top: 2.5rem; }
  nav.pager a { text-decoration: none; }
  nav.pager .spacer { color: transparent; }
  .empty { color: #777; font-style: italic; }
  footer.site { margin-top: 4rem; color: #aaa; font-size: 0.8rem; }
  @media (prefers-color-scheme: dark) {
    body { color: #e6e6e6; background: #16161a; }
    a { color: #6ea8ff; }
    pre, code { background: #24242b; }
    ul.index .excerpt { color: #b8b8c0; }
    blockquote { border-left-color: #44444f; color: #aaa; }
    th, td { border-color: #33333b; }
    /* highlight.js dark palette (GitHub-dark) */
    .hljs { color: #e6e6e6; }
    .hljs-comment, .hljs-quote { color: #8b949e; }
    .hljs-keyword, .hljs-selector-tag, .hljs-literal, .hljs-doctag { color: #ff7b72; }
    .hljs-string, .hljs-regexp, .hljs-meta .hljs-string { color: #a5d6ff; }
    .hljs-number, .hljs-symbol, .hljs-bullet { color: #79c0ff; }
    .hljs-title, .hljs-section, .hljs-function .hljs-title, .hljs-name { color: #d2a8ff; }
    .hljs-type, .hljs-class .hljs-title, .hljs-built_in, .hljs-builtin-name { color: #ffa657; }
    .hljs-attr, .hljs-attribute, .hljs-variable, .hljs-template-variable { color: #79c0ff; }
    .hljs-tag { color: #7ee787; }
    .hljs-meta { color: #8b949e; }
    .hljs-deletion { color: #ffdcd7; background: #67060c; }
    .hljs-addition { color: #aff5b4; background: #033a16; }
  }
`;

/** SEO/social metadata for a page's `<head>`. */
interface HeadMeta {
  /** Full `<title>` text (already includes any site-name suffix). */
  readonly title: string;
  /** Plain-text description for `<meta name="description">` and OpenGraph. */
  readonly description?: string | undefined;
  /** Absolute canonical URL for this page. */
  readonly canonicalUrl: string;
  /** OpenGraph object type. */
  readonly ogType: 'website' | 'article';
  /** Optional JSON-LD payload embedded as `application/ld+json`. */
  readonly jsonLd?: Record<string, unknown> | undefined;
}

/** Build the discovery/SEO `<meta>`/`<link>` block for a page head. */
function renderHead(meta: HeadMeta): string {
  const tags: string[] = [
    `<meta charset="utf-8" />`,
    `<meta name="viewport" content="width=device-width, initial-scale=1" />`,
    `<title>${escapeHtml(meta.title)}</title>`,
    `<link rel="canonical" href="${escapeHtml(meta.canonicalUrl)}" />`,
    `<link rel="alternate" type="application/atom+xml" title="${escapeHtml(SITE_NAME)}" href="/feed.xml" />`,
  ];
  if (meta.description) {
    tags.push(`<meta name="description" content="${escapeHtml(meta.description)}" />`);
  }
  // OpenGraph.
  tags.push(
    `<meta property="og:type" content="${meta.ogType}" />`,
    `<meta property="og:site_name" content="${escapeHtml(SITE_NAME)}" />`,
    `<meta property="og:title" content="${escapeHtml(meta.title)}" />`,
    `<meta property="og:url" content="${escapeHtml(meta.canonicalUrl)}" />`,
  );
  if (meta.description) {
    tags.push(`<meta property="og:description" content="${escapeHtml(meta.description)}" />`);
  }
  // Twitter Card.
  tags.push(
    `<meta name="twitter:card" content="summary" />`,
    `<meta name="twitter:title" content="${escapeHtml(meta.title)}" />`,
  );
  if (meta.description) {
    tags.push(`<meta name="twitter:description" content="${escapeHtml(meta.description)}" />`);
  }
  if (meta.jsonLd) {
    tags.push(`<script type="application/ld+json">${jsonForScript(meta.jsonLd)}</script>`);
  }
  return tags.map((tag) => `    ${tag}`).join('\n');
}

/** Wrap page-specific `<main>` markup in the shared HTML shell + styles. */
function layout(meta: HeadMeta, main: string): string {
  return `<!doctype html>
<html lang="en">
  <head>
${renderHead(meta)}
    <style>${STYLES}</style>
  </head>
  <body>
    <div class="wrap">
      <header class="site"><a class="brand" href="/">${escapeHtml(SITE_NAME)}</a></header>
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

/** Absolute URL for a document's public page. */
function documentUrl(base: string, slug: string): string {
  return `${base}/${encodeURIComponent(slug)}`;
}

/** Absolute URL for index page `n` (page 1 is the bare site root). */
function indexUrl(base: string, page: number): string {
  return page <= 1 ? `${base}/` : `${base}/page/${page}`;
}

/** Relative href for index page `n` (page 1 is `/`). */
function indexHref(page: number): string {
  return page <= 1 ? '/' : `/page/${page}`;
}

/** Pagination context passed to {@link renderIndexPage}. */
export interface IndexPageInfo {
  /** 1-based current page number. */
  readonly page: number;
  /** Total number of index pages (at least 1). */
  readonly totalPages: number;
}

/** Render the index: a list of published documents, newest first, paginated. */
export function renderIndexPage(
  documents: readonly Document[],
  info: IndexPageInfo,
  options: PageOptions = {},
): string {
  const base = normalizeSiteUrl(options.siteUrl);
  const list =
    documents.length === 0
      ? `<p class="empty">No documents published yet.</p>`
      : `<ul class="index">
${documents
  .map((doc) => {
    const excerpt = deriveExcerpt(doc.bodyMarkdown);
    const excerptHtml = excerpt
      ? `\n            <p class="excerpt">${escapeHtml(excerpt)}</p>`
      : '';
    return `          <li>
            <a class="title" href="/${encodeURIComponent(doc.slug)}">${escapeHtml(doc.title)}</a>
            <div class="meta">${dateLine('Published', doc.createdAt)}</div>${excerptHtml}
          </li>`;
  })
  .join('\n')}
        </ul>`;

  // Prev/next pager. Keep both slots present (a transparent spacer) so the
  // single remaining link stays in its column.
  const prev =
    info.page > 1
      ? `<a rel="prev" href="${indexHref(info.page - 1)}">&larr; Newer</a>`
      : `<span class="spacer">&larr; Newer</span>`;
  const next =
    info.page < info.totalPages
      ? `<a rel="next" href="${indexHref(info.page + 1)}">Older &rarr;</a>`
      : `<span class="spacer">Older &rarr;</span>`;
  const pager = info.totalPages > 1 ? `\n        <nav class="pager">${prev}${next}</nav>` : '';

  const title = info.page > 1 ? `${SITE_NAME} — Page ${info.page}` : SITE_NAME;
  return layout(
    {
      title,
      description: 'An open, API-first Markdown publishing platform.',
      canonicalUrl: indexUrl(base, info.page),
      ogType: 'website',
    },
    `${list}${pager}`,
  );
}

/** Render a single document's public reading page. */
export function renderDocumentPage(document: Document, options: PageOptions = {}): string {
  const base = normalizeSiteUrl(options.siteUrl);
  const url = documentUrl(base, document.slug);
  const description = deriveExcerpt(document.bodyMarkdown);
  const isUpdated = document.updatedAt.getTime() !== document.createdAt.getTime();
  const updated = isUpdated ? ` &middot; ${dateLine('Updated', document.updatedAt)}` : '';

  // JSON-LD BlogPosting: the structured-data view search engines consume.
  const jsonLd: Record<string, unknown> = {
    '@context': 'https://schema.org',
    '@type': 'BlogPosting',
    headline: document.title,
    datePublished: document.createdAt.toISOString(),
    dateModified: document.updatedAt.toISOString(),
    url,
    mainEntityOfPage: { '@type': 'WebPage', '@id': url },
    publisher: { '@type': 'Organization', name: SITE_NAME },
    inLanguage: 'en',
  };
  if (description) jsonLd.description = description;

  const main = `<article>
          <h1>${escapeHtml(document.title)}</h1>
          <div class="meta">${dateLine('Published', document.createdAt)}${updated}</div>
${document.renderedHtml}
        </article>`;
  return layout(
    {
      title: `${document.title} — ${SITE_NAME}`,
      description: description || undefined,
      canonicalUrl: url,
      ogType: 'article',
      jsonLd,
    },
    main,
  );
}

/** Render a styled 404 page for an unknown path/slug. */
export function renderNotFoundPage(options: PageOptions = {}): string {
  const base = normalizeSiteUrl(options.siteUrl);
  const main = `<h1>Not found</h1>
        <p>That page does not exist. <a href="/">Back to the index.</a></p>`;
  return layout(
    {
      title: `Not found — ${SITE_NAME}`,
      canonicalUrl: `${base}/`,
      ogType: 'website',
    },
    main,
  );
}

/**
 * Parse a `/page/:n` segment into a 1-based page number, or `null` if it is not
 * a positive integer. Rejects leading zeros, signs, and non-digits so only one
 * canonical spelling of each page exists.
 */
function parsePageNumber(raw: string): number | null {
  if (!/^[1-9][0-9]*$/.test(raw)) return null;
  const n = Number.parseInt(raw, 10);
  return Number.isSafeInteger(n) ? n : null;
}

/**
 * Route and render a single public page request.
 *
 * Recognized routes (GET/HEAD only):
 *   - `GET /`        -> index of published documents (page 1)
 *   - `GET /page/:n` -> index page N (newest first, {@link PAGE_SIZE} per page)
 *   - `GET /:slug`   -> a document's public page (404 page if the slug is unknown)
 *
 * Non-GET methods yield 405; any deeper/unrecognized path yields a 404 page. The
 * `/documents` and `/health` prefixes are reserved for the JSON API and never
 * reach here — the transport adapter dispatches those to {@link handleApiRequest}.
 */
export async function handlePageRequest(
  db: Queryable,
  req: PageRequest,
  options: PageOptions = {},
): Promise<PageResponse> {
  if (req.method !== 'GET' && req.method !== 'HEAD') {
    return { status: 405, html: renderNotFoundPage(options) };
  }

  // Index, optionally paginated. The public frontend only surfaces published docs.
  const isRoot = req.segments.length === 0;
  const isPaged = req.segments.length === 2 && req.segments[0] === 'page';
  if (isRoot || isPaged) {
    const page = isRoot ? 1 : parsePageNumber(req.segments[1] as string);
    if (page === null) return { status: 404, html: renderNotFoundPage(options) };

    const total = await countDocuments(db, { status: 'published' });
    const totalPages = Math.max(1, Math.ceil(total / PAGE_SIZE));
    // A page past the end (other than page 1 on an empty site) does not exist —
    // 404 rather than serve an empty page, so crawlers don't chase phantom pages.
    if (page > totalPages) return { status: 404, html: renderNotFoundPage(options) };

    const docs = await listDocuments(db, {
      status: 'published',
      limit: PAGE_SIZE,
      offset: (page - 1) * PAGE_SIZE,
    });
    return { status: 200, html: renderIndexPage(docs, { page, totalPages }, options) };
  }

  // Single document page. A draft (or unknown slug) renders the 404 page so a
  // draft's existence isn't leaked through the public surface.
  if (req.segments.length === 1) {
    const doc = await getDocumentBySlug(db, req.segments[0] as string, { status: 'published' });
    if (doc) {
      return { status: 200, html: renderDocumentPage(doc, options) };
    }
  }

  // Unknown slug or a path too deep to be a public page.
  return { status: 404, html: renderNotFoundPage(options) };
}
