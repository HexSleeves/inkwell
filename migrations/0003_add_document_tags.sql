ALTER TABLE documents
  ADD COLUMN IF NOT EXISTS tags text[] NOT NULL DEFAULT '{}';
CREATE INDEX IF NOT EXISTS documents_tags_idx ON documents USING gin (tags);
