# Inkwell — Site Configuration

This document covers the environment variables that control Inkwell's public-facing
identity: title, description, author, custom CSS, and canonical URL. All are
optional — the server starts and works correctly without any of them, using
sensible defaults.

---

## Site metadata variables (CIL-131)

These four variables configure how Inkwell presents itself on every public HTML
page, the Atom feed, and Open Graph metadata. None require a restart of
Postgres; they are read at server startup.

| Variable | Default | Where it appears |
|----------|---------|-----------------|
| `INKWELL_SITE_TITLE` | `Inkwell` | Site header, `<title>`, `og:site_name`, Atom feed title |
| `INKWELL_SITE_DESCRIPTION` | _(none)_ | Index page `<meta name="description">`, Atom feed subtitle |
| `INKWELL_SITE_AUTHOR` | _(none)_ | Atom `<author>`, JSON-LD `author` field |
| `INKWELL_CUSTOM_CSS_URL` | _(none)_ | Extra `<link rel="stylesheet">` on every public page |

### `INKWELL_SITE_TITLE`

The brand name for the site. Shown in the header, browser tab title, and RSS
feed. Defaults to `"Inkwell"`.

```bash
INKWELL_SITE_TITLE="My Digital Garden"
```

### `INKWELL_SITE_DESCRIPTION`

A short description of the site. Used in the index page `<meta name="description">`
tag for SEO and as the Atom feed subtitle. Keep it under 160 characters for
search engines.

```bash
INKWELL_SITE_DESCRIPTION="My notes on Rust, systems programming, and software design"
```

### `INKWELL_SITE_AUTHOR`

The default author name. Inserted into the Atom feed `<author>` element and
JSON-LD `author` field when no per-document author is set.

```bash
INKWELL_SITE_AUTHOR="Jane Doe"
```

### `INKWELL_CUSTOM_CSS_URL`

URL of an extra stylesheet injected via `<link rel="stylesheet">` after the
built-in Botanical Soft styles on every public HTML page. Lets you override
colors, typography, or layout without modifying source code.

```bash
# Relative URL (served by the same Inkwell process or a reverse proxy)
INKWELL_CUSTOM_CSS_URL=/assets/my-theme.css

# Absolute URL (CDN, external host)
INKWELL_CUSTOM_CSS_URL=https://cdn.example.com/inkwell-theme.css
```

CSS custom properties you can override without touching the cascade:

```css
/* Example custom CSS */
:root {
  --brand: rgb(47 93 69);    /* forest green headings and active links */
  --link:  rgb(197 107 71);  /* warm clay links */
}
.site-brand { font-size: 1.25rem; }
```

---

## Canonical URL variable

| Variable | Default | Where it appears |
|----------|---------|-----------------|
| `INKWELL_SITE_URL` | _(none)_ | RSS `<link>`, sitemap URLs, Open Graph `og:url`, JSON-LD `url` |

`INKWELL_SITE_URL` should be the full public URL with protocol and no trailing
slash (e.g. `https://blog.example.com`). Without it, absolute URLs in the feed
and sitemap fall back to `http://localhost`, which is wrong in production.

```bash
INKWELL_SITE_URL=https://blog.example.com
```

On Railway, set this to your Railway public URL or custom domain.

---

## Example `.env` for a named garden

```bash
DATABASE_URL=postgres://inkwell:secret@db:5432/inkwell
INKWELL_API_KEY=<openssl rand -hex 32>
INKWELL_SITE_URL=https://garden.example.com
INKWELL_SITE_TITLE="Alice's Digital Garden"
INKWELL_SITE_DESCRIPTION="Notes on botany, systems, and slow knowledge."
INKWELL_SITE_AUTHOR="Alice"
INKWELL_CUSTOM_CSS_URL=https://garden.example.com/assets/theme.css
```

---

## Notes

- All variables are read at startup from the process environment or `.env`.
  A change requires restarting the server.
- Secrets (`INKWELL_API_KEY`, `DATABASE_URL`) must be protected — see
  [`docs/DEPLOYMENT.md`](DEPLOYMENT.md) § Secret Handling.
- The full variable list including AI keys, rate limiting, and proxy settings
  is in [`docs/DEPLOYMENT.md`](DEPLOYMENT.md) § Environment Variables.
