-- Swap the note_chunks ANN index from IVFFlat to HNSW for cosine distance
-- (`<=>`). HNSW gives better recall/latency at query time without the IVFFlat
-- "lists" tuning + the need to ANALYZE before the index is useful, and it stays
-- valid as the garden grows (no re-train as rows accumulate). The pgvector
-- `vector` extension already exists (created in migration 0009), so the hnsw
-- access method is available here. Plain CREATE INDEX (NOT CONCURRENTLY) because
-- sqlx runs each migration inside a transaction.
CREATE INDEX IF NOT EXISTS note_chunks_embedding_hnsw_idx
  ON note_chunks USING hnsw (embedding vector_cosine_ops) WITH (m = 16, ef_construction = 64);

-- Retire the now-redundant IVFFlat index (created in migration 0009); the HNSW
-- index above supersedes it for cosine-distance related-notes retrieval.
DROP INDEX IF EXISTS note_chunks_embedding_idx;
