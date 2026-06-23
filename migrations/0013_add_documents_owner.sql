-- Document ownership (ADR 0009, plan 023). NULLABLE in slice 1: existing rows
-- are backfilled to the bootstrap admin author (migration 0015) and new rows are
-- left unstamped for now — no surface reads `owner_id` yet, and slice 3 turns on
-- ownership enforcement. The `NOT NULL` tightening is deliberately deferred to
-- slice 4 so this migration can never reject an existing document.
ALTER TABLE documents
  ADD COLUMN IF NOT EXISTS owner_id uuid REFERENCES authors (id);
