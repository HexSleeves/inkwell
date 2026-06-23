-- Append-only write-audit trail (ADR 0009, plan 023). One row per successful
-- mutating action (create|update|delete|publish|unpublish). `actor_label` is
-- 'shared-key' for the bootstrap/admin shared-key principal (the only principal
-- in slice 1), else the author name once tokens land (slice 2).
--
-- `document_id` is intentionally NOT a foreign key: an audit row must survive the
-- deletion of the document it describes (a delete is exactly the event we most
-- want to retain). The `slug` snapshot is likewise retained for review.
CREATE TABLE IF NOT EXISTS write_audit (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  -- ON DELETE SET NULL keeps the append-only trail intact if an author is ever
  -- removed: the audit row survives (actor_author_id goes NULL) rather than the
  -- default RESTRICT blocking the author's deletion. `actor_label` retains who
  -- it was. (`actor_author_id` is already Option<Uuid> in the app.)
  actor_author_id uuid REFERENCES authors (id) ON DELETE SET NULL,
  actor_label text NOT NULL,
  action text NOT NULL,
  document_id uuid,
  slug text,
  at timestamptz NOT NULL DEFAULT now(),
  CONSTRAINT write_audit_action_check
    CHECK (action IN ('create', 'update', 'delete', 'publish', 'unpublish'))
);

-- Audit review is "what happened to this document, newest first".
CREATE INDEX IF NOT EXISTS write_audit_document_id_at_idx
    ON write_audit (document_id, at DESC);
