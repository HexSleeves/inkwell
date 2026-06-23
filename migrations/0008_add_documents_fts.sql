-- Full-text search over published (and draft, for owners) notes. A generated
-- tsvector column keeps title + body_markdown in sync automatically — no
-- trigger, no application code — and a GIN index makes `@@`/`ts_rank` queries
-- fast. The title is weighted 'A' (highest) and the body 'B' so a title match
-- ranks above a body-only match, mirroring the prior ILIKE title-first ordering.
ALTER TABLE documents
  ADD COLUMN IF NOT EXISTS search_vector tsvector
  GENERATED ALWAYS AS (
    setweight(to_tsvector('english', coalesce(title, '')), 'A')
    || setweight(to_tsvector('english', coalesce(body_markdown, '')), 'B')
  ) STORED;

CREATE INDEX IF NOT EXISTS documents_search_vector_idx
  ON documents USING GIN (search_vector);
