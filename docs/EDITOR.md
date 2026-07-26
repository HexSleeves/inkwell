# Inkwell — Authoring Web UI (`/editor`)

Inkwell ships a small server-rendered web editor so an author can write, save,
and publish from a browser without the CLI. It is **off by default** and must be
enabled with `INKWELL_BROWSER_LOGIN=true`.

The pages are thin HTML shells: every data action they perform goes through the
same authenticated `/documents`, `/media`, and publish endpoints the CLI uses, so
the UI grants no capability the API does not already grant. See
[ADR 0010](adr/0010-browser-login.md) for the design rationale.

- CLI authoring: [`docs/AUTHORING.md`](AUTHORING.md)
- API shapes: [`docs/API.md`](API.md)
- Env var reference: [`docs/DEPLOYMENT.md`](DEPLOYMENT.md#environment-variables)

---

## 1. Enable it

```bash
INKWELL_BROWSER_LOGIN=true
```

Routes are registered at startup, so **restart the process** after changing the
flag. With the flag off, none of the routes below exist: `/auth/*` 404s and
`/login`, `/editor…`, `/media/new` fall through to the public `/{slug}` document
route (also a 404 unless a note owns that slug). A public deployment that never
sets the flag ships no editor or cookie surface at all.

| Route | Method | Purpose |
|-------|--------|---------|
| `/login` | `GET` | Sign-in page (paste an `ink_…` token) |
| `/auth/login` | `POST` | Exchange a token for a session cookie |
| `/auth/logout` | `POST` | Delete the session and clear the cookie |
| `/editor` | `GET` | Your documents (drafts + published) |
| `/editor/new` | `GET` | Create a document |
| `/editor/{slug}` | `GET` | Edit, save, publish/unpublish, preview |
| `/media/new` | `GET` | Standalone image upload page |

**Serve it over TLS.** The session cookie is set with `Secure`, so browsers only
send it back over HTTPS. Browsers treat `http://localhost` as a secure context,
so local development works unencrypted; any other host reached over plain HTTP
will silently drop the cookie and bounce you back to `/login`.

On Docker Compose the flag must also be listed in the `app` service's
`environment:` block — it already is; see
[Docker Compose](DEPLOYMENT.md#docker-compose).

---

## 2. Sign in: token for session

The UI adds no new credential. You log in by exchanging an existing scoped
author token for a browser session.

**Mint a token** (needs the admin key — see
[Scoped tokens](AUTHORING.md#scoped-tokens)):

```bash
export INKWELL_API_KEY=<admin-key>
export INKWELL_API_URL=https://blog.example.com

inkwell author token create --name browser --scopes read,write,publish
# Prints ink_<prefix>_<secret> exactly once — copy it.
```

**Then:**

1. Open `https://blog.example.com/login`.
2. Paste the `ink_…` token and submit. The page `POST`s
   `{"token":"ink_…"}` to `/auth/login` as JSON.
3. The server validates the token through the same path as API auth, creates a
   session row (storing only a SHA-256 hash of a freshly minted session token),
   and returns:

   ```text
   Set-Cookie: inkwell_session=<token>; HttpOnly; Secure; SameSite=Strict; Path=/; Max-Age=604800
   ```

4. A successful sign-in lands on the site home page. Go to `/editor` to author.

An invalid, revoked, or malformed token returns `401` and the page shows
"Login failed." Exchanging a token stamps its `last_used_at`, so a browser login
shows up in `inkwell author token list`.

### Session scopes and TTL

- **Session TTL is 7 days** (`Max-Age=604800`), fixed and non-sliding: activity
  does not extend it. When it lapses, API calls return `401` and the editor
  redirects you to `/login` to sign in again.
- A session inherits **exactly** the token's scopes, capped to
  `read`/`write`/`publish`. A `read`-only token yields a read-only session — it
  never gains write. An `admin` token is **downscoped** to `read`+`write`+
  `publish`: admin routes (`/admin/tokens…`) are never reachable from a browser
  session and still require the admin key or an admin token via `x-api-key`.
- Scopes are captured **at login**. Re-minting or re-scoping a token does not
  change a live session — log out and back in with the new token.
- Revoking the originating token blocks new logins (including a login racing the
  revoke), but a session already issued keeps working until it is logged out or
  the TTL lapses — the session row does not point back at the token. Treat the
  7-day TTL as the worst-case exposure window for a browser session, and keep
  `read,write,publish` tokens off shared machines.
- There is no admin endpoint for listing or killing individual browser sessions.
  Expired sessions are rejected at auth time; their rows are left in place for a
  later sweep.
- If a request carries both `x-api-key` and the cookie, the header wins.
- **Log out** with the button on `/editor` (or `/login` when already signed in).
  It deletes the server-side session row and clears the cookie, so the cookie
  cannot be replayed.

---

## 3. The pages

### `/editor` — your documents

A table of every document you can see (`GET /documents?status=all&limit=100`) —
title, status badge, last-updated time — with an **Edit** link per row and, for
published notes, a **View** link to the public page. Also holds the **Log out**
button. The list is capped at the 100 most recently created documents (newest
first) and is not paginated — older notes are reachable by URL
(`/editor/{slug}`) or via the CLI.

Drafts appear only when the session carries `read`; without it the list shows
published documents only (the API resolves draft visibility from the caller's
scopes and author id, not from the page).

### `/editor/new` — create

Fields: **Title** (required), **Slug** (optional — derived from the title),
**Tags** (comma-separated), **Growth** (`seedling`/`budding`/`evergreen`), and
the Markdown **Body**, with the image-insert control above the textarea. Submit
`POST`s to `/documents`; on `201` you are redirected to `/editor/{slug}` to keep
editing. New documents are created as drafts — publishing is a separate action.

### `/editor/{slug}` — edit, publish, preview

- Loads the document with `GET /documents/{slug}`. A missing or unauthorized
  slug surfaces as an in-page message, not a server 404.
- **Save draft** sends `PATCH /documents/{slug}` with the document's current
  version in `If-Match`. If someone else changed the note in the meantime you
  get "This document changed elsewhere. Reload before saving." (`409`) rather
  than a silent overwrite.
- Changing the **Slug** field renames on save; the old slug keeps serving a
  `301` (see [ADR 0011](adr/0011-slug-rename-redirect.md)). The URL in the
  address bar is updated without a reload.
- **Publish** / **Unpublish** is one button that flips to match current state
  (`POST /documents/{slug}/publish` or `…/unpublish`).
- The **Preview** pane renders the API's `renderedHtml` — byte-for-byte what the
  public page path renders, including wikilinks and embeds — and refreshes on
  load and after each save. Unsaved edits are not previewed; save to refresh.

### `/media/new` — standalone upload

A file picker plus drop zone that `POST`s to `/media` and returns the media URL
and a ready-to-paste `![alt](/media/{id})` snippet, each with a copy button. Use
it when you want a URL for a note you are editing elsewhere (CLI, MCP client).
When you are not signed in the page just points you at `/login`.

> The standalone page's client-side size check is a fixed 5 MiB, the default of
> `INKWELL_MEDIA_MAX_BYTES`. If you raise that limit, upload larger files from
> the in-editor control (which reflects the configured value) or the CLI; the
> server-side cap is authoritative either way.

---

## 4. Scopes each action needs

The HTML shells are **not** a second auth surface. They redirect a cookie-less
visitor to `/login` as a convenience, but `/documents` and `/media` authenticate
and scope-check every request independently — a forged or cookie-less call to
the API still gets `401`/`403`.

| Action | Where | Request | Scope required |
|--------|-------|---------|----------------|
| See your drafts in the list | `/editor` | `GET /documents?status=all` | `read` (else published only) |
| Load a document to edit | `/editor/{slug}` | `GET /documents/{slug}` | `read` for drafts |
| Create a document | `/editor/new` | `POST /documents` | `write` |
| Save changes | `/editor/{slug}` | `PATCH /documents/{slug}` | `write` |
| Upload / insert an image | both editor pages, `/media/new` | `POST /media` | `write` |
| Publish or unpublish | `/editor/{slug}` | `POST /documents/{slug}/publish` / `…/unpublish` | `publish` |

A missing scope surfaces in the page's status line — e.g. clicking **Publish**
with a `read,write` session shows `Your session lacks the "publish" scope.`
(HTTP `403`). Mint a token with the scope you need and sign in again.

Writes go through the standard rate limiter (`INKWELL_WRITE_RATE_LIMIT`,
default 60/min per principal), and `POST /auth/login` counts against it too.

---

## 5. Inserting images while writing

Both editor pages carry an **Insert image** control directly above the body
textarea, with three entry points into the same upload:

- pick a file,
- drag an image onto the body textarea,
- paste image data into the body.

Accepted types are **PNG, JPEG, GIF, and WebP** — SVG is rejected because it can
carry script. The control checks the type and the size against
`INKWELL_MEDIA_MAX_BYTES` (default 5 MiB) before uploading; the server enforces
the real cap and answers `413` if exceeded.

On success the uploader inserts `![alt](/media/{id})` at the caret and reports
"Image inserted. Save to keep it." The upload itself is already persisted, but
the Markdown reference is only stored when you save the document.

---

## 6. Troubleshooting

| Symptom | Cause / fix |
|---------|-------------|
| `/editor` or `/login` returns 404 | `INKWELL_BROWSER_LOGIN` is not `true`, or the process was not restarted. On Compose, confirm the var is in the `app` service `environment:` block. |
| Sign-in succeeds but every page bounces back to `/login` | The `Secure` cookie is being dropped — you are on plain HTTP on a non-`localhost` host. Put the app behind TLS. |
| Arriving from an external link lands on `/login` | `SameSite=Strict` withholds the cookie on cross-site navigation. Navigate from within the site or reload the URL. |
| "Login failed. Check your token and try again." | Token is invalid, revoked, or not an `ink_…` author token. Mint a new one. |
| Drafts missing from `/editor` | Session lacks `read`. Re-mint with `read` and sign in again. |
| `Your session lacks the "publish" scope.` | Token had no `publish` scope at login. New scopes need a new login. |
| "This document changed elsewhere. Reload before saving." | Concurrent edit — `If-Match` rejected the save (`409`). Reload and re-apply. |
| Upload reports too large | Above `INKWELL_MEDIA_MAX_BYTES` (server `413`). Raise the limit or shrink the image. |
| `429` while saving or signing in | Write rate limit hit; retry after `Retry-After`, or raise `INKWELL_WRITE_RATE_LIMIT`. |
| Signed out unexpectedly after about a week | The 7-day session TTL expired. Sign in again. |
