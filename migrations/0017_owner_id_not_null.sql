-- Tighten document ownership (ADR 0009, plan 023, slice 4). By slice 4 every
-- creation path supplies an owner: the write API stamps `owner_id` from the
-- request principal (slice 3) and `create_document` COALESCEs a missing value to
-- the bootstrap admin, so no row is inserted with a NULL owner. Backfill from
-- slice 1 (migration 0015) already claimed all pre-existing rows for the admin.
--
-- A defensive backfill runs first so this migration is safe even if some row
-- slipped through with NULL (e.g. a manual insert), then the column is made
-- NOT NULL.
--
-- NOTE: the DB-level DEFAULT (bootstrap admin, added in 0016) is deliberately
-- KEPT rather than dropped. The write API stamps `owner_id` explicitly, but
-- other insert paths (seed, tests, maintenance scripts) legitimately omit it;
-- the default makes those attribute to the bootstrap admin instead of violating
-- NOT NULL. NOT NULL — the actual slice-4 goal (every note has an owner) — is
-- still enforced; the default is belt-and-suspenders, not a correctness gap.
UPDATE documents
   SET owner_id = '00000000-0000-0000-0000-000000000001'
 WHERE owner_id IS NULL;

ALTER TABLE documents
  ALTER COLUMN owner_id SET NOT NULL;
