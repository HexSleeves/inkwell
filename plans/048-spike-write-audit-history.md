# Plan 048: Design spike — expose the write-audit trail as a history API

> **Executor instructions**: This is a DESIGN SPIKE plus a thin first implementation.
> Produce the design doc, then implement the read-only endpoint if Step 5 greenlights.
> Run every verification command. When done, update the status row in `plans/README.md`.
>
> **Drift check (run first)**: `git diff --stat 0819727..HEAD -- migrations/0014_create_write_audit.sql src/db/audit.rs src/http/api.rs src/http/router.rs`
> If any changed, re-read them before designing.

## Status

- **Priority**: P3
- **Effort**: M
- **Risk**: LOW
- **Depends on**: none
- **Category**: direction
- **Planned at**: commit `0819727`, 2026-06-26

## Why this matters

Every successful mutation already writes an append-only audit row (`write_audit`, migration 0014), indexed for "what happened to this document, newest first" — but there is no way to read it back. An author cannot see the history of a note, recover context after an accidental delete, or audit who changed what. The data and the index already exist; the only missing piece is a read endpoint. This is a high-value, low-cost capability: the expensive part (durable, indexed audit capture) is done.

## Current state

**`migrations/0014_create_write_audit.sql`** — the table:
```sql
CREATE TABLE IF NOT EXISTS write_audit (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  actor_author_id uuid REFERENCES authors (id) ON DELETE SET NULL,
  actor_label text NOT NULL,
  action text NOT NULL,          -- create|update|delete|publish|unpublish
  document_id uuid,              -- intentionally NOT an FK (survives document delete)
  slug text,
  at timestamptz NOT NULL DEFAULT now(),
  CONSTRAINT write_audit_action_check CHECK (action IN ('create','update','delete','publish','unpublish'))
);
CREATE INDEX IF NOT EXISTS write_audit_document_id_at_idx ON write_audit (document_id, at DESC);
```

**`src/db/audit.rs`** — currently only an insert path (`record_write`) and the `AuditAction` enum. There is **no read function** — you will add one (`list_audit_for_document`).

**Write path** stores `actor_label` ('shared-key' for admin, else author name) and `actor_author_id`. There is no body snapshot — the audit records *that* a change happened and by whom, not the before/after content. So a "history" endpoint returns an event log, not a diff/version-restore.

**Handler conventions**: handlers in `src/http/api.rs` return `Result<Response, AppError>`, derive visibility via `resolve_visibility`, and serialize camelCase response structs. DB reads go in `src/db/`. Routes register in `src/http/router.rs`.

**Visibility/auth decision needed**: who may read a document's history? Options to weigh in the spike:
- Admin only (simplest, safest).
- Admin + the document's owner (an author sees their own note's history).
- Public for published notes (probably not — exposes actor labels).

## Commands you will need

| Purpose   | Command                                                  | Expected on success |
|-----------|--------------------------------------------------------|---------------------|
| Typecheck | `cargo check --all-targets`                             | exit 0              |
| Tests     | `DATABASE_URL=... cargo nextest run --test audit_history_contract` | all pass |
| All tests | `DATABASE_URL=... cargo test --all`                    | all pass            |
| Lint      | `cargo clippy --all-targets -- -D warnings`            | exit 0              |

## Scope

**In scope**:
- `docs/spikes/0NN-write-audit-history.md` (NEW — next spike number) — design doc
- `src/db/audit.rs` — add `list_audit_for_document` read function
- `src/http/api.rs` (or a small new `src/http/history.rs`) — add `GET /documents/{slug}/history` handler
- `src/http/router.rs` — register the route
- `tests/audit_history_contract.rs` (NEW) — contract test

**Out of scope**:
- Body/diff snapshots — the audit table has no content snapshots; adding them is a separate, larger feature. Note it as a future option; do not build it.
- A history UI — API only.
- Changing the write-audit capture path (`record_write`) — read-only addition.

## Steps

### Step 1: Write the design doc

Create `docs/spikes/0NN-write-audit-history.md`:
1. **What the endpoint returns** — a list of audit events `{ action, actorLabel, at }` for a document, newest first, paginated.
2. **Auth/visibility decision** — pick one of the options above and justify it. **Recommended**: admin + owner (the owner sees their own note's history; admin sees all). This matches the ownership model in ADR 0009.
3. **Lookup by slug vs id** — the audit row stores `document_id` and a `slug` snapshot; the public API addresses documents by slug. Decide: resolve slug → current document_id then query by `document_id` (misses history from before a slug rename unless you also match the `slug` snapshot). Document the tradeoff; recommend querying by the document's current `id` (simplest; rename history is an edge case).
4. **Pagination** — `?limit=&offset=` with a default and `MAX_LIMIT` cap (reuse `MAX_LIMIT = 100` from `src/domain/document.rs`).
5. **Deleted documents** — audit rows survive deletes (no FK). Decide whether `GET /documents/{slug}/history` 404s once the document is gone (recommended: yes — the slug no longer resolves) and note that a future admin-only "all audit" endpoint could expose deleted-document history.

**Verify**: The doc states the auth model, the lookup strategy, and pagination.

### Step 2: Add the DB read function

In `src/db/audit.rs`, add:
```rust
pub struct AuditEntry {
    pub action: String,
    pub actor_label: String,
    pub at: time::OffsetDateTime,
}

pub async fn list_audit_for_document(
    pool: &PgPool,
    document_id: Uuid,
    limit: i64,
    offset: i64,
) -> Result<Vec<AuditEntry>, sqlx::Error> {
    sqlx::query_as::<_, (String, String, time::OffsetDateTime)>(
        "SELECT action, actor_label, at FROM write_audit \
         WHERE document_id = $1 ORDER BY at DESC LIMIT $2 OFFSET $3",
    )
    .bind(document_id).bind(limit).bind(offset)
    .fetch_all(pool)
    .await
    .map(|rows| rows.into_iter().map(|(action, actor_label, at)| AuditEntry { action, actor_label, at }).collect())
}
```
Match the existing turbofish-query style in `src/db/` (no compile-time macros are used in this repo).

**Verify**: `cargo check --all-targets` → exit 0

### Step 3: Add the handler

Add `GET /documents/{slug}/history`:
1. Resolve the slug to a document with the visibility the spike decided (admin + owner). Reuse `resolve_visibility` / the ownership helpers in `src/http/api.rs`. If the caller is neither admin nor the owner → 404 (do not reveal existence).
2. Parse `limit`/`offset` (cap at `MAX_LIMIT`).
3. Call `audit::list_audit_for_document(pool, document.id, limit, offset)`.
4. Serialize a camelCase response: `{ "slug": ..., "history": [ { "action", "actorLabel", "at" }, ... ] }`. Use the repo's RFC3339 timestamp serde (`crate::domain::document::timestamp`) for `at`.

**Verify**: `cargo check --all-targets` → exit 0

### Step 4: Register the route

In `src/http/router.rs`, add (near the other `/documents/{slug}/...` routes):
```rust
.route("/documents/{slug}/history", any(api::document_history))
```
(or `history::document_history` if you put the handler in a new module).

**Verify**: `cargo check --all-targets` → exit 0

### Step 5: Add a contract test

Create `tests/audit_history_contract.rs` (model structure on `tests/scoped_tokens_slice3b.rs`):
1. Create a doc as admin (`SHARED_KEY`), then PATCH it, then publish it.
2. `GET /documents/{slug}/history` as admin → assert 200, body `history` array has ≥ 3 entries with actions `create`, `update`, `publish` (newest first → `publish` first).
3. As an unrelated author token → assert 404 (not the owner, not admin).
4. (If owner-read is in the design) create a doc as author A, read its history as author A → 200.

**Verify (with DB)**: `cargo nextest run --test audit_history_contract` → all pass

## Test plan

- New `tests/audit_history_contract.rs`: admin reads full history (ordered, all actions); non-owner gets 404; owner reads own (if in scope).
- The endpoint is read-only and append-only-backed, so no mutation-safety tests needed.

## Done criteria

- [ ] `docs/spikes/0NN-write-audit-history.md` exists with auth model, lookup strategy, pagination decided
- [ ] `list_audit_for_document` added to `src/db/audit.rs`
- [ ] `GET /documents/{slug}/history` handler + route registered
- [ ] Visibility enforced: non-owner non-admin → 404
- [ ] `cargo check --all-targets`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check` all exit 0
- [ ] With DB: `cargo nextest run --test audit_history_contract` passes
- [ ] `plans/README.md` status row updated; if the endpoint is a stable surface, note it for `docs/COMPATIBILITY.md` + `docs/API.md` (coordinate with plan 044)

## STOP conditions

- The ownership helpers needed to gate history reads are entangled with `api.rs` in a way that makes a clean reuse hard (e.g. only available after the 039 split). If so, deliver the design doc + DB function and report that the handler needs 039 first.
- The visibility decision turns out to conflict with ADR 0009's ownership model. Re-read the ADR; follow it.
- Slug-rename history coverage is required by the maintainer (out of the recommended simple approach). That expands scope — note it and stop at the simple version.

## Maintenance notes

- This is the read side of an already-captured trail. If `record_write` ever starts storing body snapshots, a richer `GET .../history` (with diffs) becomes possible — note that as a future enhancement.
- If a new mutating action is added, extend the `write_audit_action_check` constraint (migration) AND the `AuditAction` enum; the history endpoint surfaces it automatically.
- If this endpoint becomes a documented stable surface, add it to `docs/API.md`, `docs/COMPATIBILITY.md`, and `docs/openapi.yaml`.
