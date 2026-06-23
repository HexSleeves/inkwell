-- Document ownership (ADR 0009, plan 023). NULLABLE in slice 1: existing rows
-- are backfilled to the bootstrap admin author (migration 0015). The column has
-- no DEFAULT here; migration 0016 sets DEFAULT after the bootstrap admin is
-- seeded so that new rows created in the slice 1 → slice 2 window are
-- automatically attributed to the admin rather than landing with owner_id = NULL.
-- The `NOT NULL` tightening is deliberately deferred to slice 4.
ALTER TABLE documents
  ADD COLUMN IF NOT EXISTS owner_id uuid REFERENCES authors (id);

-- FK columns are not auto-indexed by PostgreSQL. Ownership enforcement queries
-- (slice 3) and audit joins look up documents by owner_id; without an index
-- they would sequential-scan the entire documents table for every author lookup.
CREATE INDEX IF NOT EXISTS documents_owner_id_idx ON documents (owner_id);
