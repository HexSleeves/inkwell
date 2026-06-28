# Spike 001: Browser Login UI

Status: proposed design spike

## Goal

Ship a small browser login page for authors who already have scoped Inkwell
tokens. The page lets an author exchange an `ink_<prefix>_<secret>` token for
the existing `inkwell_session` cookie, then use browser-only authoring tools
such as the future media upload UI.

This is not an admin login surface. ADR 0010 caps browser sessions at
`read`/`write`/`publish`; an `admin` token is downscoped before the session row
is written, and the `sessions_scopes_check` constraint rejects `admin` in the
database. Browser admin/token management can only be built later if it uses
non-admin browser capabilities or a separate design.

## Non-Goals

- No registration, password login, password reset, OAuth, or email flow.
- No change to session cookie attributes or token hashing.
- No change to the existing default: `INKWELL_BROWSER_LOGIN=false`.
- No media upload UI or admin UI in this spike.
- No production code in this design-only pass.

## Decided Context

- `POST /auth/login` currently accepts a JSON object with a required `token`
  string field: `{ "token": "ink_..." }`.
- A successful login sets
  `Set-Cookie: inkwell_session=<token>; HttpOnly; Secure; SameSite=Strict; Path=/; Max-Age=604800`.
- `POST /auth/logout` deletes the server-side session row when a session cookie
  is present, then clears the cookie with `Max-Age=0`.
- Routes under `/auth/login` and `/auth/logout` exist only when
  `INKWELL_BROWSER_LOGIN=true`; otherwise they are not registered and return
  404.
- `docs/adr/0010-browser-login.md` requires author-scoped sessions and forbids
  browser sessions from carrying `admin`.
- `plans/027-media-upload-ui.md` depends on this login flow because
  `POST /media` needs a `write`-scoped credential; in a browser, that should be
  the session cookie rather than a raw `x-api-key` header.

## Page Design

Add a view module at `src/views/login.rs` and a `GET /auth/login` branch in
`src/http/auth_session.rs`. Keep route registration behind the existing
`browser_login` flag in `src/http/router.rs`.

The page should use the shared public HTML shell:

- Build metadata with `SiteMeta::from_config(&state.config)`.
- Render with `src/views/layout.rs::render_page`.
- Set `HeadMeta.csp_nonce` if the page emits inline script.
- Escape any user-facing error text with `escape_html`.

The login screen is deliberately plain: one token field, one submit button, and
an error region that does not reveal whether the token prefix existed.

```html
<h1>Author login</h1>
<form class="auth-form" data-login-form>
  <label for="token">Author token</label>
  <input
    id="token"
    name="token"
    type="password"
    autocomplete="off"
    required
    spellcheck="false"
  />
  <p class="auth-error" data-login-error hidden>Invalid token.</p>
  <button type="submit">Log in</button>
</form>
<script nonce="{csp_nonce}">
  // Intercept submit, POST JSON to /auth/login, then redirect.
</script>
```

### Request Encoding

Keep the existing `POST /auth/login` request contract as JSON:
`Content-Type: application/json` with `{ "token": "..." }`.

Native HTML forms cannot submit `application/json`, so the HTML form should use
a small inline script with the per-request CSP nonce from
`src/http/security_headers.rs`. The script intercepts submit, sends `fetch` to
`/auth/login`, includes `credentials: "same-origin"`, and redirects on success.
This avoids changing the already-tested login handler.

If the project later wants a no-JavaScript login page, add explicit
`application/x-www-form-urlencoded` support to the login handler and revisit
CSRF. Do not silently switch the existing JSON endpoint to form handling.

### Redirect Behavior

Default success redirect should be `/`.

Support for `GET /auth/login?next=/media/new` is useful but should be narrowly
validated before implementation:

- Accept only same-origin absolute paths beginning with `/`.
- Reject protocol-relative URLs (`//example.com`) and full URLs.
- Fall back to `/` on invalid input.

The first production implementation can defer `next` and hardcode `/` if the
media UI route has not been designed yet.

## CSRF Posture

The current session model relies on `SameSite=Strict`, JSON request bodies, and
no permissive CORS. Keep that posture.

For login, the JSON-only contract matters: a malicious site cannot submit a
classic cross-site HTML form with `application/json`, and CORS should not allow
the attacker to send the JSON request. This also avoids "login CSRF", where an
attacker tries to force a browser into a session for the attacker's author
token.

For logout, the endpoint is `POST /auth/logout` and depends on the
`inkwell_session` cookie. `SameSite=Strict` prevents the cookie from being sent
on cross-site form posts, so a same-site logout form is acceptable. If
`SameSite` is ever relaxed to `Lax`, add an explicit CSRF token or strict
Origin/Referer validation before defaulting the flag on.

## Error UX

Bad, revoked, malformed, or missing tokens should all show the same message:

> Invalid token.

The page must not distinguish "prefix not found" from "secret mismatch" from
"revoked token". The existing POST handler already returns `401 Unauthorized`
for invalid scoped-token cases; the client should collapse all non-success auth
responses into the same message and keep the token field focused for retry.

For network errors or server errors, use a generic recoverable message:

> Login failed. Try again.

Do not echo the submitted token in the page, logs, URL, or response body.

## Logout UI

Authenticated browser tools should expose a POST-only logout control:

```html
<form method="post" action="/auth/logout">
  <button type="submit">Log out</button>
</form>
```

The logout response currently returns `200 OK` with a cleared cookie. A future
HTML-oriented logout path can redirect to `/auth/login` after clearing the
cookie, but that should be an intentional handler addition rather than a change
to the existing API semantics.

## Files to Touch When Building

- `src/views/login.rs` - new view renderer for the login form and error state.
- `src/views/mod.rs` - export the new login view module.
- `src/http/auth_session.rs` - route `GET /auth/login` to HTML and keep
  `POST /auth/login` as the JSON exchange.
- `src/http/router.rs` - keep `/auth/login` registered only inside
  `if browser_login`; `any(auth_session::login)` can stay if the handler
  branches on method, or the route can be split into explicit `get`/`post`.
- `tests/browser_login.rs` - extend flag-on/flag-off coverage for the HTML
  route.

## Test Plan for the Build

- Flag off: `GET /auth/login` returns 404.
- Flag on: `GET /auth/login` returns `200 OK`, `Content-Type: text/html`, a
  password input named `token`, and a CSP nonce on any inline script.
- Flag on: `POST /auth/login` still accepts the existing JSON body with the
  exact `token` field.
- Bad token: client-visible error stays generic and does not reveal whether the
  token exists.
- Logout form posts to `/auth/logout`; successful logout clears
  `inkwell_session`.

## Graduation of `INKWELL_BROWSER_LOGIN`

Before defaulting `INKWELL_BROWSER_LOGIN` to `true`, all of these must be true:

- Login and logout pages are shipped behind the flag.
- `tests/browser_login.rs` covers `GET /auth/login` flag-off 404, flag-on HTML
  200, JSON login compatibility, and logout clearing the cookie.
- At least one browser-only author workflow consumes the session cookie, such as
  the media upload UI from `plans/027-media-upload-ui.md` or a non-admin author
  dashboard. The flag should unlock a real user-facing workflow, not only auth
  infrastructure.
- A focused security review confirms the cookie attributes, JSON login POST,
  CSP nonce usage, logout behavior, and CSRF posture remain valid with the flag
  on.
- Operator docs in `docs/DEPLOYMENT.md` explain `INKWELL_BROWSER_LOGIN`, the
  HTTPS requirement implied by the `Secure` cookie, and how authors obtain a
  scoped token for browser login.
- ADR 0010 is amended or superseded with the graduation decision and any
  security changes made during the UI build.

## Recommendation

Defer the production PoC until the media upload UI or another browser authoring
surface is ready to consume it.

The backend session layer is already built and tested, but a standalone login
page has limited value until there is a browser workflow behind it. Deferring
keeps this PR conflict-free and lets the login page be tested against the first
real consumer, especially the `next` redirect and logout placement.

## Open Questions

- Should the first build include `next=/media/new`, or should it redirect to `/`
  until a media UI route exists?
- Should a no-JavaScript fallback be supported by adding form-encoded login
  parsing, and if so should that also add Origin/Referer validation?
- Should logout redirect for HTML callers while preserving `200 OK` for API
  callers?
- Where should browser authoring navigation live once login, media upload, and
  future author tools coexist?
