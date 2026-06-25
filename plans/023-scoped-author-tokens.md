# Plan 023: Scoped Author Tokens, Document Ownership, and Write Audit

Implements **ADR 0009 (Option B)**. Replaces the single shared-owner `INKWELL_API_KEY`
with per-author identity, scoped bearer tokens, document ownership, and a write-audit
trail — while keeping the shared key as an admin/bootstrap fallback. Browser auth UI
stays out of scope (ADR Option C, deferred).

## Decisions (signed off 2026-06-23)

1. **Multi-author** — full ownership enforcement (an author may only edit/delete/publish their own notes; admin may touch any).
2. **Existing documents** → assigned to a single bootstrap **admin author** on migration.
3. **Token format** — opaque DB token `ink_<prefix>_<secret>`; only a SHA-256 **hash** is stored (never the token).
4. **Scopes** — `read`, `write`, `publish`, `admin` (see below).
5. **MCP key** — migrated to a scoped token (`write`, no `admin`); `INKWELL_MCP_KEY` retired once issued.
6. **Transport** — keep the existing `x-api-key` header (no `Authorization: Bearer`).
7. **Auth UI** — deferred.

## Scopes

| scope | grants |
|-------|--------|
| `read` | see drafts/unlisted (owner-visibility reads) |
| `write` | create notes; update/delete **own** notes |
| `publish` | publish/unpublish **own** notes |
| `admin` | all of the above on **any** note + manage authors/tokens; the shared-key principal has this |

A token carries a subset of scopes. Ownership is enforced for non-admin principals on every mutating route.

## Schema (new migrations)

- `authors(id uuid pk, name text not null, created_at timestamptz default now())`
- `author_tokens(id uuid pk, author_id uuid not null fk->authors on delete cascade, token_hash text not null, prefix text not null unique, scopes text[] not null, created_at timestamptz default now(), last_used_at timestamptz, revoked_at timestamptz)`
  - lookup by `prefix`, then **constant-time** compare `sha256(provided)` against `token_hash`; reject if `revoked_at is not null`.
- `documents.owner_id uuid` — nullable initially; backfilled to the bootstrap admin author; set `NOT NULL` in slice 4.
- `write_audit(id uuid pk, actor_author_id uuid null fk->authors, actor_label text not null, action text not null check in (create,update,delete,publish,unpublish), document_id uuid null, slug text, at timestamptz default now())`
  - `actor_label` = `'shared-key'` for the bootstrap/admin shared key, else the author name.

## Auth resolution (src/http/auth.rs)

Replace `is_authenticated(headers, api_key, mcp_key) -> bool` with:

```rust
authenticate(headers, &Config, &PgPool) -> Option<Principal>
struct Principal { author_id: Option<Uuid>, label: String, scopes: HashSet<Scope> }
```

- `x-api-key` matches `INKWELL_API_KEY` → admin bootstrap principal (`author_id = bootstrap admin`, `label = "shared-key"`, all scopes).
- else parse `ink_<prefix>_<secret>` → look up token by `prefix` → constant-time hash compare → live token → `Principal{ author_id, scopes }`; bump `last_used_at`.
- else `None` (public). Public reads unchanged.

Keep the single-header / non-empty / non-ASCII rejection rules from the current impl.

## Route changes (src/http/api.rs + ai.rs)

- Mutations (`create/update/delete/publish/unpublish`): require a principal with the needed scope (missing scope → 403); ownership is enforced atomically by an owner-scoped `WHERE` in the mutating query (non-owner → 0 rows → 404). `create` stamps `owner_id`. Each emits a `write_audit` row.
- Owner-visibility reads (drafts in get/list/backlinks/related/ask): gated by `read` scope (admin implies it).

## CLI (src/cli/)

`inkwell author token create --name <author> --scopes write,publish [--server URL]` (admin via shared key) → prints the token **once**. `... token list`, `... token revoke <prefix>`. Refactor `author.rs` + the MCP server to authenticate with a token from env (`INKWELL_API_KEY` keeps working as admin).

## Rollout — 4 ship-card slices (each CI-green, no lockout)

1. **[DONE — commit 0208156]** **Schema + audit + ownership backfill, no enforcement.** Migrations for `authors`, `author_tokens`, `documents.owner_id` (nullable), `write_audit`. Seed a bootstrap admin author; backfill existing docs' `owner_id`. Emit audit rows on writes. Shared key still all-powerful; behavior otherwise unchanged.
2. **[DONE — 2026-06-23]** **Token issuance + resolution.** `authenticate()` + `Principal`; token CLI (`inkwell author token create/list/revoke`); writes accept tokens **alongside** the shared key. Per-author audit attribution. Token management exposed over HTTP at `/admin/tokens` (admin-gated). No ownership/scope enforcement on document routes yet. *Notes vs plan:* token management is HTTP admin routes (operators manage prod over HTTP), admin-gated from day one to block privilege escalation; the MCP server keeps using `INKWELL_MCP_KEY` (still admin) — its move to a scoped token is folded into slice 4's retirement; the slice-1 detached audit insert was changed to an awaited (durable) insert.
3. **[DONE — 2026-06-23]** **Enforcement.** Scope checks on every mutating route (`write` create/update/delete, `publish` publish/unpublish; missing scope → 403). Ownership enforced **atomically** in the mutating query via `owner_filter` (non-admin → owner-scoped `WHERE`; non-owner → 0 rows → 404, no TOCTOU; admin bypasses). `create` stamps `owner_id` from the principal. Draft READ requires the `read` scope (admin implies all). *Scope cut:* per-owner draft READ isolation is **deferred to slice 3b** — it needs reworking the binary `Visibility` (Public/All) into an owner-aware filter across ~6 query modules; this slice ships the coarse read gate (a `read` token sees all drafts). The write-ownership boundary — the real privilege win — is fully enforced.
   - **[DONE — 2026-06-24] slice 3b:** owner-aware `Visibility` so a `read`-scoped author sees only their OWN drafts. The binary `Visibility` (Public/All) becomes `Public` / `Owner(Uuid)` / `All`, derived once in `resolve_visibility` (`src/http/api.rs`) and threaded through every read surface (documents get/list/search, links/backlinks/graph, RAG chunks vector/fts/related, garden embed transclusion, `/ask`+`/related`). Owner sees own drafts + all published; `All` is now admin-only; the public site stays pinned to `Visibility::Public`. Ownership lands in the same SQL `WHERE` as the read (no TOCTOU). Covered by `tests/scoped_tokens_slice3b.rs` (owner isolation, no cross-author leak via `/related`+`/ask`, admin sees all, write-only token stays blind).
4. **[DONE — 2026-06-23]** **Downgrade shared key + tighten.** `documents.owner_id` → `NOT NULL` (migration 0017). *Deviation:* the DB DEFAULT (bootstrap admin) is **kept**, not dropped — dropping it broke seed/test/maintenance inserts that omit `owner_id` for no correctness gain (NOT NULL already guarantees ownership). `INKWELL_MCP_KEY` **retired**: removed from `Config`/`AuthorConfig`/`auth`; the MCP server authenticates with `INKWELL_API_KEY` set to a scoped token. The shared `INKWELL_API_KEY` is now the sole static admin/bootstrap credential. **BREAKING:** deploys must set the MCP host's `INKWELL_API_KEY` to a scoped token before redeploying.

## Testing (per slice)

- token resolve: valid/revoked/unknown-prefix/wrong-secret (constant-time); shared-key admin path.
- ownership: author A cannot update/delete/publish author B's note (404 — owner-scoped write matches no row); admin can; create stamps owner.
- scope: `read`-only token can't write; `write` can't `publish`; etc.
- audit: every mutation writes one row with the right actor + action; shared-key writes labelled `shared-key`.
- migration: existing docs end up owned by the bootstrap admin; backfill idempotent.
- no-leak preserved: public still sees published-only across all surfaces.

## Risks / notes

- **No lockout:** the shared key stays valid through all slices; enforcement only flips in slice 3 after tokens exist.
- Token shown once at creation; only the hash is stored. Revocation is immediate (checked on every request).
- Existing single-owner semantics are preserved for the bootstrap admin, so the current garden keeps working.

## Out of scope

Browser sessions/login UI (ADR Option C), email/password, OAuth, per-note ACLs/sharing, rate limiting (separate backlog item).
