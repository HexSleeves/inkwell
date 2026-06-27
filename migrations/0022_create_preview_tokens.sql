-- Preview tokens (CIL-129): shareable read-only links that grant access to a
-- specific draft document without exposing author credentials.
--
-- Design:
--   * `prefix`     — public, non-secret lookup handle (UNIQUE); stored verbatim.
--   * `token_hash` — SHA-256 of the full `pvw_<prefix>_<secret>` token; the
--                    raw secret is NEVER stored; auth recomputes the hash and
--                    compares in constant time.
--   * `expires_at` — optional hard expiry; NULL means no expiry.
--   * `revoked_at` — soft-delete for revocation; NULL = live.
--   * ON DELETE CASCADE so deleting a document cleans up its tokens; a dangling
--     token for a deleted document would always fail the document lookup anyway,
--     but CASCADE makes the invariant explicit and keeps the table tidy.

CREATE TABLE preview_tokens (
    id          UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    document_id UUID        NOT NULL REFERENCES documents(id) ON DELETE CASCADE,
    prefix      TEXT        NOT NULL UNIQUE,
    token_hash  TEXT        NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at  TIMESTAMPTZ,
    revoked_at  TIMESTAMPTZ
);

-- Efficient listing of tokens for a given document (list/revoke surface).
CREATE INDEX preview_tokens_document_id_idx ON preview_tokens (document_id);
