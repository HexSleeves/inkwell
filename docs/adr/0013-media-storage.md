# ADR 0013 — Media upload and hosting

- Status: Accepted
- Date: 2026-07-25
- Ticket: CYP-45 (parent CYP-44)

## Context

The authoring web UI (CYP-42) shipped with no image path: an author could only
reference images already hosted somewhere else. Inkwell needed first-class media
so a note can carry its own images, from the browser, without a separate upload
tool.

Migration 0019 already had a `media` table that stored blobs inline in a `bytea`
column, with a raw-bytes `POST /media` and a public `GET /media/{id}`. That works
but bakes the storage target into the schema: the bytes are in Postgres, so every
backup, replica, and WAL segment carries them, and an object store can never be
adopted without changing how rows are read.

## Decision

### 1. Storage behind a trait, local filesystem by default

`crate::media::MediaStore` is a three-method async trait (`put`, `get`,
`delete`) over opaque content-addressed keys. Two backends ship:

- **`local`** (default) — bytes under `INKWELL_MEDIA_DIR` (`./data/media`).
  Writes are atomic (temp file in the same shard directory, then `rename`), so a
  crash mid-upload cannot leave a truncated blob to be served as a valid image.
- **`postgres`** — bytes in the new `media_blobs` table. For platforms with an
  ephemeral filesystem (e.g. Railway) where mounting a volume is impractical.
  Costs DB size, WAL traffic, and backup weight.

`INKWELL_MEDIA_BACKEND` selects one; an unrecognised value fails startup rather
than defaulting, because silently picking the wrong backend strands uploads.

Adding S3/R2 later is a new `impl MediaStore` plus one arm in
`build_media_store` — no API, schema, or URL change. That is the reason the trait
takes keys rather than ids: keys are the only backend-visible concept, and they
are generated, never client-supplied.

### 2. Content-addressed naming

A blob's key is derived **only** from the SHA-256 of its bytes plus a fixed
extension from the MIME allowlist:

```text
<hex[0..2]>/<hex[2..4]>/<hex>.<ext>      e.g. 3f/a9/3fa9…c1.png
```

Consequences we wanted:

- **Traversal-proof by construction.** No filename, header text, or other
  client-controlled string reaches the path, so there is no unescaping step to
  get wrong. Both backends additionally refuse any key failing
  `is_valid_storage_key`, and migration 0025 adds a `CHECK` constraint with the
  same pattern so even ad-hoc SQL cannot store a hostile key.
- **Collision-free.** Identical bytes always map to one key; different bytes
  effectively never collide.
- **Idempotent uploads.** Re-uploading the same bytes as the same owner returns
  the existing row with `200` instead of `201` and does not duplicate the blob.
- **Free strong ETags.** The digest is stored in `media.checksum_sha256` and
  served as the `ETag`, so conditional requests are answered without reading the
  blob at all.
- **Sane directory fan-out** on the filesystem backend (two shard levels).

Metadata (`id`, `filename`, `content_type`, `byte_size`, `checksum_sha256`,
`storage_key`, `storage_backend`, `owner_id`, `created_at`) stays in Postgres.
The public URL remains the opaque `/media/{uuid}` — the key is never exposed, so
switching backends or renaming shards is invisible to published pages.

### 3. Upload validation

- `write` scope required (ADR 0009 scoped tokens; the browser session works too).
- MIME allowlist: `image/png`, `image/jpeg`, `image/gif`, `image/webp`.
  **SVG is excluded**: it is XML that browsers execute as active content when
  served as `image/svg+xml`, so accepting it would make every upload a
  stored-XSS vector.
- The declared `Content-Type` is checked against **sniffed magic bytes**; a
  mismatch is a `400`. Declaring `image/png` and uploading HTML is the exact
  attack this closes.
- Size cap `INKWELL_MEDIA_MAX_BYTES` (default 5 MiB, ceiling 256 MiB) is applied
  both as the route's body limit and in the handler, so an over-cap request is
  refused before it is fully buffered.

Serving is public and unauthenticated (published pages must render for anyone),
with `Content-Type`, `Content-Length`, `ETag`, `Cache-Control: public,
max-age=31536000, immutable`, and the global `X-Content-Type-Options: nosniff`.

### 4. Delete and orphan handling

`DELETE /media/{id}` requires `write` **and** ownership (`admin` may delete any).
It deletes the row, then removes the blob only when no other row references the
same key — content addressing means two owners uploading identical bytes share
one blob, so refcounting by key is required to avoid deleting live content.

Two orphan classes are accepted, deliberately:

1. **Orphaned blobs.** If the row is deleted but the blob removal fails (or the
   process dies between the two), bytes are left behind. This direction is
   chosen on purpose: the failure mode is wasted disk, never a broken image.
   Failures are logged with the `storage_key` and `media_id`.
2. **Unreferenced media.** A media row whose URL no longer appears in any
   document body is *not* garbage-collected. Markdown bodies are free text and a
   URL may live outside Inkwell (a syndicated copy, a cached feed), so reaping by
   reference-scanning would delete images that are still in use. Cleanup is left
   to the operator, who can list media by `created_at`/`owner_id` and delete via
   the API.

A future sweep job could reconcile blobs against rows (delete blobs with no
referencing row); it is not needed for v0.2 and is not implemented.

### 5. Migration and backwards compatibility

Migration 0025 adds the new columns, drops `NOT NULL` from `media.data`, adds the
key/checksum `CHECK` constraints, backfills `checksum_sha256` for existing rows,
and creates `media_blobs`. Pre-0025 rows keep their inline bytes and are served
from `data` when `storage_key IS NULL`, so upgrading requires no blob rewrite and
no downtime.

## Alternatives considered

- **Keep blobs in Postgres only.** Simplest, and it is still available as a
  backend, but it makes the database carry binary weight forever and gives
  operators no path to an object store.
- **Store under the original filename** (sanitised). Needs a sanitiser to be
  correct forever, invites collisions, and leaks author filenames into public
  URLs. Content addressing removes the whole class of problem.
- **Multipart/form-data uploads.** Raw bytes plus `Content-Type` keeps the API
  and the client trivial (`fetch` with the `File` as the body) and needs no
  multipart parser. `media.filename` stays nullable for a future multipart path.
- **Image re-encoding / thumbnailing.** Would neutralise polyglot files and cut
  bandwidth, but costs a heavy image dependency and CPU per upload. Deferred;
  sniffing plus `nosniff` plus the SVG exclusion covers the security case.

## Consequences

- The default install writes to the local filesystem, so **the media directory
  must be persistent**: `docker-compose.yml` mounts the `inkwell-media` volume at
  `/app/data/media`, and `docs/DEPLOYMENT.md` documents the requirement.
- Backups now have two parts on the local backend (database + media directory);
  `docs/BACKUP-RESTORE.md` covers this. The Postgres backend keeps a single
  backup surface, which is its main appeal.
- A missing blob (wrong backend configured, volume not mounted) surfaces as a
  `404` with a loud `error`-level log naming the id, key, and backend.
