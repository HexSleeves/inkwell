# Inkwell v0.2.0 (unreleased)

Draft release notes for the next tag. The headline change is **scoped author
tokens** (ADR 0009), which closes the single-shared-key gap flagged in the v0.1
security audit ([`docs/audit-v0.1.md`](audit-v0.1.md)).

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
