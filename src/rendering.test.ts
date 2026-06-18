import { describe, expect, it } from 'vitest';

import { renderDocumentHtml, renderMarkdown } from './rendering.js';

describe('renderMarkdown — formatting preserved', () => {
  it('renders headings', () => {
    expect(renderMarkdown('# Title')).toBe('<h1>Title</h1>\n');
  });

  it('renders bold and italic', () => {
    const html = renderMarkdown('**bold** and *italic*');
    expect(html).toContain('<strong>bold</strong>');
    expect(html).toContain('<em>italic</em>');
  });

  it('renders unordered and ordered lists', () => {
    const ul = renderMarkdown('- one\n- two');
    expect(ul).toContain('<ul>');
    expect(ul).toContain('<li>one</li>');
    const ol = renderMarkdown('1. first\n2. second');
    expect(ol).toContain('<ol>');
    expect(ol).toContain('<li>first</li>');
  });

  it('renders blockquotes', () => {
    expect(renderMarkdown('> quoted')).toContain('<blockquote>');
  });

  it('renders inline and fenced code', () => {
    expect(renderMarkdown('`inline`')).toContain('<code>inline</code>');
    const fenced = renderMarkdown('```ts\nconst x = 1;\n```');
    expect(fenced).toContain('<pre>');
    expect(fenced).toContain('<code');
    expect(fenced).toContain('const x = 1;');
  });

  it('keeps the language class on fenced code blocks', () => {
    const fenced = renderMarkdown('```ts\nconst x = 1;\n```');
    expect(fenced).toContain('language-ts');
  });

  it('renders links with safe href', () => {
    const html = renderMarkdown('[Inkwell](https://example.com)');
    expect(html).toContain('href="https://example.com"');
    expect(html).toContain('Inkwell');
  });

  it('hardens links with a safe rel', () => {
    const html = renderMarkdown('[x](https://example.com)');
    expect(html).toContain('rel="noopener noreferrer nofollow"');
  });

  it('renders images with safe src', () => {
    const html = renderMarkdown('![alt text](https://example.com/a.png)');
    expect(html).toContain('<img');
    expect(html).toContain('src="https://example.com/a.png"');
    expect(html).toContain('alt="alt text"');
  });

  it('renders tables', () => {
    const table = renderMarkdown('| a | b |\n| - | - |\n| 1 | 2 |');
    expect(table).toContain('<table>');
    expect(table).toContain('<th>a</th>');
    expect(table).toContain('<td>1</td>');
  });

  it('preserves safe inline HTML', () => {
    expect(renderMarkdown('<abbr title="HyperText">HTML</abbr>')).toContain(
      '<abbr title="HyperText">HTML</abbr>',
    );
  });

  it('is deterministic', () => {
    const src = '# Same\n\nInput **always** gives same output.';
    expect(renderMarkdown(src)).toBe(renderMarkdown(src));
  });

  it('returns empty string for empty or whitespace-only input', () => {
    expect(renderMarkdown('')).toBe('');
    expect(renderMarkdown('   \n  ')).toBe('');
  });
});

describe('renderMarkdown — sanitization / XSS prevention', () => {
  it('strips script tags and their contents', () => {
    const html = renderMarkdown('Hello\n\n<script>alert(1)</script>');
    expect(html).not.toContain('<script');
    expect(html).not.toContain('alert(1)');
    expect(html).toContain('Hello');
  });

  it('strips inline event handler attributes', () => {
    const html = renderMarkdown('<img src="https://x/y.png" onerror="alert(1)">');
    expect(html).not.toContain('onerror');
    expect(html).not.toContain('alert(1)');
  });

  it('does not create a link from javascript: Markdown link syntax', () => {
    // markdown-it rejects the unsafe URL, so no anchor is produced — the
    // payload is left as inert literal text, never an executable href.
    const html = renderMarkdown('[click](javascript:alert(1))');
    expect(html).not.toContain('<a');
    expect(html).not.toContain('href="javascript:');
  });

  it('does not create an image from javascript: Markdown image syntax', () => {
    const html = renderMarkdown('![x](javascript:alert(1))');
    expect(html).not.toContain('<img');
    expect(html).not.toContain('src="javascript:');
  });

  it('strips javascript: href smuggled through raw HTML anchors', () => {
    const html = renderMarkdown('<a href="javascript:alert(1)">click</a>');
    expect(html).not.toContain('javascript:');
    expect(html).not.toContain('href="javascript:');
    // the visible text survives, just without the dangerous href
    expect(html).toContain('click');
  });

  it('strips iframes', () => {
    const html = renderMarkdown('<iframe src="https://evil.example"></iframe>');
    expect(html).not.toContain('<iframe');
  });

  it('strips style tags and their contents', () => {
    const html = renderMarkdown('<style>body{display:none}</style>text');
    expect(html).not.toContain('<style');
    expect(html).not.toContain('display:none');
    expect(html).toContain('text');
  });

  it('strips event handlers smuggled through raw HTML', () => {
    const html = renderMarkdown('<a href="https://x" onclick="steal()">link</a>');
    expect(html).not.toContain('onclick');
    expect(html).not.toContain('steal()');
    expect(html).toContain('href="https://x"');
  });

  it('strips disallowed form/object tags', () => {
    const html = renderMarkdown('<form action="/x"><input></form><object data="x"></object>');
    expect(html).not.toContain('<form');
    expect(html).not.toContain('<object');
    expect(html).not.toContain('<input');
  });

  it('neutralizes a classic XSS payload while keeping safe text', () => {
    const html = renderMarkdown('Intro\n\n<img src=x onerror=alert(document.cookie)>\n\nOutro');
    expect(html).not.toContain('onerror');
    expect(html).not.toContain('alert(');
    expect(html).toContain('Intro');
    expect(html).toContain('Outro');
  });
});

describe('renderDocumentHtml', () => {
  it('delegates to renderMarkdown', () => {
    const src = '# Doc\n\n<script>bad()</script>safe';
    expect(renderDocumentHtml(src)).toBe(renderMarkdown(src));
  });

  it('produces sanitized HTML suitable for a rendered_html column', () => {
    const html = renderDocumentHtml('## Heading\n\nBody **text**.');
    expect(html).toContain('<h2>Heading</h2>');
    expect(html).toContain('<strong>text</strong>');
    expect(html).not.toContain('<script');
  });
});
