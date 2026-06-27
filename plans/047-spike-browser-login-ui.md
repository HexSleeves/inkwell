# Plan 047: Design spike — browser login UI and feature-flag graduation

> **Executor instructions**: This is a DESIGN SPIKE, not a build-everything
> plan. The deliverable is a written design document plus a small, optional
> proof-of-concept login page — NOT a fully productionized auth UI. Produce the
> design doc first; only build the PoC page if Step 4 says to. When done, update
> the status row in `plans/README.md`.
>
> **Drift check (run first)**: `git diff --stat 0819727..HEAD -- src/http/auth_session.rs src/http/router.rs migrations/0020_create_sessions.sql`
> If any changed, re-read them before writing the design doc.

## Status

- **Priority**: P2
- **Effort**: M
- **Risk**: LOW (spike — no production behaviour change unless the PoC is wired in)
- **Depends on**: none
- **Category**: direction
- **Planned at**: commit `0819727`, 2026-06-26

## Why this matters

The browser session backend is fully implemented and flag-gated: `src/http/auth_session.rs` mints a session cookie (`HttpOnly; Secure; SameSite=Strict`), the `sessions` table exists (migration 0020), and routes `/auth/login` + `/auth/logout` register only when `INKWELL_BROWSER_LOGIN=true` (`src/http/router.rs:113-117`). What is missing is the **login HTML page** — there is no way for a human to obtain a session in a browser. This blocks two other deferred features that the maintainer wants:
- **Media upload UI** (`plans/027-media-upload-ui.md`) — needs browser auth so the uploader page can call `POST /media`.
- **Admin/token management UI**.

There is also **flag debt**: `INKWELL_BROWSER_LOGIN` has no documented graduation criteria (when does it default to on?). A spike resolves both: design the login page and define the graduation path.

## Current state

**`src/http/auth_session.rs:114-134`** — login mints a session and sets the cookie:
```rust
let created = sessions::create_session(&state.pool, resolved.author_id, prefix, &session_hash, &session_scopes, expires_at).await?;
if !created { return Err(AppError::Unauthorized); }
let cookie = format!(
    "{SESSION_COOKIE_NAME}={raw_session_token}; HttpOnly; Secure; SameSite=Strict; Path=/; Max-Age={SESSION_TTL_SECS}"
);
Ok((StatusCode::OK, [(header::SET_COOKIE, cookie)]).into_response())
```

**`src/http/router.rs:110-117`** — flag-gated routes:
```rust
if browser_login {
    router = router
        .route("/auth/login", any(auth_session::login))
        .route("/auth/logout", any(auth_session::logout));
}
```

**Already-decided constraints (do NOT re-litigate — read and honor):**
- ADR 0010 (in `.mex/context/decisions.md` / `.mex/events/decisions.jsonl`) records the browser-session decision. **Read it before designing.** It caps browser sessions at `read/write/publish` and **downscopes admin** — a browser session can never hold the `admin` scope (confirmed in `auth_session.rs`: admin tokens are downscoped when minting a session). So the login UI authenticates authors, not admins.
- The login input is a scoped author token (`ink_<prefix>_<secret>`), exchanged at `POST /auth/login`. Read `auth_session.rs` `login` handler to confirm the exact request body field name (e.g. `token`).
- Cookie attributes are already correct (`HttpOnly; Secure; SameSite=Strict`).

**View conventions**: HTML is built with Rust string formatting in `src/views/` (no templating engine). The shared layout entry is `src/views/layout.rs` with `SiteMeta`. A login page view would live in `src/views/` and be served by a handler in `src/http/`. CSP uses a per-request nonce for `<script>` (see `src/http/security_headers.rs`); any inline script on the login page must carry the nonce.

## Commands you will need

| Purpose   | Command                                              | Expected on success |
|-----------|-----------------------------------------------------|---------------------|
| Read ADR  | `grep -rn "0010\|browser.login\|session" .mex/context/decisions.md` | finds ADR 0010 |
| Typecheck (if PoC built) | `cargo check --all-targets`          | exit 0              |
| Tests (if PoC built) | `cargo nextest run --test browser_login` | all pass        |

## Scope

**In scope**:
- `docs/spikes/0NN-browser-login-ui.md` (NEW — pick the next spike number by listing `docs/spikes/`) — the design document
- OPTIONAL (only if Step 4 greenlights): `src/views/login.rs` + a handler in `src/http/auth_session.rs` serving `GET /auth/login` as an HTML form (the existing `/auth/login` is POST-only), all behind the existing `INKWELL_BROWSER_LOGIN` flag

**Out of scope**:
- Media upload UI and admin UI — separate features; this spike only unblocks them
- Changing the session backend, cookie attributes, or scope model — all decided
- Defaulting `INKWELL_BROWSER_LOGIN` to `true` — the spike *defines* when that happens; it does not do it

## Steps

### Step 1: Read the decided context

Read: ADR 0010 (decisions), `src/http/auth_session.rs` (full), `migrations/0020_create_sessions.sql`, `src/views/layout.rs` (for the layout/SiteMeta pattern), `plans/027-media-upload-ui.md` (the downstream dependency).

**Verify**: You can state (a) the exact `/auth/login` request body field, (b) the session scope cap, (c) the cookie attributes, (d) why the media UI needs this.

### Step 2: Write the design document

Create `docs/spikes/0NN-browser-login-ui.md` covering:

1. **Goal & non-goals** — login page for authors; not admin, not registration.
2. **Page design** — a minimal HTML form (token field + submit) rendered via `src/views/`, served at `GET /auth/login` (HTML) while `POST /auth/login` (existing) does the exchange. Sketch the form markup and where the nonce goes. Decide: does the form POST as `application/x-www-form-urlencoded` or `application/json`? (Check what the existing POST handler accepts — match it, or note the handler needs to accept form encoding.)
3. **CSRF posture** — `SameSite=Strict` already mitigates cross-site cookie use; document whether an additional CSRF token is needed for the login POST (a login form that sets a cookie is lower-risk, but state the reasoning).
4. **Error UX** — bad token → re-render form with an error message; never reveal whether the token existed.
5. **Logout** — link/button POSTing to `/auth/logout`.
6. **Open questions** — list anything unresolved (e.g. "should there be a `GET /auth/login?next=/media` redirect param?").

**Verify**: The doc names exact files to touch, the request encoding, and the open questions.

### Step 3: Define the flag graduation criteria

In the same doc, add a "Graduation of `INKWELL_BROWSER_LOGIN`" section answering: what must be true before the flag defaults to `true`? Propose concrete criteria, e.g.:
- Login + logout pages shipped and tested (`tests/browser_login.rs` extended to cover the HTML form path).
- Media upload UI or admin UI consuming the session (so the flag has a user-facing payoff).
- Security review of the session cookie + CSRF posture under flag-on.
- Documentation in `docs/DEPLOYMENT.md` for operators.

Record this so the flag does not become permanent dead infrastructure (the D04 finding).

**Verify**: The criteria are concrete and checkable, not "when it's ready".

### Step 4: Decide whether to build the PoC now

Based on the design, make a recommendation: build the minimal `GET /auth/login` HTML form now (behind the flag, default off), or defer until the media UI work begins. State the recommendation in the doc.

- If building now: implement `src/views/login.rs` + the `GET /auth/login` handler (extend `auth_session.rs`), register it behind `browser_login` in `router.rs`, and add a test to `tests/browser_login.rs` that with the flag on, `GET /auth/login` returns 200 HTML containing a token field, and with the flag off returns 404. Keep it minimal.
- If deferring: leave production code untouched; the doc is the deliverable.

**Verify**: If PoC built — `cargo check --all-targets` exits 0 and `cargo nextest run --test browser_login` passes. If deferred — no `src/` changes.

## Test plan

- If PoC built: `tests/browser_login.rs` gains a test for `GET /auth/login` (200 + form HTML when flag on; 404 when off). Model it on the existing flag-on/flag-off tests already in that file.
- If deferred: no tests; the spike doc is reviewed by a human.

## Done criteria

- [ ] `docs/spikes/0NN-browser-login-ui.md` exists with: page design, CSRF posture, error UX, and flag graduation criteria
- [ ] The doc honors ADR 0010 (author-scoped sessions, no admin) — does not propose contradicting it
- [ ] A clear build-now-or-defer recommendation is recorded
- [ ] If PoC built: `cargo check --all-targets` and `cargo nextest run --test browser_login` pass; routes are flag-gated
- [ ] If deferred: no `src/` files modified
- [ ] `plans/README.md` status row updated

## STOP conditions

- ADR 0010 contradicts a design choice you were about to make (e.g. it forbids token-in-form). Follow the ADR; note the conflict.
- The existing `POST /auth/login` handler accepts a request shape that the HTML form cannot produce without a backend change larger than a small handler tweak. Document this as an open question; do not expand scope.
- Building the PoC would require touching the session backend or cookie logic. STOP — that is decided/out of scope.

## Maintenance notes

- When the media upload UI (plan 027) is built, it depends on this login flow; cross-reference the two.
- Track the graduation criteria as a checklist in the spike doc; flip `INKWELL_BROWSER_LOGIN`'s default only when all are met, with an ADR amendment.
