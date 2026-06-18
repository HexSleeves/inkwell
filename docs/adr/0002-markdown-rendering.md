# ADR 0002: Markdown rendering and HTML sanitization

- **Status:** Accepted
- **Date:** 2026-06-18

## Context

Inkwell renders author-supplied Markdown to HTML for public web pages. Because
that content is published and served to readers, the rendered HTML is an XSS
surface: a malicious or careless author must not be able to inject executable
script, event handlers, or dangerous URLs into a public page. We need a
well-supported Markdown parser and a robust, allowlist-based sanitizer.

## Decision

- **Parser: [`markdown-it`](https://github.com/markdown-it/markdown-it).**
  CommonMark-compliant, widely used, actively maintained, pluggable, and pure
  Node (no DOM dependency). Configured with `linkify` and `typographer` on, and
  `html: true` so authors can use the occasional bit of safe inline HTML — the
  sanitizer, not the parser, is the security boundary.
- **Sanitizer: [`sanitize-html`](https://github.com/apostrophecms/sanitize-html).**
  Server-side, allowlist-based, and dependency-light (no `jsdom`/browser needed),
  which suits a small self-hostable core. We chose it over DOMPurify
  specifically to avoid pulling a full DOM implementation into the server.
- **Allowlist, not denylist.** Only an explicit set of tags/attributes is
  permitted; everything else is discarded. URL schemes are limited to
  `http`/`https`/`mailto` (images: `http`/`https` only), so `javascript:`,
  `data:` links, and `vbscript:` are dropped. `<script>`, `<style>`,
  `<iframe>`, `<object>`, `<form>`, and `on*` event handlers are stripped.
- **Link hardening.** Every emitted `<a>` gets `rel="noopener noreferrer
nofollow"`.
- **Single seam.** `renderDocumentHtml(markdownBody)` is the one function the
  document create/update path calls to populate `rendered_html`, so rendering
  and sanitization stay centralized and consistently applied.

## Consequences

- Rendering is pure and deterministic — easy to unit-test and cache. The stored
  `rendered_html` can be served directly without per-request sanitization.
- The allowlist is intentionally conservative; expanding it (e.g. to support
  embeds, syntax-highlight markup, or footnotes) is a deliberate, reviewable
  change in `src/rendering.ts`, with tests, rather than an accidental hole.
- Sanitization runs at write time (on create/update). If the allowlist later
  tightens for security reasons, existing documents must be re-rendered.
