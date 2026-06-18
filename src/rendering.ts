/**
 * Markdown -> sanitized HTML rendering pipeline.
 *
 * Authors write Markdown; Inkwell renders it to clean, safe HTML for public
 * web pages. The pipeline is two stages:
 *
 *   1. Parse Markdown to HTML with `markdown-it` (a well-supported,
 *      CommonMark-compliant parser). Raw inline HTML is *allowed* at this
 *      stage so authors can drop in the occasional `<figure>` or `<abbr>`...
 *   2. ...but the result is then run through `sanitize-html` with a strict
 *      allowlist, so any XSS vector (script tags, event handlers,
 *      `javascript:` URLs, etc.) is stripped before it can reach a reader.
 *
 * Allowlist-based sanitization is the safe default: anything not explicitly
 * permitted is removed. The rendered output is what gets persisted to a
 * document's `rendered_html` column on create/update (see `renderDocumentHtml`).
 */

import MarkdownIt from 'markdown-it';
import sanitizeHtml from 'sanitize-html';

const md = new MarkdownIt({
  html: true, // allow raw HTML in source; the sanitizer is the safety net
  linkify: true, // autolink bare URLs
  typographer: true, // smart quotes / dashes
  breaks: false, // a single newline is not a hard break (CommonMark)
});

/**
 * Tags an author is allowed to emit. Deliberately covers common prose,
 * formatting, lists, links, images, code, tables, and a few semantic
 * grouping elements — and nothing that can execute script or embed
 * arbitrary content (`script`, `iframe`, `object`, `style`, `form`, ...).
 */
const ALLOWED_TAGS: string[] = [
  // headings
  'h1',
  'h2',
  'h3',
  'h4',
  'h5',
  'h6',
  // prose
  'p',
  'blockquote',
  'hr',
  'br',
  // inline formatting
  'a',
  'em',
  'strong',
  'del',
  's',
  'sub',
  'sup',
  'mark',
  'abbr',
  'small',
  // code
  'code',
  'pre',
  'kbd',
  'samp',
  // lists
  'ul',
  'ol',
  'li',
  // tables
  'table',
  'thead',
  'tbody',
  'tfoot',
  'tr',
  'th',
  'td',
  // media + grouping
  'img',
  'figure',
  'figcaption',
  'div',
  'span',
];

const SANITIZE_OPTIONS: sanitizeHtml.IOptions = {
  allowedTags: ALLOWED_TAGS,
  allowedAttributes: {
    a: ['href', 'title', 'rel'],
    img: ['src', 'alt', 'title'],
    abbr: ['title'],
    // language hint emitted by markdown-it fenced code blocks, e.g.
    // `<code class="language-ts">` — keep it for downstream highlighting.
    code: ['class'],
    pre: ['class'],
    // common alignment attribute markdown-it emits on table cells
    th: ['align'],
    td: ['align'],
  },
  // Only safe URL schemes. `javascript:`, `data:` (for links), `vbscript:`
  // etc. are not on the list, so they are dropped.
  allowedSchemes: ['http', 'https', 'mailto'],
  allowedSchemesByTag: {
    // images may use http(s) only — no `data:` blobs by default
    img: ['http', 'https'],
  },
  // Strip the *contents* of disallowed script-bearing tags entirely, rather
  // than leaving the inner text behind as readable prose.
  nonTextTags: ['style', 'script', 'textarea', 'noscript'],
  // Harden every emitted link: external-safe rel, no referrer leakage.
  transformTags: {
    a: sanitizeHtml.simpleTransform('a', {
      rel: 'noopener noreferrer nofollow',
    }),
  },
  disallowedTagsMode: 'discard',
};

/**
 * Render a Markdown string to sanitized, XSS-safe HTML.
 *
 * Pure and deterministic: the same input always yields the same output.
 * Returns an empty string for empty/whitespace-only input.
 */
export function renderMarkdown(markdown: string): string {
  if (!markdown || markdown.trim() === '') {
    return '';
  }
  const rawHtml = md.render(markdown);
  return sanitizeHtml(rawHtml, SANITIZE_OPTIONS);
}

/**
 * Produce the value for a document's `rendered_html` field from its Markdown
 * body. Thin wrapper over {@link renderMarkdown} that the document
 * create/update path calls so rendering stays centralized in one place.
 */
export function renderDocumentHtml(markdownBody: string): string {
  return renderMarkdown(markdownBody);
}
