# ADR 0016 — Multi-user accounts and per-document ownership

- Status: **Proposed** — the board decision on CYP-48 is what accepts this. No
  schema, auth, or handler code is written under CYP-61.
- Date: 2026-07-25
- Ticket: CYP-61 (parent CYP-44)
- Companion: [Multi-user migration plan](../MULTI-USER-MIGRATION-PLAN.md) —
  the executable migration sequence and the blast-radius list.

## Context

Inkwell today is a **single-operator tool with a multi-owner-shaped data model**.
That distinction is the whole ADR, so it comes first.

Per-document ownership is **already shipped**, not pending. ADR 0009 / CYP-41
landed it in four slices:

| Shipped surface | Where |
|---|---|
| `authors` — first-class principals | `migrations/0011_create_authors.sql` |
| `author_tokens` — per-author scoped tokens, hash-only, revocable | `migrations/0012_create_author_tokens.sql` |
| `documents.owner_id` **NOT NULL** REFERENCES `authors` | `migrations/0013`, `0016`, `0017` |
| `media.owner_id` **NOT NULL** REFERENCES `authors` | `migrations/0019_create_media.sql` |
| `sessions.author_id` — browser sessions (ADR 0010) | `migrations/0020_create_sessions.sql` |
| `Principal { author_id, label, scopes }` | `src/domain/author.rs:98` |
| `Visibility::{Public, Owner(uuid), All}` | `src/db/links.rs:41` |
| One derivation point for read visibility | `resolve_visibility`, `src/http/auth.rs:239` |

So "add `ownerId` to documents" is not the work. What is actually missing for
multi-user is three things, in increasing order of expense:

1. **Accounts.** An `authors` row exists only when an operator inserts one and
   mints a token. There is no email, no password, no signup. `POST /auth/login`
   exchanges an *already-issued scoped token* for a session cookie — a hosted
   product cannot ask a new signup to paste a token it does not have.
2. **A per-owner slug namespace.** `documents.slug` is `NOT NULL UNIQUE`
   (`migrations/0001_create_documents.sql:3`) and `slug_aliases.old_slug` is the
   `PRIMARY KEY` (`migrations/0021`). Both namespaces are **global**. The second
   account on the instance cannot create `about`, and cannot retire a slug the
   first account already retired.
3. **Public-surface semantics.** Every public renderer path runs at
   `Visibility::Public` over *all* rows: `/`, `/{slug}`, `/notes`, `/tags/*`,
   `/archive/*`, `/page/{n}`, `/feed.xml`, `/sitemap*.xml`, and the HTML
   `/search`. One merged garden across all owners is correct for a team blog and
   wrong for multi-tenant hosting.

Item 2 is the only genuinely expensive-to-reverse decision, and it is expensive
precisely because it is cheap *today*: a global-unique slug column can become
per-owner-unique for free while exactly one owner exists, and becomes a data
merge the moment two owners hold the same slug.

## The product fork this ADR is really about

Two coherent products can be built on the shipped model. They differ in cost by
an order of magnitude, and the board is choosing between them, not between
"multi-user yes/no".

### Option A — Multi-author single publication

One site, many authors. Keeps the global slug namespace, keeps `/` merged, keeps
every public path byte-identical. Needs only accounts (item 1) plus an authorship
byline. This is a team blog.

- **Cost:** small. No uniqueness migration, no URL change, no renderer change.
- **What it does not deliver:** tenants cannot be isolated. Alice sees Bob's
  drafts nowhere, but Alice's published notes and Bob's published notes are one
  publication with one feed, one sitemap, and one slug namespace. Two customers
  cannot be hosted on one instance.

### Option B — Multi-tenant hosting

Each account gets its own site. Needs items 1, 2, and 3.

- **Cost:** large, and item 2 must land before the second account exists.
- **What it delivers:** the actual hosted product — one Inkwell instance serving
  N independent gardens.

### Decision on the fork

**Recommend Option B, sequenced so the irreversible half lands first and the
visible half lands behind a flag.**

The reasoning is asymmetry of regret, not enthusiasm for the feature. The
uniqueness migration (item 2) is free while `count(distinct owner_id) = 1` and
unaffordable afterwards; the URL/renderer work (item 3) is expensive but stays
affordable forever. So the schema half is the one with a deadline. If the board
declines multi-user entirely, Option A is what the code already almost is, and
this ADR is the record of what was declined — see *Consequences* for what
"declined" costs later.

## Decision

### 1. `owner_id` lands on exactly one new table: `slug_aliases`

The state surface is the 11-table list enumerated on CYP-59 and confirmed
byte-for-byte against `src/backup/mod.rs` `TABLES: [&str; 11]`. Classifying every
one of them:

| Table | Owner column? | Why |
|---|---|---|
| `authors` | **is** the owner | The account row itself. |
| `documents` | ✅ **already has it** | `owner_id` NOT NULL, indexed (`documents_owner_id_idx`). |
| `media` | ✅ **already has it** | `owner_id` NOT NULL, `ON DELETE CASCADE`. |
| `author_tokens` | ✅ has `author_id` | Already the tenancy edge. |
| `sessions` | ✅ has `author_id` | Already the tenancy edge. |
| `write_audit` | ✅ has `actor_author_id` | Nullable by design so the trail survives author deletion. |
| `slug_aliases` | ❌ **needs a new `owner_id`** | See below — the one real addition. |
| `links` | ❌ inherits | FK `source_note_id` / `target_note_id` → `documents`. |
| `note_chunks` | ❌ inherits | FK `note_id` → `documents`, `UNIQUE (note_id, chunk_index)`. |
| `webmentions` | ❌ inherits | FK `target_note_id` → `documents`, `UNIQUE (source_url, target_note_id)`. |
| `preview_tokens` | ❌ inherits | FK `document_id` → `documents`; `prefix` is random, so global uniqueness carries no tenant semantics. |

`slug_aliases` is the exception, and the reason is specific rather than stylistic:
alias resolution is keyed **by slug before any document row is known**
(`documents::resolve_alias_target`, called from `src/http/pages.rs:101`). Its
`old_slug PRIMARY KEY` is therefore a *global* slug reservation that no join can
scope. Every other child table's uniqueness key already contains a
`documents.id`, so per-owner uniqueness follows from the parent for free.

**Rule to apply to future tables:** add `owner_id` only when the table has a
uniqueness constraint that does not already contain a `documents.id` or
`authors.id`. Otherwise inherit and join. Denormalized owner columns that exist
only "for convenience" are a second source of truth for tenancy and will
eventually disagree with the parent.

### 2. Accounts layer **on top of** `authors`; `author_tokens` stays as-is

`authors` becomes the account row. Add nullable `email`, `password_hash`,
`email_verified_at`, `disabled_at`. Do **not** introduce a separate `accounts`
table, and do **not** re-scope `author_tokens`.

Justification, concretely: `author_tokens.author_id`, `sessions.author_id`,
`documents.owner_id`, `media.owner_id`, and `Principal.author_id` are already the
same edge, and `resolve_visibility` already returns
`Visibility::Owner(principal.author_id)`. A separate `accounts` table would make
`Principal` carry two ids and force every owner-filtered query — roughly 20 call
sites across `src/db/{documents,links,chunks,media,audit}.rs` — to re-derive
which id it filters on. The account *is* the author; make it literally so and the
diff is four nullable columns plus a partial unique index.

Two consequences are accepted deliberately:

- **`email` and `password_hash` stay nullable forever.** Token-only authors are
  legitimate first-class principals: the bootstrap admin
  (`00000000-0000-0000-0000-000000000001`, `src/domain/author.rs`), CI clients,
  and the MCP server all authenticate with a scoped token and have no password.
  A `NOT NULL email` would make the shipped bootstrap path illegal. The unique
  index is therefore partial: `UNIQUE (lower(email)) WHERE email IS NOT NULL`.
- **`authors.name` becomes the public handle.** It is already `UNIQUE`, and
  Option B needs a per-owner URL segment anyway. Adding email means two unique
  identity columns on one table; that is the correct shape (handle is public and
  routable, email is private and a login key), not an accident to be cleaned up.

### 3. Browser sessions are **IN** the first slice

Not deferrable, for one reason: the authoring UI already exists and already
assumes a single operator. `INKWELL_BROWSER_LOGIN`, `sessions`, `/login`,
`/editor`, `/editor/new`, `/editor/{slug}`, and `/media/new` all shipped under
ADR 0010 / CYP-42, and today "login" means pasting a scoped token into a form
(`src/http/auth_session.rs`). Multi-user accounts whose only credential path is
"an admin mints you a token out of band" is not an account system; it is the
current system with more rows.

So slice 1 adds `POST /auth/register` and password login against the **existing**
`sessions` table, reusing its shipped properties: SHA-256 hash only, never the
raw token; `scopes` stored per session and applied verbatim; admin excluded by
CHECK constraint so a browser session can never exceed read/write/publish.

Explicitly **out** of the first slice: email delivery, enforced verification,
password reset, OAuth/SSO, MFA, org/team membership, per-document sharing or ACLs
beyond single-owner. `email_verified_at` is recorded but not gated on, so
verification can be enforced later without a schema change.

Signup is gated by a new `INKWELL_SIGNUP_OPEN` (default **false**), matching the
`INKWELL_BROWSER_LOGIN` precedent: a self-hoster's single-operator instance must
not silently become an open registration surface on upgrade.

### 4. Enforcement is a **query-layer owner filter**, made non-optional in types

This is the sub-decision most likely to be got wrong later, and the repo already
contains a worked example of each approach, so the comparison is evidence rather
than preference.

**Query-layer (the shipped majority).** `Visibility` is threaded as a parameter
through `src/db/documents.rs`, `src/db/links.rs`, and `src/db/chunks.rs`. Two
properties follow:

- A new read query **cannot compile** without deciding its visibility, because
  the parameter is required. The compiler is the reviewer.
- Write paths put the filter in the statement itself — e.g.
  `WHERE slug = $1 AND ($7::uuid IS NULL OR owner_id = $7)`
  (`src/db/documents.rs:222`). A missing match is a `404`, atomically, not a
  leak, and there is no window between checking and mutating.

**Per-handler (one shipped example).** `DELETE /media/{id}`
(`src/http/media.rs:265-285`) reads the row, compares
`principal.author_id == Some(row.owner_id)` in Rust, then calls
`media::delete_media(&state.pool, id)` — whose SQL is
`DELETE FROM media WHERE id = $1`, with no owner predicate
(`src/db/media.rs:122-127`). Two failure modes the query-layer form does not
have:

- The check and the mutation are separate statements, so it is a read-then-write
  race. Benign today (ownership never changes), but it is a correctness
  invariant maintained by nobody.
- `delete_media` is a public function with an unscoped `WHERE id = $1`. Any
  second caller — a bulk cleanup, a CLI command, a future admin route — gets a
  cross-tenant delete with no compiler complaint and no review signal.

**Therefore:** the query-layer filter is harder to get wrong, and the proposal is
to strengthen it rather than merely keep it:

- Keep `Visibility` for reads unchanged.
- For writes, replace the `Option<Uuid>` owner parameter with an explicit
  `OwnerScope::{Any, Only(Uuid)}`. `Option::None` spells "no filter" by omission,
  which is what a careless call site produces by default;
  `OwnerScope::Any` must be typed out, which makes `grep -rn 'OwnerScope::Any'`
  the complete audit list of intentional cross-tenant writes.
- Give `delete_media` an owner parameter and delete the Rust-side comparison, so
  media matches documents.
- Add one test per owner-filtered surface asserting a second author gets `404`
  (not `403` — a `403` confirms the row exists, which is itself a cross-tenant
  disclosure).

Per-handler checks are not banned; they are the wrong *default*. The narrow
legitimate use is authorization that is not expressible as a row predicate, e.g.
scope checks (`require_scope`, `src/http/auth.rs:261`).

## Alternatives considered

**Row-level security (RLS) in Postgres.** Genuinely the strongest guarantee: the
filter cannot be forgotten because the database applies it. Rejected for now on
two grounds — it needs a per-request `SET LOCAL` of the tenant id, which
interacts badly with `sqlx`'s pooled connections unless every query runs inside a
transaction; and `inkwell backup` / `inkwell restore` (ADR 0014) read and write
all 11 tables directly, so every maintenance path would need `BYPASSRLS`, moving
the risk rather than removing it. Worth revisiting if a real cross-tenant leak
ships.

**Separate database or schema per tenant.** Perfect isolation, and it makes
per-tenant backup/restore trivial. Rejected: migrations become N-way fan-out,
connection pooling degrades per tenant, and the cross-tenant reads Inkwell
actually wants later (a global search, a discovery index) become impossible
rather than merely filtered.

**Subdomain-per-owner instead of a path prefix.** Deferred, not rejected. The
schema decision (per-owner unique slug) is identical either way, and the routing
decision can be made after the migration by reading the owner from either the
`Host` header or a path segment. Recording it here so the migration is not
blocked on a hostname/TLS discussion.

**Do nothing (stay single-operator).** The honest default, and cheap today. Its
cost is stated in *Consequences* below and is not zero.

## Consequences

**If accepted (Option B):**

- The uniqueness migration must land before a second account is created. After
  that, per-owner slugs are a data merge, not a migration.
- Every public page path gains an owner dimension. That is the bulk of the
  implementation cost and it lands entirely in `src/http/pages.rs`,
  `feed.rs`, `sitemap.rs`, `search.rs`, and `src/views/`. See the blast-radius
  list in the companion plan.
- **The owner must be part of the URL, not derived from credentials.** This falls
  out of the shared cache, independently of the slug namespace: `src/http/cache.rs`
  keys its ETag on route + body only and sends `Cache-Control: public`, so a page
  that varied by caller on a shared route would let a cache serve one tenant's
  page to another. `src/http/search.rs:56-71` already documents that constraint
  and keeps the HTML search page public-only because of it. Two independent
  arguments therefore point at the same namespace decision.
- `GET /media/{id}` remains public and unauthenticated by design (embeds in
  published notes must work), which means media ids are a cross-tenant read
  channel by construction. Acceptable — ids are random UUIDs and the bytes are
  already served to anonymous readers — but it must be a written decision rather
  than an oversight, and it is the reason media ids should never be enumerable.

**If declined (Option A or nothing):**

- Nothing breaks. The shipped model is coherent single-owner.
- The bill comes due if it is ever reversed: the longer one instance runs with
  one owner and a global slug namespace, the more third-party links and
  `slug_aliases` rows depend on unprefixed URLs, and the more the eventual
  namespace change becomes a redirect-compatibility problem on top of a schema
  problem. `ADR 0011` (slug rename → 301 alias) is the mechanism that would carry
  it, and it is per-document, not per-namespace.
- This ADR stands as the record of what was declined and at what future cost.

## Follow-up

Implementation is gated on the CYP-48 board answer and is **not** part of CYP-61.
When unblocked, the executable sequence is in
[docs/MULTI-USER-MIGRATION-PLAN.md](../MULTI-USER-MIGRATION-PLAN.md), which also
carries the endpoint-and-page blast-radius list.
