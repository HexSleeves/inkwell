# ADR 0014 — Tested backup and restore

- Status: Accepted
- Date: 2026-07-25
- Ticket: CYP-49 (parent CYP-44)

## Context

Inkwell had a `pg_dump`/`pg_restore` runbook and, after
[ADR 0013](0013-media-storage.md), media bytes that may live outside the database
entirely. Neither was covered by a test. "Restore works" was a claim in a Markdown
file, which is the same as not knowing — and v0.2 should not ship a
self-hostable product whose disaster recovery has never been executed.

Three concrete gaps:

1. A complete backup needed two uncoordinated steps (`pg_dump` plus `tar` of
   `INKWELL_MEDIA_DIR`), with nothing tying the two halves together or recording
   which schema version they came from.
2. `pg_dump` and `pg_restore` are external binaries whose major version must match
   or exceed the server's. A host with `postgresql-client-16` cannot dump a pg17
   server, which makes the documented procedure fail exactly when an operator
   reaches for it under pressure — and makes it impossible to exercise from
   `cargo test` on a machine without matching client tools.
3. Nothing stopped a restore from silently overwriting a live deployment, or from
   loading a bundle written by a newer Inkwell into an older binary.

## Decision

### 1. `inkwell backup` / `inkwell restore` writing one logical bundle

A bundle is a single gzipped JSON Lines file: a manifest line, then table rows,
then media blobs. No external binaries — just the connection pool and media store
the server already has. That is what makes the acceptance test possible: seed,
publish, back up, restore into a freshly created database, and assert every read
surface answers identically.

`pg_dump` stays documented as the physical-fidelity alternative. It is still the
right tool for partial table recovery and for platform snapshot tooling; it is no
longer the only tested path.

### 2. Postgres does the serialisation

Rows are dumped with `to_jsonb` over an explicit non-generated column list and
loaded with `jsonb_populate_recordset` against the table's own row type. There is
no per-table Rust struct to keep in sync: a column added by a migration is in the
next bundle automatically, and every value is parsed by its real Postgres input
function — `bytea` hex, pgvector `[…]` literals, `timestamptz`, text arrays —
without a type-specific branch in our code.

Generated columns (`documents.search_vector`) are excluded from both directions,
because Postgres recomputes them on insert. The search index is therefore restored
by construction rather than copied.

The dump runs in one `REPEATABLE READ` transaction, so the manifest's row counts,
the rows, and cross-table foreign keys all come from one snapshot. Backups do not
require downtime.

### 3. Media blobs travel through the store trait, not as a table

`media_blobs` is deliberately **not** dumped as a table. Blobs are read via
`MediaStore::get` and written via `MediaStore::put`, base64-encoded in the bundle
and verified on the way in against the SHA-256 in their own content-addressed key.

This makes a bundle **backend-portable**: a filesystem-backed deployment restores
onto a Postgres-backed one and every `/media/{id}` URL still resolves. Dumping
`media_blobs` as a table would instead have doubled the bundle for Postgres-backed
deployments and produced nothing restorable for filesystem-backed ones.

### 4. Refuse rather than guess

- **Non-empty target.** Without `--overwrite`, a restore into a deployment holding
  any data beyond the migration-seeded bootstrap admin aborts before the first
  write. Destructive is opt-in, never the default you get by forgetting a flag.
- **Newer schema.** A bundle whose schema version exceeds what the binary knows is
  refused by name and number, before migrations run. The reverse direction (older
  bundle into newer binary) is supported and warns about each column that will
  take a schema default.
- **Corrupt blob.** Bytes that do not hash to their key abort the restore.

All database writes happen in one transaction. Blob writes precede the commit —
safe because content addressing makes them idempotent and additive, so a failed
restore leaves extra unreferenced bytes, never wrong ones. Blob *deletion*
(superseded images after `--overwrite`) happens only after the commit succeeds, so
a failed restore never strands you with neither the old data nor the new.

## Consequences

- Backup and restore are one command each and covered by
  `tests/backup_restore_contract.rs`, which rehearses the full
  seed → publish → backup → clean volume → restore → compare loop.
- Bundles are ~33% larger than raw for the media portion (base64) before gzip.
  Acceptable at v0.2's scale; a binary side-car format is the escape hatch if it
  stops being.
- A restore holds the whole batch (500 rows) in memory, not the whole bundle. Very
  large individual media blobs are the only unbounded allocation, and those are
  already capped by `INKWELL_MEDIA_MAX_BYTES`.
- Bundles contain **no** configuration or secrets, deliberately: they get copied
  to laptops and object stores, and a bundle carrying `INKWELL_API_KEY` would be a
  credential leak waiting to happen. Restoring a deployment means restoring data
  *and* re-providing environment.
- The bundle format is versioned (`bundleFormat`). Changing it in a way an older
  reader could misinterpret requires a bump, and older readers refuse rather than
  mis-parse.
