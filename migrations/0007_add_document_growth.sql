-- Digital-garden maturity stage for a note. Mirrors the `status` column's
-- text + CHECK pattern (0002): a small closed vocabulary stored as text so the
-- domain `GrowthStage` enum decodes it directly. Defaults to 'seedling' (a new,
-- rough note); authors promote to 'budding' then 'evergreen' as it matures.
ALTER TABLE documents
  ADD COLUMN IF NOT EXISTS growth text NOT NULL DEFAULT 'seedling'
  CHECK (growth IN ('seedling', 'budding', 'evergreen'));
