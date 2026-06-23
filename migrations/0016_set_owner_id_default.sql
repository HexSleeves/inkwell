-- Set a DB-level DEFAULT on documents.owner_id so that any document created
-- after slice 1 deploys (but before slice 2 stamps owner_id explicitly) is
-- automatically attributed to the bootstrap admin rather than landing with
-- owner_id = NULL. Without this DEFAULT, those documents would fail slice 3
-- ownership enforcement for non-admin authors even though no user created them.
--
-- This migration runs after 0015 (which seeds the bootstrap admin), so the
-- referenced row is guaranteed to exist when the DEFAULT is evaluated.
-- Slice 4 tightens the column to NOT NULL and removes the DEFAULT once
-- explicit token-based ownership covers all creation paths.
ALTER TABLE documents
  ALTER COLUMN owner_id SET DEFAULT '00000000-0000-0000-0000-000000000001';
