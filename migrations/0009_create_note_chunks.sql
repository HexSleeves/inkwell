-- Semantic-search substrate (card T10, P3). pgvector ships in the dev/CI image
-- (pgvector/pgvector:pg17), so the extension creates cleanly; CREATE EXTENSION
-- IF NOT EXISTS is idempotent and a no-op once present.
CREATE EXTENSION IF NOT EXISTS vector;

-- One row per embedded chunk of a note. A SEPARATE table (not a column on
-- documents) so a note can have many chunks, the embedding lifecycle is
-- decoupled from the document row, and re-embedding just replaces this note's
-- chunk rows. The dimension matches the Voyage model we target (voyage-3 =
-- 1024 dims, mirrored by `crate::ai::EMBEDDING_DIMENSIONS`). ON DELETE CASCADE
-- so deleting a note reaps its chunks.
CREATE TABLE IF NOT EXISTS note_chunks (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  note_id uuid NOT NULL REFERENCES documents (id) ON DELETE CASCADE,
  chunk_index integer NOT NULL,
  content text NOT NULL,
  embedding vector(1024) NOT NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  CONSTRAINT note_chunks_note_id_chunk_index_key UNIQUE (note_id, chunk_index)
);

-- Replace a note's chunk set on re-embed (best-effort, mirrors edge replace).
CREATE INDEX IF NOT EXISTS note_chunks_note_id_idx ON note_chunks (note_id);

-- Approximate-nearest-neighbour index over cosine distance (`<=>`). IVFFlat is
-- available on the pgvector image and keeps related-notes / ask-your-garden
-- retrieval bounded as the garden grows.
CREATE INDEX IF NOT EXISTS note_chunks_embedding_idx
  ON note_chunks USING ivfflat (embedding vector_cosine_ops) WITH (lists = 100);
