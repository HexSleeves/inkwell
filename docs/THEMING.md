# Theming

Inkwell renders its public pages from hand-written Rust. A **theme** lets you
replace parts of that markup — the page shell, the header/nav/footer, the CSS,
and the index/document page bodies — without forking the binary.

```bash
INKWELL_THEME_DIR=/srv/inkwell/themes/plainpaper
```

Unset the variable and Inkwell renders exactly as it always has, byte for byte.
Delete your theme directory and the site goes back to the built-in look. Nothing
else changes.

A ready-to-copy theme ships in [`examples/themes/plainpaper`](https://github.com/HexSleeves/inkwell/tree/main/examples/themes/plainpaper):

```bash
INKWELL_THEME_DIR=examples/themes/plainpaper cargo run
```

## Scope: public pages only

Themes apply to the **public** surfaces:

| Themed | Not themed |
| --- | --- |
| index / `/page/N`, document pages, `/notes`, `/tags`, `/graph`, `/search`, `/archive` | `/editor`, `/auth/login`, `/settings`, `/media` |

Authenticated and admin surfaces deliberately keep the built-in look, so a theme
can never leave you unable to log in and fix it. The `/404` page is also always
built-in.

## Directory layout

A theme is one **flat** directory of files. Every file is optional; ship only the
slots you want to change. Subdirectories are rejected (Inkwell serves no files
out of a theme directory — see [Static assets](#static-assets)).

```
my-theme/
├── layout.html     whole-page shell
├── head.html       extra markup appended inside <head>
├── header.html     the <header> block
├── nav.html        the <nav> block
├── footer.html     the <footer> block
├── styles.css      REPLACES the built-in stylesheet
├── extra.css       APPENDED after the stylesheet
├── index.html      body of the index / paginated pages
├── document.html   body of a single document page
└── README.md       ignored (as are LICENSE* and CHANGELOG.md)
```

Any other file name is a startup error — that way a typo like `style.css`
tells you instead of silently doing nothing.

Every slot you omit falls back to the built-in markup, independently. A theme
containing nothing but `extra.css` is valid and complete.

## Templates

A slot file is plain text with `{{ variable }}` placeholders. That is the whole
template language: no loops, no conditionals, no includes. Substitution is
literal; anything that is not a `{{ name }}` placeholder passes through
untouched, so CSS braces and JS in a `layout.html` are fine.

An unknown variable, or an unclosed `{{`, is a startup error that names the file,
the variable, and what *is* available in that slot.

### Available variables

**`layout.html`** — the full shell. Everything else is handed to it pre-rendered.

| Variable | Contents |
| --- | --- |
| `{{ lang }}` | `en` |
| `{{ head }}` | The built-in `<head>` contents: `<title>`, canonical, OpenGraph/Twitter, JSON-LD, the font preload, the `/assets/site.css` link, `INKWELL_CUSTOM_CSS_URL`, and your `head.html` |
| `{{ header }}` | The rendered `header.html` (or built-in header) |
| `{{ nav }}` | The rendered `nav.html` (or built-in nav) |
| `{{ footer }}` | The rendered `footer.html` (or built-in footer) |
| `{{ main }}` | The page body for this route |
| `{{ main_class }}` | `site-main`, plus ` wide-layout` on the pages that opt into a wider column |
| `{{ site_name }}` | `INKWELL_SITE_TITLE`, HTML-escaped |
| `{{ styles_url }}` | `/assets/site.css` |
| `{{ botanical_band }}` | The built-in decorative footer SVG. Omit it and it is gone |

**`head.html`** — appended after the built-in `<head>` tags, never instead of
them: canonical URLs, OpenGraph, and JSON-LD are correctness, not decoration.
Use `layout.html` if you need full control over `<head>` placement.
Variables: `{{ site_name }}`, `{{ title }}`, `{{ canonical_url }}`,
`{{ styles_url }}`.

**`header.html`** — `{{ site_name }}`, `{{ nav }}`.

**`footer.html`** — `{{ site_name }}`.

**`nav.html`** — `{{ site_name }}`, `{{ nav_current }}` (one of `dashboard`,
`notes`, `tags`, `graph`, `settings`, or empty), and one `active_*` variable per
nav item: `{{ active_dashboard }}`, `{{ active_notes }}`, `{{ active_tags }}`,
`{{ active_graph }}`, `{{ active_settings }}`. Each expands to
` site-nav--active` on the current page and to nothing otherwise, which is how
you mark the current item without needing template conditionals:

```html
<nav class="my-nav" aria-label="Main navigation">
  <a class="my-link{{ active_dashboard }}" href="/">Home</a>
  <a class="my-link{{ active_notes }}" href="/notes">Notes</a>
</nav>
```

**`index.html`** — `{{ site_name }}`, `{{ site_description }}`,
`{{ documents }}` (Inkwell's `<ul class="index">` list, or the empty-state
paragraph), `{{ pager }}`, `{{ page }}`, `{{ total_pages }}`.

**`document.html`** — `{{ site_name }}`, `{{ title }}`, `{{ slug }}`,
`{{ url }}`, `{{ body_html }}` (the sanitized document body),
`{{ meta_line }}` (the built-in `<div class="meta">` with published/updated/
growth), `{{ published }}`, `{{ updated }}`, `{{ growth }}`, `{{ tags }}`,
`{{ backlinks }}` (the "Linked from" panel, empty when there are none),
`{{ doc_nav }}` (prev/next, empty when there are neither).

### Styling Inkwell-rendered fragments

`{{ documents }}`, `{{ tags }}`, `{{ backlinks }}`, `{{ pager }}` and friends are
rendered by Inkwell with stable class names, so a `styles.css` can restyle them:
`ul.index`, `a.title`, `.excerpt`, `.meta`, `ul.tags`, `.growth`, `.backlinks`,
`ul.backlinks-list`, `a.backlink`, `.backlink-context`, `nav.pager`,
`nav.doc-nav`, `.spacer`, `.empty`.

### CSS

`styles.css` replaces the built-in stylesheet served at `/assets/site.css`;
`extra.css` is appended after whichever sheet is in play. So:

- tweak the default look → ship only `extra.css`
- start from scratch → ship `styles.css` (and optionally `extra.css`)

`INKWELL_CUSTOM_CSS_URL` still works and still loads after both, if you would
rather host CSS elsewhere.

### Static assets

Inkwell does not serve files out of a theme directory — a theme is templates, not
a webroot. Reference images and fonts by absolute URL, from `/media/...` (see
[Deployment](DEPLOYMENT.md)), or from a CDN. Note that the default Content
Security Policy is same-origin, so cross-origin assets need a CSP you control.

## Failure model

A theme is read **once, at startup**, and every problem is a startup error with
the offending file named — never a half-broken live page and never a panic
mid-request:

- the directory is missing or is not a directory
- it contains a subdirectory
- it contains an unrecognized file name
- a file is not valid UTF-8
- a `{{` is never closed, or a placeholder name is not a plain lowercase
  identifier
- a slot uses a variable that slot does not receive
- the directory contains no recognized slot files at all

Editing a theme therefore requires a restart. That is the trade for having a
broken theme fail in your terminal instead of in a reader's browser.

## Untrusted input is still sanitized

Document bodies are sanitized by Inkwell before a theme ever sees them: a theme
receives `{{ body_html }}`, which is post-`sanitize` HTML, and there is no slot
variable that carries raw markdown or pre-sanitize markup. Titles, slugs, tags,
and the site name arrive HTML-escaped. Adding a theme cannot widen what a note
author is allowed to emit.

Theme markup itself is **trusted operator content** and is emitted raw — exactly
like the built-in markup it replaces. Anyone who can write your theme directory
can already run your server, so this is not a new privilege. Treat a theme like
any other config: don't install one you haven't read.

## Why this design

Template engine, or a plain substitution seam? We took the seam:

- **The requirement is composition, not computation.** Every slot needs to place
  a handful of pre-rendered fragments. `{{ name }}` covers that; loops and
  conditionals would only exist to be misused.
- **No new dependency.** minijinja/tera/askama each bring a compile-time or
  runtime cost, an error-reporting model to map onto ours, and a sandbox question
  to answer. Nine files and ~250 lines answer it instead.
- **A smaller security surface.** Because slots only ever receive fragments
  Inkwell already escaped or sanitized, `rendering::sanitize` stays the single
  gate on untrusted markup. A general template engine with document objects in
  scope would make that a policy question rather than a structural fact.
- **Deleting it restores the status quo exactly.** The built-in renderer is still
  the built-in renderer; a theme is an `Option<&Theme>` consulted slot by slot.
  A regression test asserts byte-identical output when no theme is configured.

If a future need genuinely requires iteration or conditionals in a template, the
slot boundaries here are the natural place to drop an engine in behind — the
variables each slot receives are already the interface.
