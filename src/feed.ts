/**
 * Atom 1.0 syndication feed for published documents.
 *
 * Where `src/pages.ts` renders the human-facing HTML surface and `src/api.ts`
 * speaks JSON, this module emits the machine-readable feed that readers and
 * aggregators subscribe to. Like those modules it is framework-free:
 * {@link handleFeedRequest} takes a normalized {@link FeedRequest} plus a
 * {@link Queryable} and returns a {@link FeedResponse} (status + content-type +
 * XML body), so it is integration-tested directly against the data-access layer
 * without binding a socket. The thin `node:http` adapter in `src/server.ts`
 * dispatches `GET /feed.xml` here.
 *
 * The XML is built by hand from template literals — no XML dependency. Every
 * value interpolated into the template is text (titles, slugs, the rendered
 * HTML body) and is XML-escaped via {@link escapeXml} before insertion. The
 * document's `renderedHtml` is already sanitized at write time by the rendering
 * pipeline (see `src/rendering.ts`); here it is additionally entity-escaped so
 * it travels safely as the text content of an Atom `<content type="html">`
 * element rather than as live child markup.
 */

import { listPublishedDocuments, type Document } from './db/documents.js';
import type { Queryable } from './db/pool.js';

/** Number of most-recent documents the feed advertises. */
export const FEED_MAX_ENTRIES = 20;

/** Default site origin used when {@link FeedOptions.siteUrl} is not configured. */
const DEFAULT_SITE_URL = 'http://localhost';

/** A normalized inbound feed request, independent of any HTTP framework. */
export interface FeedRequest {
  /** Upper-case HTTP method, e.g. `GET`. */
  readonly method: string;
}

/** A feed response to be written by the transport adapter. */
export interface FeedResponse {
  readonly status: number;
  /** Value for the `Content-Type` header. */
  readonly contentType: string;
  /** The serialized feed (an Atom 1.0 XML document), or empty for non-200s. */
  readonly xml: string;
}

/** Server-side options for {@link buildAtomFeed} / {@link handleFeedRequest}. */
export interface FeedOptions {
  /**
   * The public origin the site is served from, e.g. `https://blog.example.com`.
   * Used to build absolute IRIs for the feed id, the self link, and per-entry
   * links/ids. Read from `INKWELL_SITE_URL`; falls back to {@link DEFAULT_SITE_URL}.
   * A trailing slash is tolerated and trimmed.
   */
  readonly siteUrl?: string | undefined;
}

/** The exact Content-Type for an Atom feed. */
export const ATOM_CONTENT_TYPE = 'application/atom+xml; charset=utf-8';

/** Escape the five characters that are unsafe in XML text/attribute contexts. */
export function escapeXml(value: string): string {
  return value
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
    .replace(/'/g, '&apos;');
}

/** Normalize a configured site URL to an absolute origin with no trailing slash. */
function normalizeSiteUrl(siteUrl: string | undefined): string {
  const base = (siteUrl ?? '').trim() || DEFAULT_SITE_URL;
  return base.replace(/\/+$/, '');
}

/**
 * Build an Atom 1.0 feed document for the given documents (already ordered
 * newest-first). The feed's `<updated>` is the most recent document's
 * `updatedAt`; for an empty feed it falls back to the Unix epoch so the output
 * stays a valid, deterministic Atom document.
 */
export function buildAtomFeed(documents: readonly Document[], options: FeedOptions = {}): string {
  const base = normalizeSiteUrl(options.siteUrl);
  const selfHref = `${base}/feed.xml`;
  // RFC3339: Date#toISOString yields e.g. 2026-06-18T12:00:00.000Z.
  const feedUpdated = (documents[0]?.updatedAt ?? new Date(0)).toISOString();

  const entries = documents
    .map((doc) => {
      const url = `${base}/${encodeURIComponent(doc.slug)}`;
      return `  <entry>
    <title>${escapeXml(doc.title)}</title>
    <id>${escapeXml(url)}</id>
    <link rel="alternate" type="text/html" href="${escapeXml(url)}" />
    <published>${doc.createdAt.toISOString()}</published>
    <updated>${doc.updatedAt.toISOString()}</updated>
    <content type="html">${escapeXml(doc.renderedHtml)}</content>
  </entry>`;
    })
    .join('\n');

  return `<?xml version="1.0" encoding="utf-8"?>
<feed xmlns="http://www.w3.org/2005/Atom">
  <title>Inkwell</title>
  <id>${escapeXml(`${base}/`)}</id>
  <updated>${feedUpdated}</updated>
  <link rel="self" type="application/atom+xml" href="${escapeXml(selfHref)}" />
  <link rel="alternate" type="text/html" href="${escapeXml(`${base}/`)}" />
${entries}
</feed>
`;
}

/**
 * Route and render a single `/feed.xml` request.
 *
 * Only `GET` (and `HEAD`, handled by the transport) is supported; any other
 * method yields 405 with no body. On success it serves the {@link FEED_MAX_ENTRIES}
 * most recent documents as an Atom 1.0 feed.
 */
export async function handleFeedRequest(
  db: Queryable,
  req: FeedRequest,
  options: FeedOptions = {},
): Promise<FeedResponse> {
  if (req.method !== 'GET' && req.method !== 'HEAD') {
    return { status: 405, contentType: ATOM_CONTENT_TYPE, xml: '' };
  }
  const documents = await listPublishedDocuments(db, { limit: FEED_MAX_ENTRIES });
  return { status: 200, contentType: ATOM_CONTENT_TYPE, xml: buildAtomFeed(documents, options) };
}
