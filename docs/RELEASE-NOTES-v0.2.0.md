# Inkwell v0.2.0

Released 2026-07-25. Five headline changes:

| Area | What landed |
|------|-------------|
| **Scoped API tokens** | Per-author, individually revocable bearer tokens with scopes, replacing the single shared key as the everyday credential (ADR 0009) |
| **Authoring web UI** | Server-rendered browser editor + media picker behind `INKWELL_BROWSER_LOGIN` (ADR 0010) |
| **Media upload + hosting** | `POST/GET/DELETE /media`, pluggable storage backend, magic-byte sniffing, immutable cache + `ETag` (ADR 0013) |
| **Observability** | Structured JSON logs with request ids, Prometheus `/metrics`, split `/healthz` + `/readyz` (ADR 0012) |
| **Backup + restore** | `inkwell backup` / `inkwell restore` for database + media in one bundle, refusing a non-empty target without `--overwrite` (ADR 0014) |

The per-release summary of everything else in this span is in
[`CHANGELOG.md`](https://github.com/HexSleeves/inkwell/blob/main/CHANGELOG.md).

## Highlights

### Scoped author tokens replace the single shared key

v0.1 protected every write route with one all-powerful `INKWELL_API_KEY`. A leak
required whole-site rotation and gave no attribution. v0.2 introduces per-author,
individually revocable bearer tokens with explicit scopes.

- **Token model** — `authors` own `author_tokens`. Each token is
  `ink_<prefix>_<secret>`; only a SHA-256 digest is stored, the full secret is
  shown once at creation. Scopes: `read`, `write`, `publish`, `admin`.
- **Scope enforcement** — every mutating route now checks scope:
  - `read` — list/read drafts owned by the author (owner-scoped read visibility).
  - `write` — create/update/delete documents and mint draft preview tokens.
  - `publish` — `POST /documents/:slug/publish` and `/unpublish`.
  - `admin` — manage tokens (`/admin/tokens*`). A `write` token **cannot** mint or
    revoke tokens.
- **Ownership** — non-admin principals only act on documents they own;
  `documents.owner_id` is `NOT NULL`, backfilled to the bootstrap admin.
- **Immediate revocation** — a revoked token fails authentication on the very
  next request (the auth path rejects `revoked` rows before any scope check).
- **Token management** — `GET/POST /admin/tokens`,
  `POST /admin/tokens/:prefix/revoke`, `POST /admin/tokens/prune`, and the
  `inkwell author token {create,list,revoke}` CLI.
- **Write audit** — create/update/delete/publish/unpublish actions emit audit
  rows attributed to the acting principal (shared key logs as `bootstrap-admin`,
  scoped tokens log as the owning author), so fallback use is distinguishable.
- **MCP** — the standalone `INKWELL_MCP_KEY` was retired; the MCP server
  authenticates with a scoped token supplied via `INKWELL_API_KEY`.

### Authoring web UI

Writing no longer requires the CLI. `INKWELL_BROWSER_LOGIN=true` (default off)
turns on a server-rendered authoring surface:

- `GET /login` + `POST /auth/login` / `POST /auth/logout` — session-cookie login
  (`sessions` table, migration 0020) for an author who holds a scoped token.
- `GET /editor` — list your documents; `GET /editor/new` and
  `GET /editor/{slug}` — create and edit Markdown in the browser.
- `GET /media/new` — upload an image and copy its `/media/{id}` URL into a note.

The HTML pages are shells over the existing `/documents` JSON API: **auth and
scope are enforced on the API routes, not by the UI**, so a browser session gets
exactly the permissions its author's token carries. With the flag off, all of
these paths fall through to the public `/{slug}` handler, so a default install
exposes no auth or authoring surface at all. See
[ADR 0010](adr/0010-browser-login.md).

### Media upload and hosting

- `POST /media` (multipart, `write` scope), `GET /media/{id}` (public),
  `DELETE /media/{id}` (owner or admin).
- **Pluggable storage** — `INKWELL_MEDIA_BACKEND=local` (default) writes
  content-addressed files under `INKWELL_MEDIA_DIR` (`./data/media`);
  `postgres` keeps bytes in the `media_blobs` table. See
  [ADR 0013](adr/0013-media-storage.md).
- **Type verification** — the stored content type comes from magic-byte
  sniffing, not the client's claim. Size cap is `INKWELL_MEDIA_MAX_BYTES`
  (default 5 MiB, hard ceiling 256 MiB).
- **Caching** — served with `Cache-Control: public, max-age=31536000, immutable`
  and a content-SHA-256 `ETag`; a conditional request gets `304`.
- `inkwell author upload <file>` prints the URL to embed; the authoring UI can
  insert it directly.

### Observability

- **Structured logs** — JSON by default; `INKWELL_LOG_FORMAT=pretty` for local
  dev. Filter with `INKWELL_LOG` (checked before `RUST_LOG`).
- **Request ids** — every request carries `X-Request-Id`, echoed in the response
  headers, the log line, and `error.requestId`.
- **Metrics** — Prometheus text exposition at `GET /metrics`, registered only
  when `INKWELL_METRICS_ENABLED=true`. Set `INKWELL_METRICS_TOKEN` to require
  `Authorization: Bearer <token>` on scrapes. Secrets are redacted from logs and
  config dumps.
- **Probes** — `GET /healthz` (liveness, no DB) and `GET /readyz` (readiness,
  DB-aware); `GET /health` is retained as an alias of `/readyz`.
- Guide: [`docs/OBSERVABILITY.md`](OBSERVABILITY.md),
  [ADR 0012](adr/0012-observability.md).

### Backup and restore

- **One bundle** — `inkwell backup` captures the database and the media blobs
  together with a manifest, so a restore cannot silently half-apply.
- **Refuses to clobber** — `inkwell restore` exits non-zero against a non-empty
  target unless `--overwrite` is passed, and against a corrupt bundle changes
  nothing. Both paths matter because the docs ship a cron wrapper.
- **Search index is rebuilt, not copied** — `search_vector` is a generated
  column and is excluded from the bundle.
- Guide: [`docs/BACKUP-RESTORE.md`](BACKUP-RESTORE.md),
  [ADR 0014](adr/0014-backup-restore.md).

## Migrating from v0.1

**Your existing deployment keeps working with no change.** The shared
`INKWELL_API_KEY` is retained as the **bootstrap / admin (break-glass)**
credential (ADR 0009, Option B): when it matches, the request is treated as a
synthetic `bootstrap-admin` actor with implicit `admin` scope that bypasses
ownership checks. No `legacy`/migration downtime — v0.1 clients that send the
shared key are still authenticated and authorized exactly as before.

Recommended upgrade path:

1. **Upgrade in place.** Apply migrations (`0011`–`0017`); the bootstrap admin is
   seeded automatically and existing documents are backfilled to own it. Your
   current `INKWELL_API_KEY` continues to authenticate.
2. **Mint scoped tokens** for day-to-day use:
   ```bash
   # uses the admin/shared key to mint a per-client token
   inkwell author token create --name laptop --scopes read,write,publish
   ```
3. **Switch clients** (CLI, MCP server, CI) to the scoped token by setting
   `INKWELL_API_KEY` to the minted `ink_…` value. Admin scope is only needed for
   token management itself.
4. **Reserve the shared key** for initial provisioning and emergency recovery.

### Deprecation: shared `INKWELL_API_KEY` as an everyday credential

The shared key is **not removed** and remains required at boot (fail-closed). It
is, however, **deprecated as the default authoring credential** and is now
positioned as a setup / break-glass admin key only. Mint scoped tokens for all
routine authoring, automation, and agent access. A future major release may
require the shared key to be used solely for bootstrap; plan to move clients onto
scoped tokens now. Shared-key use is auditable (logged as `bootstrap-admin`) so
operators can track residual reliance on it.

## Verification

- Scope enforcement, owner-scoped read visibility, immediate revocation, and the
  shared-key admin fallback are covered by the database-backed contract tests
  (`tests/scoped_tokens_slice{1,2,3,3b,4}.rs`, `tests/token_admin_ux.rs`) and the
  no-DB auth unit tests in `src/http/auth.rs`.
- Run the DB-backed suite against a Postgres with `DATABASE_URL` set (see
  `docker-compose.yml`); they skip automatically when it is unset.

## References

- ADR: [`docs/adr/0009-scoped-author-tokens.md`](adr/0009-scoped-author-tokens.md)
- Authoring guide: [`docs/AUTHORING.md`](AUTHORING.md)
- v0.1 audit (gap closed): [`docs/audit-v0.1.md`](audit-v0.1.md)
