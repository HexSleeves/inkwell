# ADR 0011 — Slug rename with 301 alias redirect

- Status: Accepted
- Date: 2026-06-25
- Supersedes/relates: ADR 0009 (scoped author tokens — rename respects ownership
  and scopes), slice 3b (owner-aware read visibility — alias redirects never leak
  drafts).

## Context

A document's `slug` was immutable after creation. But slugs are derived from
titles, and titles get corrected. Without a rename path, fixing a slug means
deleting and recreating the note — which breaks every existing inbound link,
wikilink, and bookmark to the old URL. A garden that silently 404s its own old
links is a broken garden.

## Decision

Make the slug **renameable**, and keep old URLs working with a permanent
redirect:

1. **`slug_aliases` table** (`migrations/0021`): `old_slug` (PK) → `document_id`
   (FK, `ON DELETE CASCADE`). A retired slug maps to exactly one document and is
   removed automatically when the document is deleted.
2. **Rename via the existing update route.** `PUT/PATCH /documents/{slug}` accepts
   a `"slug"` field. A value that differs from the current slug triggers a rename;
   a value equal to it is a no-op. The slug is format-validated (`is_valid_slug`)
   up front — a bad value is a `400` before any DB work.
3. **Atomic, owner-enforced rename.** `rename_and_update` runs in one transaction
   under a `SELECT … FOR UPDATE` lock on the target row:
   - non-owner / missing slug → `404` (no existence leak, same as other mutations);
   - destination slug already live → `409 Conflict`;
   - otherwise: upsert `old_slug → id` into `slug_aliases`, delete any alias equal
     to the *new* slug (so renaming back to a retired slug can't form a loop), then
     change `documents.slug` and apply any field patch with a single version bump.
   It composes with optimistic concurrency: the `If-Match` version is checked
   against the locked row, so a rename can't clobber a concurrent edit.
4. **Redirect on read.** When a slug isn't found, both the JSON route
   (`GET /documents/{slug}`) and the HTML page (`GET /{slug}`) consult
   `slug_aliases` and return **301 Moved Permanently** with `Location` pointing at
   the document's *current* slug. Because aliases store the document id (not a
   slug), a chain of renames `a → b → c` resolves `a` and `b` straight to `c`.

## No draft leak

Alias resolution is visibility-scoped exactly like a normal read: the redirect
only fires when the target document is visible to the caller. An anonymous
request for the retired slug of an unpublished note gets a `404`, not a `301` —
the redirect would otherwise confirm the draft exists. Owners still get the
redirect for their own drafts; admins for everything.

## Alternatives considered

- **Hard 404 on the old slug** — simplest, but breaks every existing link. Rejected.
- **Client/JS redirect** — needs the old page to still render and ships logic to
  the browser; a server 301 is cacheable and works for API clients and crawlers.
- **Store the alias as `old_slug → new_slug` (string, not id)** — would need a
  cascade-rewrite of every alias on each subsequent rename to avoid stale chains.
  Pointing at the document id makes chained renames resolve in one hop.

## Consequences

- Old URLs survive renames; SEO/link equity transfers via 301.
- One extra table and a lookup only on the 404 path (no cost to live-slug reads).
- A future "vanity slug" or bulk-rename feature can build on the same alias table.
