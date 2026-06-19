/**
 * `sitemap.xml` for published documents.
 *
 * A sitemap lets search engines discover every public URL and learn when each
 * last changed, which is the baseline crawlability win for SEO. Like
 * `src/feed.ts` this module is framework-free: {@link handleSitemapRequest} takes
 * a normalized {@link SitemapRequest} plus a {@link Queryable} and returns a
 * {@link SitemapResponse} (status + content-type + XML body), integration-tested
 * directly against the data-access layer. The thin `node:http` adapter in
 * `src/server.ts` dispatches `GET /sitemap.xml` here.
 *
 * The XML is built by hand from template literals — no XML dependency. Every
 * interpolated URL is XML-escaped via {@link escapeXml}. Only `published`
 * documents are listed, so the sitemap never leaks a draft. The `/tags` index
 * and one `/tags/:tag` URL per published tag are listed alongside documents.
 */

import {
  listPublishedDocuments,
  listPublishedTags,
  type Document,
  type TagCount,
} from './db/documents.js';
import type { Queryable } from './db/pool.js';
import { normalizeSiteUrl } from './site-url.js';

/** The exact Content-Type for an XML sitemap. */
export const SITEMAP_CONTENT_TYPE = 'application/xml; charset=utf-8';

/** A normalized inbound sitemap request, independent of any HTTP framework. */
export interface SitemapRequest {
  /** Upper-case HTTP method, e.g. `GET`. */
  readonly method: string;
}

/** A sitemap response to be written by the transport adapter. */
export interface SitemapResponse {
  readonly status: number;
  /** Value for the `Content-Type` header. */
  readonly contentType: string;
  /** The serialized sitemap (an XML urlset), or empty for non-200s. */
  readonly xml: string;
}

/** Server-side options for {@link buildSitemap} / {@link handleSitemapRequest}. */
export interface SitemapOptions {
  /**
   * The public origin the site is served from. Read from `INKWELL_SITE_URL`;
   * falls back to a localhost origin. A trailing slash is tolerated and trimmed.
   */
  readonly siteUrl?: string | undefined;
}

/** Escape the five characters that are unsafe in XML text/attribute contexts. */
export function escapeXml(value: string): string {
  return value
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
    .replace(/'/g, '&apos;');
}

/**
 * Build a `sitemap.xml` urlset for the given documents (ordered newest-first).
 * The home page is always listed; its `lastmod` is the most recent document's
 * `updatedAt`, or omitted entirely for an empty site. Each document contributes
 * one `<url>` whose `lastmod` is its own `updatedAt`.
 */
export function buildSitemap(
  documents: readonly Document[],
  options: SitemapOptions = {},
  tags: readonly TagCount[] = [],
): string {
  const base = normalizeSiteUrl(options.siteUrl);

  const homeLastmod = documents[0]?.updatedAt;
  const home = `  <url>
    <loc>${escapeXml(`${base}/`)}</loc>${
      homeLastmod ? `\n    <lastmod>${homeLastmod.toISOString()}</lastmod>` : ''
    }
  </url>`;

  const urls = documents
    .map((doc) => {
      const url = `${base}/${encodeURIComponent(doc.slug)}`;
      return `  <url>
    <loc>${escapeXml(url)}</loc>
    <lastmod>${doc.updatedAt.toISOString()}</lastmod>
  </url>`;
    })
    .join('\n');

  // Tag surfaces: the `/tags` index plus one `/tags/:tag` listing per published
  // tag. No `<lastmod>` — a tag page changes whenever any tagged document does,
  // which a single timestamp can't capture honestly, so it is omitted (valid).
  const tagUrls =
    tags.length === 0
      ? ''
      : [
          `  <url>\n    <loc>${escapeXml(`${base}/tags`)}</loc>\n  </url>`,
          ...tags.map(
            (t) =>
              `  <url>\n    <loc>${escapeXml(`${base}/tags/${encodeURIComponent(t.tag)}`)}</loc>\n  </url>`,
          ),
        ].join('\n');

  const body = [home, urls, tagUrls].filter((part) => part !== '').join('\n');
  return `<?xml version="1.0" encoding="utf-8"?>
<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">
${body}
</urlset>
`;
}

/**
 * Route and render a single `/sitemap.xml` request.
 *
 * Only `GET` (and `HEAD`, handled by the transport) is supported; any other
 * method yields 405 with no body. On success it lists the home page plus every
 * published document.
 */
export async function handleSitemapRequest(
  db: Queryable,
  req: SitemapRequest,
  options: SitemapOptions = {},
): Promise<SitemapResponse> {
  if (req.method !== 'GET' && req.method !== 'HEAD') {
    return { status: 405, contentType: SITEMAP_CONTENT_TYPE, xml: '' };
  }
  const [documents, tags] = await Promise.all([listPublishedDocuments(db), listPublishedTags(db)]);
  return {
    status: 200,
    contentType: SITEMAP_CONTENT_TYPE,
    xml: buildSitemap(documents, options, tags),
  };
}
