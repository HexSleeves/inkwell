-- Inbound Webmentions (card T11, federation P3). One row per received mention:
-- a remote `source_url` claiming to link to a local note (`target_note_id`).
-- Mentions are recorded `pending` on receipt and flipped to `verified` only
-- after an async, SSRF-hardened fetch confirms the source actually links to the
-- target; an unverifiable source is dropped (the row is deleted), never stored
-- as verified. ON DELETE CASCADE so deleting a note reaps its mentions.
CREATE TABLE IF NOT EXISTS webmentions (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  source_url text NOT NULL,
  target_note_id uuid NOT NULL REFERENCES documents (id) ON DELETE CASCADE,
  status text NOT NULL DEFAULT 'pending',
  created_at timestamptz NOT NULL DEFAULT now(),
  CONSTRAINT webmentions_status_check CHECK (status IN ('pending', 'verified')),
  -- A given (source, target) pair is one mention: re-sending the same webmention
  -- updates the existing row rather than piling up duplicates.
  CONSTRAINT webmentions_source_target_key UNIQUE (source_url, target_note_id)
);

-- Surface a note's verified mentions (the "mentions" read path) by target.
CREATE INDEX IF NOT EXISTS webmentions_target_note_id_idx
    ON webmentions (target_note_id);
