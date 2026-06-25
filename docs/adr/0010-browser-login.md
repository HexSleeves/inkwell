# ADR 0010: Flag-Gated Browser Session Login

Status: accepted

## Context

ADR 0009 (scoped author tokens) chose Option B — API bearer tokens — and
explicitly deferred Option C (browser session / login UI) because it required a
larger product and UX scope that was not needed at the time.

The immediate goal is now narrower: allow a human author to log in to the
Inkwell admin surface from a browser without copying a raw `ink_*` token into
every HTTP request. A browser session (httpOnly cookie) is the standard answer.
However, browser sessions introduce new attack surfaces (CSRF, session fixation,
cookie theft) that must be addressed carefully and incrementally.

This ADR ships the **smallest safe first step**: a flag-gated scaffold that is
**completely off by default**, so production is unaffected until the operator
explicitly opts in.

## Non-goals (deferred to future ADRs)

- A registration UI (sign-up form, email verification).
- Password-based authentication (password storage, hashing, reset flow).
- OAuth / third-party identity providers.
- Session renewal / sliding expiry.
- Per-session scope selection in the login UI.
- IP binding or device fingerprinting on sessions.
- A browser-facing admin UI (this ADR only adds the auth mechanism, not the UI).

## Decision

Implement a minimal, flag-gated browser session layer that:

1. Is **off by default** (`INKWELL_BROWSER_LOGIN`, parsed like
   `INKWELL_WEBMENTION_SEND`). When the flag is off, the login routes are not
   registered (returning 404) and the existing auth paths are byte-for-byte
   unchanged.
2. Reuses the **existing scoped token** as the login credential: `POST
   /auth/login` accepts `{ "token": "ink_…" }` and validates it via the same
   `find_token_by_prefix` + constant-time hash compare path already used in
   `auth::authenticate`. This means no new credential type is introduced; an
   author who already has a scoped token can immediately log in without any
   registration step.
3. Issues a browser session (a random 256-bit token stored only as a SHA-256
   hash in a new `sessions` table, mirroring the scoped-token hash pattern)
   returned via `Set-Cookie: inkwell_session=…; HttpOnly; Secure;
   SameSite=Strict`.
4. Extends `auth::authenticate` to resolve, when the flag is on and no
   `x-api-key` is present, a valid `inkwell_session` cookie to the owning
   author's `Principal` carrying the scopes the session inherited from its
   minting token (capped to read/write/publish at login — see "Admin sessions").
   The existing `x-api-key` path is unchanged and always evaluated first.
5. Exposes `POST /auth/logout` to delete the session row and clear the cookie.

## Why reuse scoped tokens instead of adding passwords?

- **Zero new credential surface.** Authors already hold `ink_*` tokens. Adding
  passwords would require password hashing (argon2/bcrypt), storage, and a reset
  flow — all of which are non-trivial to do securely and entirely unnecessary
  if the goal is "let a human use their existing token from a browser".
- **Consistent revocation model.** Revoking a scoped token immediately
  invalidates the ability to *create* new sessions with it; any existing
  sessions from that token live until they expire or are explicitly logged out.
  (If stronger coupling is needed, sessions could be purged on token revocation
  in a future slice.)
- **Simpler trust model.** A session is a time-bounded delegation from a token.
  The token is the root credential; the session is a convenience wrapper for
  browser use.

## Security properties

### Cookie attributes
`HttpOnly` — the session token is never readable by page JavaScript, preventing
XSS-based token exfiltration.

`Secure` — the cookie is only transmitted over HTTPS, preventing interception
on plain HTTP. (Local development without TLS must be tested with a flag-off
config or via a local TLS proxy.)

`SameSite=Strict` — the cookie is not sent on cross-site navigations or
requests, which is the primary CSRF mitigation. Combined with the fact that the
`POST /auth/login` endpoint requires a JSON body (not form-encoded), a classic
CSRF attack cannot trigger a login. Similarly, state-changing endpoints require
the session cookie which `SameSite=Strict` withholds from cross-origin requests.

### CSRF considerations
SameSite=Strict is the primary CSRF defense and is sufficient for the current
threat model. An additional CSRF token is not required because:
- All state-changing `POST` endpoints require a JSON `Content-Type` body, which
  browsers do not send cross-site without CORS permission (and we do not relax
  CORS).
- `SameSite=Strict` prevents the cookie from being attached to cross-origin
  requests entirely.

A double-submit cookie pattern or explicit CSRF token should be added if
`SameSite=Lax` is ever required (e.g., to support OAuth redirects). That is
deferred.

### Session token entropy
Session tokens are 64 hex characters derived from two independent v4 UUIDs
(≈ 244 bits of effective entropy). Only the SHA-256 hash is stored. A database
compromise does not expose raw session tokens.

### Session expiry
Sessions expire after 7 days (`expires_at`). The expiry is checked in
`auth::authenticate` on every request. Expired sessions are not automatically
purged; a maintenance sweep can delete rows where `expires_at < now()`.

### Token-to-session coupling
A revoked scoped token cannot be used to create new sessions. The guard is
atomic: the session `INSERT` carries a `WHERE EXISTS (… author_tokens … NOT
revoked)` predicate, so a `revoke_token` that commits between the login lookup
and the insert results in zero rows inserted and a `401` — there is no
check-then-insert TOCTOU window. Existing sessions created before revocation
remain valid until they expire or the user logs out. This is intentional:
strong coupling (auto-expiring sessions on token revocation) is deferred; if
required, a session.token_id FK can be added later.

### Admin sessions
A session inherits EXACTLY the scopes of the token it was minted from, stored in
`sessions.scopes` and applied verbatim on resolution — a `read`-only token never
becomes a write/publish session (no upward privilege escalation). Those scopes
are additionally capped to `read`/`write`/`publish` at login: an `admin`-scoped
token is downscoped (admin is filtered out before the row is written, and a DB
CHECK enforces it as defense in depth), so a browser session can never hold
`Admin`. Admin operations (token management, etc.) still require the shared
`INKWELL_API_KEY` or an `admin`-scoped API token. This preserves least privilege
for browser sessions.

### Flag-off safety
When `INKWELL_BROWSER_LOGIN` is false (the default):
- The `/auth/login` and `/auth/logout` routes are not registered → 404.
- `auth::authenticate` never consults the `Cookie` header.
- The `sessions` table is created by migration 0020 but is never read or written.
- No behavior change from the perspective of existing callers.

## Session table schema

See `migrations/0020_create_sessions.sql`.

```
sessions
  id                 uuid        PK  DEFAULT gen_random_uuid()
  session_token_hash text        NOT NULL  UNIQUE
  author_id          uuid        NOT NULL  REFERENCES authors(id) ON DELETE CASCADE
  scopes             text[]      NOT NULL  CHECK (scopes <@ ARRAY['read','write','publish'])
  created_at         timestamptz NOT NULL  DEFAULT now()
  expires_at         timestamptz NOT NULL
```

`scopes` carries the (downscoped) capabilities the session inherited from its
minting token; the CHECK guarantees no browser session can hold `admin`.

Only the SHA-256 hex of the raw session token is stored. The raw token is
transmitted once in the `Set-Cookie` response and never persisted.

## Routes (when flag is on)

| Method | Path           | Auth required | Description                                     |
|--------|----------------|---------------|-------------------------------------------------|
| POST   | /auth/login    | none          | Exchange a scoped token for a session cookie    |
| POST   | /auth/logout   | none          | Delete the session, clear the cookie            |

## Authentication resolution order (when flag is on)

1. `x-api-key: <shared-key>` → admin `Principal` (unchanged)
2. `x-api-key: ink_*` → scoped-token `Principal` (unchanged)
3. `Cookie: inkwell_session=<token>` → session `Principal` (new, flag-gated)
4. No credential → `None` (public or 401 depending on the route)

## Consequences

- Adds one migration (0020) and three new source files
  (`src/db/sessions.rs`, `src/http/auth_session.rs`) plus config / router
  wiring.
- No existing behavior changes when the flag is off.
- Opens a path toward a browser admin UI without committing to its full design.
- Defers passwords, registration, OAuth, and CSRF token pattern to later ADRs.
