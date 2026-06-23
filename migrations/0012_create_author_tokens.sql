-- Scoped author tokens (ADR 0009, plan 023). Created now as foundation; UNUSED
-- until slice 2 wires up `authenticate()` + `Principal`. An opaque token is
-- `ink_<prefix>_<secret>`: only a SHA-256 `token_hash` is ever stored (never the
-- token). Lookup is by the unique `prefix`, then a constant-time compare of
-- `sha256(provided)` against `token_hash`; a row with `revoked_at` set is dead.
-- `scopes` is a closed vocabulary (read|write|publish|admin) enforced in the
-- domain `Scope` enum rather than a DB CHECK, since the array is variable-length.
CREATE TABLE IF NOT EXISTS author_tokens (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  author_id uuid NOT NULL REFERENCES authors (id) ON DELETE CASCADE,
  token_hash text NOT NULL,
  prefix text NOT NULL UNIQUE,
  scopes text[] NOT NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  last_used_at timestamptz,
  revoked_at timestamptz
);

-- Token resolution (slice 2) looks a row up by its unique `prefix`; the UNIQUE
-- constraint already backs that lookup with an index, so no extra index here.
