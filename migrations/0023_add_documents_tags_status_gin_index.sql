-- The common public listing filters on `status = 'published' AND tags @>
-- ARRAY[$1]::text[]`. A plain gin(tags) index (migration 0003) covers the tag
-- containment but leaves the status equality as a post-filter. btree_gin (a
-- standard trusted contrib extension) lets a single GIN index combine the
-- scalar `status` equality with the `tags` array containment, so that listing
-- can be served by one index scan instead of a scan + filter.
CREATE EXTENSION IF NOT EXISTS btree_gin;

CREATE INDEX IF NOT EXISTS documents_status_tags_gin_idx
    ON documents USING gin (status, tags);
