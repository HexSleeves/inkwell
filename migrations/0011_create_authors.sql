-- Authors: first-class principals that can own documents and hold tokens
-- (ADR 0009, plan 023, slice 1). Foundation only — no surface reads or writes
-- this table yet; slice 2 introduces token resolution and slice 3 ownership
-- enforcement. Mirrors the existing uuid-pk + timestamptz-default-now() shape
-- of `documents`/`links`.
CREATE TABLE IF NOT EXISTS authors (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  name text NOT NULL,
  created_at timestamptz NOT NULL DEFAULT now()
);
