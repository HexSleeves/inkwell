ALTER TABLE documents
  ADD COLUMN IF NOT EXISTS status text NOT NULL DEFAULT 'draft'
  CHECK (status IN ('draft', 'published'));
UPDATE documents SET status = 'published' WHERE status IS DISTINCT FROM 'draft';
