-- Migration 0019: media upload table.
--
-- Stores image blobs directly in Postgres (bytea). No external object store
-- for v1. Each row is owned by the uploading author; serving is public
-- (no auth gate) so embeds work in published notes.
CREATE TABLE IF NOT EXISTS media (
    id           uuid        PRIMARY KEY DEFAULT gen_random_uuid(),
    filename     text,
    content_type text        NOT NULL,
    byte_size    integer     NOT NULL,
    data         bytea       NOT NULL,
    owner_id     uuid        NOT NULL REFERENCES authors(id) ON DELETE CASCADE,
    created_at   timestamptz NOT NULL DEFAULT now(),
    -- Defense in depth: the HTTP handler already enforces the content-type
    -- allowlist and 5 MiB cap, but enforce them at the table too so a non-HTTP
    -- insert path or ad-hoc SQL can't store active content (HTML/SVG) or an
    -- oversized blob that `GET /media/{id}` would then serve. `byte_size` must
    -- equal the actual blob length.
    CONSTRAINT media_content_type_check CHECK (
        content_type IN ('image/png', 'image/jpeg', 'image/gif', 'image/webp')
    ),
    CONSTRAINT media_size_check CHECK (
        byte_size = octet_length(data) AND byte_size BETWEEN 0 AND 5242880
    )
);
