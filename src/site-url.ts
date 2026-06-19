/**
 * Shared helper for resolving the public site origin.
 *
 * Discovery surfaces (HTML pages, the Atom feed, the sitemap) all need absolute
 * URLs built from a single configured origin so canonical links, OpenGraph
 * `og:url`, sitemap `<loc>` entries, and JSON-LD identifiers agree. The origin
 * is read from `INKWELL_SITE_URL`; when unset it falls back to a localhost
 * origin so output stays valid and deterministic in development and tests.
 *
 * (`src/feed.ts` predates this module and keeps its own copy of the same logic;
 * new code shares this one.)
 */

/** Default site origin used when `INKWELL_SITE_URL` is not configured. */
export const DEFAULT_SITE_URL = 'http://localhost';

/**
 * Normalize a configured site URL to an absolute origin with no trailing slash.
 * A trailing slash is tolerated and trimmed so `https://x/` and `https://x`
 * produce identical absolute URLs downstream.
 */
export function normalizeSiteUrl(siteUrl: string | undefined): string {
  const base = (siteUrl ?? '').trim() || DEFAULT_SITE_URL;
  return base.replace(/\/+$/, '');
}
