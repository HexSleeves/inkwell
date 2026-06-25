-- Migration 0020: browser session table for flag-gated login (ADR 0010).
--
-- Only the SHA-256 hash of the session token is stored — never the raw token.
-- This mirrors the scoped-token pattern in author_tokens (migration 0012).
--
-- sessions is intentionally minimal: no password, no registration, no profile.
-- A session is created by exchanging a valid scoped token at POST /auth/login
-- (only when INKWELL_BROWSER_LOGIN=true) and deleted on logout or expiry.

CREATE TABLE sessions (
    id                  uuid        PRIMARY KEY DEFAULT gen_random_uuid(),
    session_token_hash  text        NOT NULL UNIQUE,
    author_id           uuid        NOT NULL REFERENCES authors(id) ON DELETE CASCADE,
    -- Scopes inherited from the scoped token the session was minted from. A
    -- session must NOT grant more than its originating token (a read-only token
    -- must not become a write/publish session), so the granted scopes are stored
    -- here and applied verbatim on resolution. Admin is deliberately excluded:
    -- ADR 0010 caps browser sessions at read/write/publish, so login downscopes
    -- an admin token to this set and the CHECK enforces it as defense in depth.
    scopes              text[]      NOT NULL,
    created_at          timestamptz NOT NULL DEFAULT now(),
    expires_at          timestamptz NOT NULL,
    CONSTRAINT sessions_scopes_check
        CHECK (scopes <@ ARRAY['read','write','publish']::text[])
);

CREATE INDEX sessions_author_id_idx ON sessions (author_id);
