-- Monotonic per-note version. Powers MCP If-Match optimistic concurrency,
-- re-render retry bookkeeping, and (later) RAG embedding freshness. Bumped on
-- every content update by the write path (wired in a later step).
ALTER TABLE documents ADD COLUMN IF NOT EXISTS version bigint NOT NULL DEFAULT 1;
