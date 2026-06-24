-- Migration 0018: add embedding provenance to note_chunks.
--
-- Existing rows are marked with provider='unknown' / model='unknown' /
-- dimensions=1024 so they are never compared against real-provider query
-- vectors. Retrieval filters on (embedding_provider, embedding_model) so only
-- chunks from the active embedder are ever compared; unknown rows are silently
-- ignored until the note is re-saved or reindexed.
ALTER TABLE note_chunks
  ADD COLUMN IF NOT EXISTS embedding_provider text NOT NULL DEFAULT 'unknown',
  ADD COLUMN IF NOT EXISTS embedding_model text NOT NULL DEFAULT 'unknown',
  ADD COLUMN IF NOT EXISTS embedding_dimensions integer NOT NULL DEFAULT 1024;

CREATE INDEX IF NOT EXISTS note_chunks_embedding_provenance_idx
  ON note_chunks (embedding_provider, embedding_model, note_id);
