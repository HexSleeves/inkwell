-- Seed the bootstrap admin author at a FIXED uuid and backfill existing document
-- ownership (ADR 0009, plan 023, slice 1). The shared `INKWELL_API_KEY` acts as
-- this principal until per-author tokens exist (slice 2); existing documents are
-- assigned to it so the current single-owner garden keeps working.
--
-- Idempotent by construction: the INSERT is a no-op on conflict and the UPDATE
-- only touches rows still missing an owner, so re-running this migration (or the
-- equivalent statements) changes nothing.
INSERT INTO authors (id, name)
VALUES ('00000000-0000-0000-0000-000000000001', 'admin')
ON CONFLICT (id) DO NOTHING;

UPDATE documents
   SET owner_id = '00000000-0000-0000-0000-000000000001'
 WHERE owner_id IS NULL;
