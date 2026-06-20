CREATE INDEX IF NOT EXISTS documents_status_created_at_id_idx
    ON documents (status, created_at DESC, id DESC);
