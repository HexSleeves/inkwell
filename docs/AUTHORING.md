# Inkwell — Author Workflow

This document covers the complete author workflow: CLI commands, scoped tokens,
media upload, draft preview links, and bulk import. For API shapes and HTTP
details see [`docs/API.md`](API.md).

---

## Authoring in the browser (CYP-42)

When `INKWELL_BROWSER_LOGIN=true`, Inkwell serves a minimal server-rendered web
editor over the existing `/documents` API — no CLI required:

| Route | Purpose |
|-------|---------|
| `GET /login` | Sign in by pasting an `ink_…` token; mints an httpOnly `inkwell_session` cookie. |
| `GET /editor` | List your documents (draft + published) with edit/view links. |
| `GET /editor/new` | Create a document (title, slug, tags, growth, Markdown body). |
| `GET /editor/{slug}` | Edit, save (optimistic-concurrency `If-Match`), publish/unpublish, and preview. |

The pages are thin HTML shells driven by nonce'd inline scripts that `fetch` the
JSON API; the session cookie carries the token's scopes (capped to
read/write/publish), so the API enforces auth and scope on every action. The
preview pane shows the API's `renderedHtml` — the exact HTML the public page path
renders — so a saved draft previews identically to how it will publish. With the
flag off, none of these routes exist (the public build ships no editor surface).

---

## Configuration

```bash
export INKWELL_API_KEY=ink_abc123_secretvalue   # or the admin key
export INKWELL_API_URL=https://blog.example.com  # or use --server per-command
```

`INKWELL_API_URL` defaults to `http://HOST:PORT`. Override it with `--server <url>`
on any `inkwell author` command.

---

## Document Markdown format

```yaml
---
title: Hello World          # required; max 500 characters
slug: hello-world           # optional; auto-derived from title
status: draft               # advisory; publishing always goes through a CLI command
tags:
  - rust
  - notes
growth: seedling            # seedling | budding | evergreen (default: seedling)
---

# Hello World

Body Markdown lives here. Wikilinks work: [[Other Note]], ![[Embedded Note]].
```

`title` is required. The body is capped at 256 KiB; the CLI rejects oversized
files before sending. `status` in the front matter is ignored by the server —
it is advisory metadata only.

---

## Author CLI commands

### `inkwell author new`

Scaffold a new Markdown file from a template:

```bash
inkwell author new "Hello World" --tag rust --tag notes
# Writes: ./hello-world.md
```

Options:

| Flag | Notes |
|------|-------|
| `--slug <slug>` | Override the auto-derived slug |
| `--tag <tag>` | Add a tag (repeat for multiple) |
| `--growth <stage>` | `seedling` (default), `budding`, `evergreen` |
| `--out <path>` | Write to a specific path |

---

### `inkwell author push`

Create or update a document from a Markdown file. On first push it sends
`POST /documents` (201); on subsequent pushes it sends `PUT /documents/{slug}`
(200):

```bash
inkwell author push hello-world.md

# Target a remote server
inkwell author push hello-world.md --server https://blog.example.com
```

The CLI probes the slug with a `GET` to decide create vs. update. If the server
version differs from your local edit, it warns but proceeds unless you add
`--if-match` to guard against concurrent edits.

---

### `inkwell author publish`

Publish a draft document (makes it publicly visible):

```bash
inkwell author publish hello-world
```

---

### `inkwell author unpublish`

Return a published document to draft status:

```bash
inkwell author unpublish hello-world
```

---

### `inkwell author upload`

Upload a binary image and receive a `/media/{id}` URL to embed in your notes:

```bash
inkwell author upload screenshot.png
# Output: /media/550e8400-e29b-41d4-a716-446655440000
```

Embed the URL in Markdown:

```markdown
![Alt text](/media/550e8400-e29b-41d4-a716-446655440000)
```

Supported formats: `image/png`, `image/jpeg`, `image/gif`, `image/webp`.
Max file size: 5 MiB. Images are stored in PostgreSQL `bytea` (no external
object store required).

The full browser file-picker UI is deferred (CIL-130); use this CLI command
or `POST /media` directly in the meantime.

---

## Scoped tokens

The shared `INKWELL_API_KEY` is the admin key — it carries all scopes. For
day-to-day authoring and AI agent connections, use a scoped token instead.

### Why scoped tokens?

- Least-privilege: an author token can be `write`+`publish` only, never admin.
- Independently revocable: rotate or revoke without changing the admin key.
- Audit trail: every write records which author token performed the action.

### Mint a token

```bash
# Requires the admin key
export INKWELL_API_KEY=<admin-key>
export INKWELL_API_URL=https://blog.example.com

# Laptop authoring token
inkwell author token create --name laptop --scopes read,write,publish
# Prints: ink_abc123_secretvalue
# Store this — it is shown exactly once.
export INKWELL_AUTHOR_TOKEN=ink_<prefix>_<secret>
```

Available scopes:

| Scope | Grants |
|-------|--------|
| `read` | Read your own draft documents |
| `write` | Create, update, delete documents; upload media; mint preview tokens |
| `publish` | Publish and unpublish documents |
| `admin` | All of the above + token management |

### List and revoke tokens

```bash
inkwell author token list          # lists all live tokens (secrets never shown)
inkwell author token revoke <prefix>  # revoke immediately by prefix
```

### MCP agent token

The `inkwell mcp` server uses `INKWELL_API_KEY` — set it to a scoped token,
not the admin key:

```bash
# Mint a read+write token for the AI agent
inkwell author token create --name ai-agent --scopes read,write
# Copy ink_… and set:
export INKWELL_API_KEY=ink_<prefix>_<secret>
inkwell mcp   # starts the MCP server over stdio
```

In your MCP client config (Claude Desktop, etc.), point `INKWELL_API_KEY` to
the scoped token.

---

## Draft preview links

Share a rendered draft with anyone who has the link, before publishing:

```bash
export INKWELL_AUTHOR_TOKEN=ink_<prefix>_<secret>

# Mint a preview token (requires write scope or admin)
curl -X POST https://blog.example.com/documents/my-draft/preview-tokens \
  -H "X-Api-Key: $INKWELL_AUTHOR_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{}'
# Returns: {"token":"pvw_abc123_secret","prefix":"abc123","expiresAt":null,...}

# Share this URL (no auth required):
https://blog.example.com/documents/my-draft/preview?token=pvw_abc123_secret

# List tokens for this document
curl https://blog.example.com/documents/my-draft/preview-tokens \
  -H "X-Api-Key: $INKWELL_AUTHOR_TOKEN"

# Revoke a token
curl -X DELETE https://blog.example.com/documents/my-draft/preview-tokens/abc123 \
  -H "X-Api-Key: $INKWELL_AUTHOR_TOKEN"
```

The preview URL renders the draft as a full HTML page. Any failure (expired,
revoked, unknown document) returns `401` — the draft's existence is never
leaked to anonymous callers.

Add an expiry:

```bash
curl -X POST .../preview-tokens \
  -H "X-Api-Key: $INKWELL_AUTHOR_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"expiresAt":"2026-12-31T23:59:59Z"}'
```

---

## Bulk import

Import a folder of Markdown files in one command:

```bash
inkwell import ./vault/
```

The importer:
- Reads every `.md` file in the given directory (non-recursive by default; use
  `--recursive` for subdirectories).
- Extracts YAML front matter (`title`, `slug`, `tags`, `growth`).
- Creates a draft document for each file via `POST /documents`.
- Skips files that already have a matching slug (idempotent).
- Prints one status line per file (created / skipped / error).

```bash
inkwell import ./vault/ --server https://blog.example.com --recursive
```

After import, use `inkwell author publish <slug>` to make individual notes
public, or the admin `POST /documents/{slug}/publish` in bulk.

---

## Typical workflows

### First-time setup

```bash
# 1. Copy .env.example, set INKWELL_API_KEY and INKWELL_API_URL
cp .env.example .env

# 2. Start the server
docker compose up

# 3. Mint a personal authoring token
inkwell author token create --name laptop --scopes read,write,publish
# Store the returned ink_… token as your INKWELL_API_KEY from now on
```

### Day-to-day authoring

```bash
# New note
inkwell author new "My Note" --tag rust
# Edit ./my-note.md

# Push and publish
inkwell author push my-note.md
inkwell author publish my-note

# Add an image
inkwell author upload photo.jpg
# Paste the /media/… URL into my-note.md, then push again
inkwell author push my-note.md
```

### Share a draft for review

```bash
inkwell author push my-draft.md   # sync local changes
# Mint a preview link
curl -X POST $INKWELL_API_URL/documents/my-draft/preview-tokens \
  -H "X-Api-Key: $INKWELL_API_KEY" -H "Content-Type: application/json" -d '{}' \
  | jq -r '"Preview: \(.token | split("_") | .[0:2] | join("_") | . ) ... share: '"$INKWELL_API_URL"'/documents/my-draft/preview?token=\(.token)"'
```
