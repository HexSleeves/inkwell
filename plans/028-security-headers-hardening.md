# Plan 028: Harden HTTP security headers (TraceLayer URI, HSTS, img-src colons)

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `plans/README.md`.
>
> **Drift check (run first)**: `git diff --stat 0819727..HEAD -- src/http/router.rs src/http/security_headers.rs`
> If either file changed since this plan was written, compare the "Current
> state" excerpts against the live code before proceeding; on a mismatch,
> treat it as a STOP condition.

## Status

- **Priority**: P1
- **Effort**: S
- **Risk**: LOW
- **Depends on**: none
- **Category**: security
- **Planned at**: commit `0819727`, 2026-06-26

## Why this matters

Three distinct security issues can be fixed in two files:

1. **TraceLayer logs the full request URI**, including query-string tokens like `?token=pvw_…`. Anyone with Railway log access can harvest live preview tokens and read private drafts.
2. **No `Strict-Transport-Security` header**: session cookies and API keys are vulnerable if a connection is ever made over plain HTTP (Railway redirect window, misconfigured proxy).
3. **CSP `img-src` directive has bare `http` and `https` without colons** (`img-src 'self' http https`). Per the W3C CSP Level 2 grammar, scheme-sources require a trailing colon (`http:`, `https:`). Bare tokens are invalid scheme-sources and browsers ignore them — so any external image in user Markdown is silently blocked.

All three fixes are additive-only (no existing behaviour is removed).

## Current state

**`src/http/router.rs:126–134`** — TraceLayer make_span_with:
```rust
TraceLayer::new_for_http().make_span_with(|request: &axum::extract::Request| {
    let request_id = request_id::current().unwrap_or_default();
    tracing::info_span!(
        "http_request",
        method = %request.method(),
        uri = %request.uri(),   // ← logs full URI including ?token=pvw_...
        %request_id,
    )
}),
```

**`src/http/security_headers.rs:52–59`** — CSP policy (HTML responses only):
```rust
if is_html {
    let policy = format!(
        "default-src 'self'; object-src 'none'; base-uri 'self'; frame-ancestors 'none'; \
         img-src 'self' http https; style-src 'self' 'unsafe-inline'; \
         script-src 'self' 'nonce-{}'",
        nonce.as_str()
    );
    // ...
}
```
Issue 1: `http https` → should be `http: https:` (missing colons).
Issue 2: No `Strict-Transport-Security` header anywhere in this file.

**Conventions**: security_headers.rs uses `headers.insert(HeaderName, HeaderValue)` for each header. HSTS header name is `"strict-transport-security"` (lowercase). Value: `"max-age=63072000; includeSubDomains"` (2 years, no `preload` until opt-in decision is made).

## Commands you will need

| Purpose    | Command                                              | Expected on success          |
|------------|------------------------------------------------------|------------------------------|
| Typecheck  | `cargo check --all-targets`                          | exit 0, no errors            |
| Tests      | `cargo nextest run --test security_headers_contract` | all pass                     |
| All tests  | `cargo nextest run`                                  | all pass                     |
| Lint       | `cargo clippy --all-targets -- -D warnings`          | exit 0, no warnings          |
| Fmt        | `cargo fmt --check`                                  | exit 0                       |

## Scope

**In scope** (only files you should modify):
- `src/http/router.rs`
- `src/http/security_headers.rs`
- `tests/security_headers_contract.rs` (update/add tests)

**Out of scope** (do NOT touch):
- `src/http/auth_session.rs` — session cookie already has `SameSite=Strict`; leave it
- Any change to CSP `script-src` nonce logic — that is working correctly
- `style-src 'unsafe-inline'` — that is a separate plan (029)

## Git workflow

- Branch: `advisor/028-security-headers`
- Commit style: `fix(http): <description>` (match repo conventional commits)
- Do NOT push or open a PR unless instructed

## Steps

### Step 1: Redact preview token from TraceLayer span

In `src/http/router.rs`, change the `make_span_with` closure so it logs only the URI path, not the full URI with query string:

```rust
// Before:
uri = %request.uri(),

// After:
uri = %request.uri().path(),
```

The full file change (lines 130–132 of the `make_span_with` closure):
```rust
tracing::info_span!(
    "http_request",
    method = %request.method(),
    uri = %request.uri().path(),   // path only — query string may contain tokens
    %request_id,
)
```

**Verify**: `cargo check --all-targets` → exit 0

### Step 2: Fix CSP img-src scheme tokens (add colons)

In `src/http/security_headers.rs`, update the CSP format string at line 54:

```rust
// Before:
"img-src 'self' http https; style-src 'self' 'unsafe-inline'; script-src 'self' 'nonce-{}'",

// After:
"img-src 'self' http: https:; style-src 'self' 'unsafe-inline'; script-src 'self' 'nonce-{}'",
```

Only the `img-src` directive changes. Everything else in the format string stays identical.

**Verify**: `cargo check --all-targets` → exit 0

### Step 3: Add Strict-Transport-Security header

In `src/http/security_headers.rs`, add HSTS to the unconditional headers section (alongside `X_CONTENT_TYPE_OPTIONS`, `REFERRER_POLICY`, and `permissions-policy`). Insert after the `permissions-policy` block, before the `if is_html` block:

```rust
headers.insert(
    HeaderName::from_static("strict-transport-security"),
    HeaderValue::from_static("max-age=63072000; includeSubDomains"),
);
```

**Verify**: `cargo check --all-targets` → exit 0

### Step 4: Update security header contract tests

Open `tests/security_headers_contract.rs` and verify the existing tests still pass. Then add:

1. A test asserting `Strict-Transport-Security: max-age=63072000; includeSubDomains` is present on both HTML and JSON responses.
2. A test asserting `Content-Security-Policy` on an HTML response contains `img-src 'self' http: https:` (with colons).
3. A test asserting the request span URI does NOT contain a query string (this may need to check logs or be a unit test on the span builder — if hard to test via integration, skip and add a comment; do not block the plan on this).

Follow the pattern of existing tests in `tests/security_headers_contract.rs`.

**Verify**: `cargo nextest run --test security_headers_contract` → all pass

## Test plan

- Existing tests in `tests/security_headers_contract.rs` must still pass.
- New test: HSTS header present on `GET /health` (JSON response) → confirms unconditional header.
- New test: HSTS header present on `GET /` (HTML response) → confirms it survives the is_html branch too.
- New test: CSP on HTML response contains `http: https:` (with colons).

## Done criteria

- [ ] `cargo check --all-targets` exits 0
- [ ] `cargo clippy --all-targets -- -D warnings` exits 0
- [ ] `cargo fmt --check` exits 0
- [ ] `cargo nextest run` exits 0; new HSTS + img-src tests pass
- [ ] `src/http/router.rs` TraceLayer span logs `uri.path()` not `uri()`
- [ ] `src/http/security_headers.rs` CSP string contains `http: https:` (with colons)
- [ ] `src/http/security_headers.rs` includes HSTS unconditionally
- [ ] No files outside the in-scope list are modified (`git status`)
- [ ] `plans/README.md` status row updated

## STOP conditions

- Code at the cited locations does not match the excerpts above (drift).
- `cargo check` fails after any step.
- HSTS addition causes existing tests to fail and the cause is not obvious.
- The `security_headers_contract.rs` file does not exist — check `tests/` and adapt.

## Maintenance notes

- The `max-age=63072000` (2 years) should only be raised to `preload` candidate levels after confirming no plain-HTTP entry points exist in production.
- If a separate CDN or proxy terminates TLS before reaching inkwell, HSTS from the app itself is still correct (defense in depth).
- The `uri.path()` change means query parameters (search terms, pagination) are also not logged. If future debugging needs query params, add a separate `query = %request.uri().query().unwrap_or("")` field — but ensure it is scrubbed or redacted before logging.
