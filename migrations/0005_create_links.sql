-- Link graph for the digital garden (wikilinks now; embeds + external/webmention later).
-- One row per directed edge from a source note to a target (resolved note, or an
-- unresolved stub identified by its raw [[text]], or an external URL).
CREATE TABLE IF NOT EXISTS links (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  source_note_id uuid NOT NULL REFERENCES documents (id) ON DELETE CASCADE,
  target_kind text NOT NULL,
  target_note_id uuid REFERENCES documents (id) ON DELETE SET NULL,
  target_url text,
  target_text text,
  link_type text NOT NULL,
  context_snippet text,
  resolved boolean NOT NULL DEFAULT false,
  created_at timestamptz NOT NULL DEFAULT now(),
  CONSTRAINT links_target_kind_check CHECK (target_kind IN ('internal', 'external')),
  CONSTRAINT links_link_type_check CHECK (link_type IN ('wikilink', 'embed'))
);

-- Backlinks + bounded rename re-render: "which sources resolve to this note?"
CREATE INDEX IF NOT EXISTS links_target_note_id_idx ON links (target_note_id);
-- Stub backfill: resolve unresolved internal links when a matching slug appears.
CREATE INDEX IF NOT EXISTS links_unresolved_target_text_idx
    ON links (target_text)
    WHERE target_kind = 'internal' AND resolved = false;
-- Replace a source's outbound edges on re-render.
CREATE INDEX IF NOT EXISTS links_source_note_id_idx ON links (source_note_id);
