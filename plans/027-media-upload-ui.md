# 027 — Media upload UI (file-picker / drag-drop)

Status: **Backlog** (follow-up to #28, which shipped the API)

## Context

#28 shipped the media API only — `POST /media` (raw image bytes + `Content-Type`
header, `write` scope, ≤5 MiB, png/jpeg/gif/webp) and `GET /media/{id}`. There is
no browser UI: uploading today means `curl --data-binary`, the CLI, or MCP. Authors
can embed an uploaded image with Markdown `![alt](/media/<uuid>)`.

## Goal

A small browser UI to upload an image and get its `/media/{id}` URL without leaving
the page:

- A file-picker + drag-drop drop-zone that reads the selected file and `POST`s its
  raw bytes to `/media` with the file's MIME type as `Content-Type`.
- On `201`, show a copy-able `/media/{id}` URL and a ready-to-paste
  `![](...)` Markdown snippet; on error, surface the server message (413/400/401/403).
- Client-side guards mirroring the server: reject > 5 MiB and non-allowlisted types
  before the request, so the user gets instant feedback.

## Open questions / dependencies

- **Auth in the browser.** `POST /media` needs a `write`-scoped credential. The
  natural pairing is the flag-gated browser session from #29 (ADR 0010): the upload
  page rides the session cookie instead of an `x-api-key` header. So this card is
  effectively gated on turning `INKWELL_BROWSER_LOGIN` on and shipping the login UI.
- Where the page lives (standalone `/media/new`, or an editor affordance) — decide
  alongside any future authoring UI.

## Out of scope

- Image processing/resizing/thumbnails, gallery/listing, deletion UI — separate
  cards if wanted.

## Acceptance

- Drag-drop or pick a PNG/JPEG/GIF/WebP ≤ 5 MiB → see the `/media/{id}` URL and a
  copy button; the URL renders when pasted into a note.
- Oversized / wrong-type / unauthenticated attempts show the matching server error.
