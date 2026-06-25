-- Migration 0021: slug aliases for rename + redirect (ADR 0011).
--
-- A document's slug used to be immutable. It is now renameable; when it changes,
-- the previous slug is recorded here so old links keep working via a 301 redirect
-- to the document's current slug.
--
-- `old_slug` is the PRIMARY KEY, so a retired slug maps to at most one document.
-- The FK CASCADEs on delete: removing a document drops its aliases with it.

CREATE TABLE slug_aliases (
    old_slug    text        PRIMARY KEY,
    document_id uuid        NOT NULL REFERENCES documents(id) ON DELETE CASCADE,
    created_at  timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX slug_aliases_document_id_idx ON slug_aliases (document_id);
