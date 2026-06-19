/**
 * Markdown -> sanitized HTML rendering pipeline.
 *
 * Authors write Markdown; Inkwell renders it to clean, safe HTML for public
 * web pages. The pipeline is two stages:
 *
 *   1. Parse Markdown to HTML with `markdown-it` (a well-supported,
 *      CommonMark-compliant parser). Raw inline HTML is *allowed* at this
 *      stage so authors can drop in the occasional `<figure>` or `<abbr>`.
 *      Fenced code blocks with a language hint (```ts) are syntax-highlighted
 *      server-side with `highlight.js`, which emits `<span class="hljs-…">`
 *      tokens that the inlined page stylesheet (see `src/pages.ts`) colors.
 *   2. ...but the result is then run through `sanitize-html` with a strict
 *      allowlist, so any XSS vector (script tags, event handlers,
 *      `javascript:` URLs, etc.) is stripped before it can reach a reader.
 *
 * Allowlist-based sanitization is the safe default: anything not explicitly
 * permitted is removed. The rendered output is what gets persisted to a
 * document's `rendered_html` column on create/update (see `renderDocumentHtml`).
 */

import hljs from 'highlight.js';
import MarkdownIt from 'markdown-it';
import sanitizeHtml from 'sanitize-html';

/**
 * Syntax-highlight a fenced code block's contents server-side.
 *
 * markdown-it calls this for every fenced block, passing the raw code and the
 * language hint from the fence info string (```ts -> `lang === 'ts'`). When the
 * language is one highlight.js recognizes we return the highlighted *inner*
 * HTML — a run of `<span class="hljs-…">` tokens; otherwise we fall back to the
 * plain escaped source so unknown/absent languages still render safely.
 *
 * We return only the inner markup (not a wrapping `<pre><code>`), so markdown-it
 * keeps emitting its own `<pre><code class="language-…">` wrapper. We prepend
 * the `hljs` class to that wrapper via {@link highlightedFenceClass} below so
 * the theme's `code.hljs` background/foreground rules apply.
 */
function highlightCode(code: string, lang: string): string {
  if (lang && hljs.getLanguage(lang)) {
    try {
      return hljs.highlight(code, { language: lang, ignoreIllegals: true }).value;
    } catch {
      // Fall through to the escaped-plaintext path on any highlighter error.
    }
  }
  // No (recognized) language: escape so raw `<`, `&`, quotes in the source can
  // never be parsed as markup. markdown-it would otherwise escape this for us,
  // but since we're supplying the highlighted string it expects pre-escaped output.
  return md.utils.escapeHtml(code);
}

const md = new MarkdownIt({
  html: true, // allow raw HTML in source; the sanitizer is the safety net
  linkify: true, // autolink bare URLs
  typographer: true, // smart quotes / dashes
  breaks: false, // a single newline is not a hard break (CommonMark)
  highlight: highlightCode,
});

/**
 * Override the fenced-block renderer so the `<code>` element carries both the
 * `hljs` class (which highlight.js themes style) and the original
 * `language-<lang>` hint. markdown-it's default rule only emits `language-<lang>`;
 * highlight.js theme CSS keys off `.hljs`, so without this the token spans would
 * be colored but the block's own background/foreground would not.
 */
md.renderer.rules.fence = (tokens, idx, options) => {
  const token = tokens[idx];
  if (!token) return '';
  const info = token.info ? md.utils.unescapeAll(token.info).trim() : '';
  const lang = info.split(/\s+/, 1)[0] ?? '';
  const highlighted = options.highlight?.(token.content, lang, '') ?? md.utils.escapeHtml(token.content);
  const langClass = lang ? ` language-${md.utils.escapeHtml(lang)}` : '';
  return `<pre><code class="hljs${langClass}">${highlighted}</code></pre>\n`;
};

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
    // `<code class="hljs language-ts">` — keep it for theme styling.
    code: ['class'],
    pre: ['class'],
    // highlight.js token spans, e.g. `<span class="hljs-keyword">`. Only the
    // class attribute is permitted, and class values are otherwise inert.
    span: ['class'],
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
