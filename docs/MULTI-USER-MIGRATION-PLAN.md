# Multi-user migration plan and blast radius

Companion to [ADR 0016 — Multi-user accounts and per-document
ownership](adr/0016-multi-user-accounts.md). The ADR decides *what* and *why*;
this document is the executable *how*, plus the list of every surface whose
behaviour changes once documents are owned.

> **Status: not executable yet.** Implementation is gated on the CYP-48 board
> answer. Nothing here has been applied — `migrations/` stops at
> `0025_media_storage_backends.sql`. This document exists so that when the gate
> opens, no decision has to be made twice.

---

## 0. Facts an executor must know before starting

Verified against `main`, not assumed:

- **Ownership already exists.** `documents.owner_id` and `media.owner_id` are
  both `NOT NULL REFERENCES authors(id)`. Do not add them.
- **The bootstrap owner already exists** at the fixed uuid
  `00000000-0000-0000-0000-000000000001`, name `admin`, seeded by
  `migrations/0015_seed_bootstrap_admin.sql` and exposed in Rust as
  `BOOTSTRAP_ADMIN_ID` (`src/domain/author.rs`). Reuse it; do not seed a second
  one.
- **The nullable → backfill → default → NOT NULL pattern is already proven here**
  by migrations `0013` → `0015` → `0016` → `0017`. Follow it verbatim for new
  columns.
- **Migrations run in a transaction by default.** `sqlx::migrate!("./migrations")`
  (`src/db/migrations.rs:43`). A migration file whose **first line** is
  `-- no-transaction` runs outside one (sqlx 0.9, `sqlx-core/src/migrate/source.rs`).
  That directive is **required** for any `CREATE INDEX CONCURRENTLY`.
- **`inkwell migrate rollback` cannot roll back anything past version 6.** The
  hand-maintained `MIGRATIONS: [MigrationDef; 6]` array in
  `src/db/migrations.rs:10` is the only source of down-SQL, and `rollback()`
  errors with `no migration definition found for {version}` for any later
  version (`src/db/migrations.rs:60-63`). It also runs every step inside one
  transaction, so a `-- no-transaction` migration could not be reversed by it
  even if a definition existed. **Rollback is therefore a new forward
  migration.** Plan accordingly; see §5.

---

## 1. Migration sequence

Seven migrations, `0026`–`0032`. Every one is additive or index-only. No single
locking migration is required, and no table rewrite occurs: on PostgreSQL 11+ an
`ADD COLUMN` with no default and no `NOT NULL` is a catalog-only change.

### 0026 — account columns on `authors` (in-transaction, instant)

```sql
ALTER TABLE authors
  ADD COLUMN IF NOT EXISTS email              text,
  ADD COLUMN IF NOT EXISTS password_hash      text,
  ADD COLUMN IF NOT EXISTS email_verified_at  timestamptz,
  ADD COLUMN IF NOT EXISTS disabled_at        timestamptz;
```

Decisions already made — do not revisit:

- All four columns are **nullable forever**. Token-only authors (the bootstrap
  admin, CI clients, the MCP server) have no password and must stay legal. See
  ADR 0016 §2.
- `email` is plain `text`, not `citext`: `citext` needs an extension, and the
  uniqueness rule is enforced by a `lower(email)` expression index instead
  (0027). Application code lowercases on write.
- `password_hash` holds an Argon2id PHC string. Argon2 is not currently a
  dependency; adding `argon2` is part of the implementation slice, not this
  migration.
- `disabled_at` is a soft-disable for accounts. Deleting an `authors` row already
  cascades `author_tokens`, `sessions`, and `media`
  (`ON DELETE CASCADE`) but **not** `documents` (plain `REFERENCES`, so a delete
  is refused while the author owns notes) and sets `write_audit.actor_author_id`
  to `NULL`. That is the intended shape; disable, do not delete.

### 0027 — unique email index (`-- no-transaction`)

```sql
-- no-transaction
CREATE UNIQUE INDEX CONCURRENTLY IF NOT EXISTS authors_email_lower_uniq
  ON authors (lower(email))
  WHERE email IS NOT NULL;
```

Partial, so the bootstrap admin and every other passwordless author remain legal.
`CONCURRENTLY` here is precautionary rather than necessary (`authors` is tiny),
but it establishes the pattern the next two migrations depend on and costs
nothing.

**If this fails** it leaves an `INVALID` index. Recovery is
`DROP INDEX authors_email_lower_uniq;` then re-run — this is why `IF NOT EXISTS`
is present.

### 0028 — per-owner slug uniqueness, add (`-- no-transaction`)

```sql
-- no-transaction
CREATE UNIQUE INDEX CONCURRENTLY IF NOT EXISTS documents_owner_slug_uniq
  ON documents (owner_id, slug);
```

Semantically a no-op today: global `UNIQUE (slug)` already implies
`UNIQUE (owner_id, slug)`. After this runs, the instance is **strictly stricter**
than before — both constraints hold — which is the safe state to be halted in.

### 0029 — per-owner slug uniqueness, drop the global one (in-transaction, milliseconds)

```sql
ALTER TABLE documents DROP CONSTRAINT IF EXISTS documents_slug_key;
```

Takes `ACCESS EXCLUSIVE` on `documents`, but it is a catalog-only drop — O(1),
milliseconds, no scan, no rewrite. Set a short `lock_timeout` (e.g.
`SET LOCAL lock_timeout = '3s';`) at the top so it fails fast behind a long
transaction rather than queueing and blocking every reader.

Ordering is not negotiable: 0028 must be committed and `VALID` before 0029 runs,
so there is never an instant without a uniqueness guarantee on slugs.

> Confirm the constraint name before executing:
> `SELECT conname FROM pg_constraint WHERE conrelid = 'documents'::regclass AND contype = 'u';`
> `documents_slug_key` is PostgreSQL's default name for a column-level `UNIQUE`
> on `slug`, but a restored database could differ.

### 0030 — `slug_aliases.owner_id`, nullable then backfilled (in-transaction)

```sql
ALTER TABLE slug_aliases
  ADD COLUMN IF NOT EXISTS owner_id uuid REFERENCES authors (id);

UPDATE slug_aliases AS a
   SET owner_id = d.owner_id
  FROM documents AS d
 WHERE a.document_id = d.id
   AND a.owner_id IS NULL;

ALTER TABLE slug_aliases
  ALTER COLUMN owner_id SET NOT NULL;
```

Safe as one transaction because `slug_aliases` only grows on document rename — it
is orders of magnitude smaller than `documents`. If a deployment has an unusually
large alias table, split it exactly as `0013`/`0015`/`0017` split `documents`:
add nullable, backfill in a separate migration, tighten in a third.

The `UPDATE` is idempotent (`WHERE owner_id IS NULL`), so a re-run is a no-op.
The `SET NOT NULL` is guaranteed to succeed because `document_id` is already
`NOT NULL` with an FK to `documents`, and `documents.owner_id` is `NOT NULL` —
there is no row the join can miss.

### 0031 / 0032 — per-owner alias uniqueness (`-- no-transaction`, then in-transaction)

The alias primary key is a **global slug reservation** (`old_slug PRIMARY KEY`)
and must become per-owner. Two files, in this order:

`0031_slug_aliases_owner_unique.sql`:

```sql
-- no-transaction
CREATE UNIQUE INDEX CONCURRENTLY IF NOT EXISTS slug_aliases_owner_old_slug_uniq
  ON slug_aliases (owner_id, old_slug);
```

`0032_slug_aliases_drop_global_pk.sql`:

```sql
SET LOCAL lock_timeout = '3s';
ALTER TABLE slug_aliases DROP CONSTRAINT slug_aliases_pkey;
ALTER TABLE slug_aliases
  ADD CONSTRAINT slug_aliases_pkey
  PRIMARY KEY USING INDEX slug_aliases_owner_old_slug_uniq;
```

`PRIMARY KEY USING INDEX` adopts the index built concurrently in 0031 instead of
building a new one under an exclusive lock. `slug_aliases_document_id_idx`
(from `migrations/0021`) is unaffected and still serves the cascade-side lookups.

### What is deliberately NOT in the sequence

- **No `owner_id` on `links`, `note_chunks`, `webmentions`, or
  `preview_tokens`.** They inherit via their `documents` FK, and their uniqueness
  keys already contain a `documents.id`. ADR 0016 §1 has the rule and the
  reasoning.
- **No new `accounts` table.** ADR 0016 §2.
- **No changes to `sessions`.** The shipped table already carries `author_id` and
  per-session `scopes` with an admin-excluding CHECK. Password login mints rows in
  it exactly as token exchange does today.
- **No RLS policies.** ADR 0016 *Alternatives*.

---

## 2. Assigning existing rows without downtime

Already solved, and the solution is reused rather than redesigned:

1. **Bootstrap owner:** exists at the fixed uuid, seeded by `0015`. Existing
   `documents` and `media` rows already point at it. Nothing to backfill.
2. **New columns:** every one is added nullable, so writers on the old binary
   keep working during the rollout. `slug_aliases.owner_id` is derived by join,
   not guessed.
3. **Rollout order:** migrations `0026`–`0032` are all backward-compatible with
   the *currently deployed* binary — it reads none of the new columns, and the
   uniqueness swap is invisible to it while one owner exists. So migrate first,
   deploy second, in either order, with no coordinated window.
4. **The bootstrap admin becomes the first account** by an operator setting its
   email and password (a new `inkwell author set-password` subcommand alongside
   the existing `src/cli/author.rs`), not by a migration. Never write a
   credential into a migration file: migrations are committed to git, and
   `docs/BACKUP-RESTORE.md` deliberately excludes secrets from bundles for the
   same reason.

---

## 3. Nullable-then-backfill-then-constrain, or one locking migration?

**Nullable-then-backfill-then-constrain, for every column.** No locking migration
is needed anywhere. But the honest answer has an exception, and it is not about
columns:

- **Columns:** all four `authors` columns and `slug_aliases.owner_id` are
  additive. `ADD COLUMN` with no default is catalog-only on PG11+. Zero risk.
- **Indexes:** built `CONCURRENTLY`, outside a transaction, so no writer is
  blocked.
- **Constraint swaps (0029, 0032):** these *do* take `ACCESS EXCLUSIVE`, and
  there is no concurrent form of `DROP CONSTRAINT`. They are catalog-only and
  complete in milliseconds, so the exposure is queueing behind an existing long
  transaction, not the drop itself. `SET LOCAL lock_timeout` converts that from
  an outage into a retry. This is the one place where "no locking migration" is a
  simplification: the lock is taken, it is just not *held*.

---

## 4. The point of no return

State it plainly, because it governs the whole schedule:

> Rollback is free until the first `documents` row exists whose `slug` collides
> with another owner's `slug`.

Before that moment, global slug uniqueness can be restored:

```sql
-- no-transaction
CREATE UNIQUE INDEX CONCURRENTLY documents_slug_key_restored ON documents (slug);
```

If that command **succeeds**, rollback was still available and has been taken. If
it **fails** with a duplicate-key error, rollback is no longer a migration — it is
a data merge requiring a human decision per colliding slug. That failure is the
correct signal, not an error to work around.

The same test, as a check rather than an attempt:

```sql
SELECT slug, count(*) FROM documents GROUP BY slug HAVING count(*) > 1;
```

Empty result ⇒ reversible.

---

## 5. Rollback story if the migration is halted midway

There is no `down` migration path past version 6 (§0). Every recovery below is a
**new forward migration** or a manual `psql` statement, and each is written to be
safe from the exact state a halt leaves behind.

| Halted after | Database state | Recovery |
|---|---|---|
| 0026 | Four unused nullable columns on `authors` | **Do nothing.** No code reads them. Leave them; they are inert. |
| 0027 | Email index exists, possibly `INVALID` if it failed | `DROP INDEX IF EXISTS authors_email_lower_uniq;`. Check validity with `SELECT indisvalid FROM pg_index WHERE indexrelid = 'authors_email_lower_uniq'::regclass;` |
| 0028 | **Both** slug constraints present | Safest possible halt state — strictly stricter than before, app fully functional. Either continue or `DROP INDEX documents_owner_slug_uniq;`. |
| 0029 | Only `(owner_id, slug)` unique | Reversible **only** while §4 holds. Restore with the `CONCURRENTLY` statement in §4, then drop the composite index. |
| 0030 | `slug_aliases.owner_id` populated and `NOT NULL` | Reverse with `ALTER TABLE slug_aliases ALTER COLUMN owner_id DROP NOT NULL;` then `DROP COLUMN owner_id;`. No data loss — the value is derivable from the `documents` join at any time. |
| 0031 | Composite alias index exists alongside the global PK | Same shape as 0028: strictly stricter, safe. `DROP INDEX slug_aliases_owner_old_slug_uniq;` to reverse. |
| 0032 | Alias PK is `(owner_id, old_slug)` | Reversible while no two owners have retired the same slug: `ALTER TABLE slug_aliases DROP CONSTRAINT slug_aliases_pkey; ALTER TABLE slug_aliases ADD PRIMARY KEY (old_slug);` — fails if collisions exist, which is again the correct signal. |

Two rules for the executor:

1. **Never mix a `-- no-transaction` migration with other statements.** If the
   `CREATE INDEX CONCURRENTLY` fails halfway there is no rollback, so the file
   must contain nothing else to leave half-applied.
2. **Extend `MIGRATIONS` in `src/db/migrations.rs` or delete it.** Shipping a
   `rollback` subcommand that silently only works for versions 1–6 is worse than
   not shipping one. Pick one in the implementation slice; ADR 0016 does not
   decide it because it is a CLI-surface question, not a data-model one.

---

## 6. Blast radius

Every endpoint and public page whose behaviour changes once documents are owned
by more than one account. `→ shared` marks surfaces that change under **Option A
(multi-author single publication)** too; everything else changes only under
**Option B (multi-tenant hosting)**. Files are cited so an executor can go
straight to them.

### 6.1 Cross-tenant leaks that exist *today* and become real with a second owner

These are the ones that matter. Each is currently correct because there is
exactly one owner.

| Surface | File | What breaks |
|---|---|---|
| Wikilink resolution | `src/garden.rs:70` — `links::resolve_slug_ids(pool, &slugs, Visibility::Public)` | `[[about]]` in Alice's note resolves to **whichever owner** has a published `about`. Must resolve within the author's namespace. → **shared** |
| Embed resolution | `src/garden.rs:126` `resolve_embeds` | Same defect for `![[note]]` transclusion: Alice's note can inline Bob's body. → **shared** |
| Rename re-render fan-out | `src/garden.rs:266` `affected_sources(pool, note_id, slug)` | Renaming Alice's slug re-renders **Bob's** notes that link the same slug text. → **shared** |
| Stub backfill | `src/garden.rs:330` `backfill_after_change` | An unresolved `[[x]]` in Bob's note silently resolves the moment Alice publishes `x`. → **shared** |
| Outbound webhooks | `src/webhooks/mod.rs:143` `maybe_dispatch`; config `INKWELL_WEBHOOK_URLS` / `INKWELL_WEBHOOK_SECRET` (`src/config.rs:151-164`) | Endpoints and the signing secret are **process-global**. Alice publishing fires a signed webhook carrying Alice's document to **Bob's** endpoint. Shipped under CYP-53; needs per-owner endpoint config. → **shared** |
| `DELETE /media/{id}` | `src/http/media.rs:265-285` + `src/db/media.rs:122-127` | Per-handler owner check in front of an unscoped `DELETE ... WHERE id = $1`. See ADR 0016 §4. |
| `GET /admin/tokens` | `src/http/admin.rs:74-90` | Admin-scoped, lists **all** tokens for **all** authors. Needs the instance-admin vs account-admin split. → **shared** |
| `GET /settings` | `src/http/settings.rs:39` | Instance-wide counts at `Visibility::Public`, so the panel discloses the aggregate size of other tenants' published gardens. Low severity, real. |
| Media dedup by checksum | `src/db/media.rs:100-110` | Already correctly scoped (`WHERE owner_id = $1 AND checksum_sha256 = $2`). Listed as a **do not regress** anchor. |
| Shared HTML/XML cache | `src/http/cache.rs:8,17` — ETag is `build_etag(route_key, body)` and the response carries `Cache-Control: public` | **The structural constraint on the whole design.** The cache key is route + body only, with no principal dimension, so any HTML page that varies by caller would let a shared cache serve one tenant's page to another. `src/http/search.rs:56-71` already documents this and deliberately keeps the HTML search page public-only for exactly this reason. **Consequence: under Option B the owner must be part of the route** (`/@{handle}/…` or a host mapping) rather than derived from credentials — otherwise every cached public page becomes a cross-tenant channel. This is an independent argument for the same namespace decision the slug uniqueness migration forces. |

### 6.2 API endpoints

| Route | Handler | Change |
|---|---|---|
| `GET/POST /documents` | `src/http/documents.rs:171` | List already owner-filtered via `Visibility`. Create must stamp `owner_id` from the principal (it does) and reject a slug colliding **within the owner** rather than globally. |
| `GET/PUT/PATCH/DELETE /documents/{slug}` | `src/http/documents.rs:220,292,381,527` | Slug is no longer a global key. Every `WHERE slug = $1` in `src/db/documents.rs` (lines 112, 222, 287, 310, 348, 487, 524, 788) needs the owner predicate promoted from optional to required — the `OwnerScope` change in ADR 0016 §4. |
| `GET /documents/{slug}/history` | `src/http/documents.rs` | Owner-scoped lookup; same slug-key change. |
| `POST /documents/{slug}/publish`, `/unpublish` | `src/http/publish.rs:25,67` | Scope-checked but resolve by bare slug. Must resolve within the caller's namespace, and fire only the owner's webhooks. |
| `GET /documents/{slug}/backlinks` | `src/http/graph.rs:135` | Already `Visibility`-filtered. Backlinks must additionally not cross owners under Option B. |
| `GET /documents/{slug}/graph`, `GET /graph` | `src/http/graph.rs:172,265` | Same. `/graph` is the whole-instance graph — becomes per-owner. |
| `GET /documents/{slug}/related` | `src/http/ai.rs:176` | `Visibility`-filtered via `src/db/chunks.rs:342`. Retrieval must not cross owners. |
| `POST /ask` | `src/http/ai.rs:77` | RAG over `note_chunks` (`src/db/chunks.rs:195,470`) — already owner-visibility-filtered for **drafts**, but published chunks from all owners are in scope. Cross-tenant answer synthesis. |
| `GET /search` | `src/http/search.rs:56-71` | JSON path is owner-aware; the **HTML** path is hardcoded `Visibility::Public`, so it searches every owner's published notes. |
| `POST /media` | `src/http/media.rs:106` | Already stamps `owner_id`. No change. |
| `GET /media/{id}` | `src/http/media.rs:72` | Public and unauthenticated **by design** so embeds work. Cross-tenant read by id is an accepted consequence (ADR 0016 *Consequences*); ids must stay non-enumerable. |
| `POST/GET /documents/{slug}/preview-tokens`, `DELETE .../{prefix}` | `src/http/preview.rs:126,232,259` | Scope- and visibility-checked; slug resolution becomes owner-scoped. |
| `GET /documents/{slug}/preview` | `src/http/preview.rs` | Token-gated, unaffected by ownership except slug resolution. |
| `POST /webmention` | `src/http/webmention.rs` | Resolves the target note from a public URL. Once URLs are namespaced, target parsing changes shape. |
| `GET/POST /admin/tokens`, `/admin/tokens/prune`, `/admin/tokens/{prefix}/revoke` | `src/http/admin.rs:74,92,188` | `Scope::Admin` currently means instance-god. Splits into: account-admin (manage **my** tokens) vs instance-admin (manage anyone's). |
| `POST /auth/login`, `/auth/logout` | `src/http/auth_session.rs` | Login gains a password path beside token exchange. → **shared** |
| `POST /auth/register` | *new* | Gated by `INKWELL_SIGNUP_OPEN` (default false). → **shared** |
| `GET /health`, `/healthz`, `/readyz`, `/metrics` | `src/http/health.rs`, `observability.rs` | **No change.** Instance-level by definition. |

### 6.3 Public page paths

All served at `Visibility::Public` across every owner today.

| Path | File | Change under Option B |
|---|---|---|
| `GET /` | `src/http/pages.rs` `index` | Whose garden? Either an instance landing page listing accounts, or the primary owner's index. |
| `GET /page/{page}` | `src/http/pages.rs` `index_page` | Follows `/`. |
| `GET /{slug}` | `src/http/pages.rs` `document_page` | **The load-bearing route.** A bare slug stops being unique. Becomes `/@{handle}/{slug}` (or host-mapped — ADR 0016 *Alternatives* leaves routing open, schema unaffected). |
| Alias 301 redirect | `src/http/pages.rs:101` → `documents::resolve_alias_target` (`src/db/documents.rs:788`) | Alias lookup is by slug alone; needs the `slug_aliases.owner_id` added in 0030. This is why that column exists. |
| Public backlinks block | `src/http/pages.rs:47-51` | Rendered at `Visibility::Public`; must also not cross owners. |
| `GET /notes` | `src/http/pages.rs` `notes_index` | Per-owner listing. |
| `GET /tags`, `/tags/{tag}`, `/tags/{tag}/page/{page}` | `src/http/pages.rs` | Tag namespace becomes per-owner, or explicitly instance-wide as a discovery feature. **Decide once**; today it is accidentally instance-wide. |
| `GET /archive`, `/archive/{year}/{month}`, `/archive/{year}/{month}/page/{page}` | `src/http/pages.rs` | Per-owner. |
| `GET /feed.xml` | `src/http/feed.rs` | One merged feed today. Per-owner feed + optional instance firehose. |
| `GET /sitemap.xml`, `/sitemap-static.xml`, `/sitemaps/documents/{page}`, `/sitemaps/tags/{page}` | `src/http/sitemap.rs` | Per-owner sitemaps, or an index-of-sitemaps. Affects SEO contracts in ADR 0006. |
| `GET /settings` | `src/http/settings.rs` | See §6.1 — global counts. |
| `GET /login`, `/media/new` | `src/http/auth_session.rs`, `src/http/media.rs:90` | Flag-gated pages; login gains password fields. → **shared** |
| `GET /editor`, `/editor/new`, `/editor/{slug}` | `src/http/editor.rs` | HTML shells over `/documents`; inherit whatever the API enforces. `/editor/{slug}` resolves a bare slug and needs the namespace change. → **shared** |
| `GET /assets/site.css`, `/assets/fonts/nunito.woff2` | `src/http/assets.rs` | **No change.** Static. |

### 6.4 Non-HTTP surfaces

| Surface | File | Change |
|---|---|---|
| `inkwell backup` / `restore` | `src/backup/{mod,create,restore}.rs` | Whole-instance today (all 11 tables). Multi-tenant wants per-account export. The schema-drift guard must classify new columns. `TABLES: [&str; 11]` is unchanged by this plan — no new tables. |
| MCP server | `src/mcp/mod.rs` | Authenticates with a scoped token, so it inherits `Principal`. Verify its tool surface resolves slugs owner-scoped. |
| Authoring CLI | `src/cli/author.rs`, `src/client/mod.rs` | Gains `set-password`; slug arguments become owner-relative. → **shared** |
| Seed / import | `src/cli/seed.rs:117` (`owner_id: None`), `src/cli/import.rs` | Rely on the DB default (bootstrap admin) from `migrations/0016`. Must take an explicit owner once accounts exist. |
| `docs/openapi.yaml`, `docs/API.md`, `docs/COMPATIBILITY.md` | — | Public wire contracts. Any URL-shape change is a documented compatibility break. |

---

## 7. Test plan for the implementation slice

Not part of CYP-61; recorded so the follow-up issue does not have to re-derive it.

- **Two-author fixture.** Every owner-filtered surface gets one test asserting
  author B receives `404` — never `403`, which would confirm the row exists and is
  itself a cross-tenant disclosure.
- **Slug collision.** Both authors create `about`; both succeed; each resolves to
  its own note through the API and the public page.
- **Alias collision.** Both authors rename away from `about`; both alias rows
  coexist; each 301 lands on the right document.
- **Wikilink containment.** Author B's `[[about]]` resolves to B's note while A's
  published `about` exists.
- **Webhook containment.** A's publish delivers to A's endpoints only.
- **Migration test.** Apply `0026`–`0032` against a database seeded with the
  single-owner v0.2 fixture; assert row counts unchanged, `slug_aliases.owner_id`
  fully populated, and `documents_slug_key` gone.
- **Rollback test.** Apply through 0029, run the §4 restore statement, assert it
  succeeds on single-owner data and fails after a colliding row is inserted.

Note: DB-backed tests need pgvector (`migrations/0009` runs
`CREATE EXTENSION vector`) and `INKWELL_REQUIRE_DB_TESTS=1`, or they skip
silently.
