# Plan 045: Rate-limit the unauthenticated preview-read endpoint

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `plans/README.md`.
>
> **Drift check (run first)**: `git diff --stat 0819727..HEAD -- src/http/rate_limit.rs tests/rate_limit_contract.rs`
> If either file changed, compare the "Current state" excerpt before proceeding.

## Status

- **Priority**: P3
- **Effort**: S
- **Risk**: LOW
- **Depends on**: none
- **Category**: security (defense-in-depth)
- **Planned at**: commit `0819727`, 2026-06-26

## Why this matters

`GET /documents/{slug}/preview?token=<pvw_...>` is the only unauthenticated endpoint that performs a secret comparison, and it is a `GET`, so the rate limiter excludes it (`should_limit` only governs writes + `/ask`). The preview secret is 256-bit and compared in constant time, so brute force is already infeasible — this is **defense-in-depth, not a critical fix**. Adding the preview-read path to the rate limiter means a leaked/guessed prefix cannot be probed at unlimited network speed, and abusive traffic to a known shareable link is bounded. Honest assessment: low leverage; included for completeness. If the maintainer prefers to skip it, mark REJECTED with that rationale.

## Current state

**`src/http/rate_limit.rs:155-163`** — the gate:
```rust
/// Which requests the limiter governs: any non-idempotent mutation, plus `/ask`
/// (which is `GET|POST` but drives two AI providers). Reads (`GET`/`HEAD`) and
/// the public HTML site are deliberately excluded.
fn should_limit(method: &Method, path: &str) -> bool {
    matches!(
        *method,
        Method::POST | Method::PUT | Method::PATCH | Method::DELETE
    ) || path == "/ask"
}
```

**`src/http/rate_limit.rs:201-214`** — existing unit test for `should_limit`:
```rust
#[test]
fn should_limit_targets_writes_and_ask_only() {
    assert!(should_limit(&Method::POST, "/documents"));
    // ...
    assert!(should_limit(&Method::GET, "/ask"));
    assert!(!should_limit(&Method::GET, "/documents"));
    // ...
}
```

The preview path is `/documents/{slug}/preview` — at the middleware layer the concrete path is e.g. `/documents/my-note/preview`. Matching requires a suffix check (`path.ends_with("/preview")`), since `{slug}` is dynamic.

## Commands you will need

| Purpose   | Command                                                  | Expected on success |
|-----------|--------------------------------------------------------|---------------------|
| Typecheck | `cargo check --all-targets`                             | exit 0              |
| Unit test | `cargo nextest run --lib http::rate_limit`             | all pass            |
| All tests | `cargo nextest run`                                    | all pass            |
| Lint      | `cargo clippy --all-targets -- -D warnings`            | exit 0              |

## Scope

**In scope**:
- `src/http/rate_limit.rs` — extend `should_limit` and its unit test

**Out of scope**:
- The limiter's keying logic (`resolve_key`, `client_ip`) — no change; the preview path has no principal so it keys by IP, which is correct
- Other GET endpoints — only `/preview` is added; do NOT add general read throttling

## Git workflow

- Branch: `advisor/045-rate-limit-preview`
- Commit: `feat(http): rate-limit the unauthenticated preview-read endpoint`

## Steps

### Step 1: Extend should_limit to cover the preview-read path

Change `should_limit` so it also governs `GET /documents/{slug}/preview`:
```rust
fn should_limit(method: &Method, path: &str) -> bool {
    matches!(
        *method,
        Method::POST | Method::PUT | Method::PATCH | Method::DELETE
    ) || path == "/ask"
        || (*method == Method::GET && path.ends_with("/preview"))
}
```

Note: use `ends_with("/preview")` — `{slug}` is dynamic so an exact match is impossible. The preview-tokens management routes (`/preview-tokens`, `/preview-tokens/{prefix}`) are POST/GET/DELETE; the POST/DELETE ones are already covered by the mutation arm, and listing (GET `/preview-tokens`) requires auth so it is not a brute-force surface — `ends_with("/preview")` does NOT match `/preview-tokens` (different suffix), which is correct.

**Verify**: `cargo check --all-targets` → exit 0

### Step 2: Extend the unit test

In the `#[cfg(test)] mod tests` block, add assertions to `should_limit_targets_writes_and_ask_only` (or a new test):
```rust
// Preview-read is rate-limited (unauthenticated secret comparison).
assert!(should_limit(&Method::GET, "/documents/my-note/preview"));
// But the authenticated preview-token management list is not caught by the suffix.
assert!(!should_limit(&Method::GET, "/documents/my-note/preview-tokens"));
```

**Verify**: `cargo nextest run --lib http::rate_limit` → all pass

### Step 3: Run all tests

**Verify**: `cargo nextest run` → all pass

## Test plan

- Unit: `should_limit(GET, ".../preview")` → true; `should_limit(GET, ".../preview-tokens")` → false (the existing unit-test harness in `rate_limit.rs` covers this with no DB needed).
- The existing `tests/rate_limit_contract.rs` integration behaviour is unchanged (writes still limited); no new integration test required, but if you add one, model it on the existing burst-to-429 test and drive `GET .../preview` requests.

## Done criteria

- [ ] `should_limit` returns true for `GET .../preview`, false for `GET .../preview-tokens`
- [ ] `cargo check --all-targets` exits 0
- [ ] `cargo clippy --all-targets -- -D warnings` exits 0
- [ ] `cargo nextest run` exits 0; updated unit test passes
- [ ] `plans/README.md` status row updated

## STOP conditions

- `should_limit` has been refactored to take a typed route enum instead of a string path (drift). Adapt to the new signature; report.
- Adding `ends_with("/preview")` accidentally catches another route ending in `/preview`. Grep `src/http/router.rs` for `/preview` to confirm only the preview-read route ends that way (it does at `0819727`).

## Maintenance notes

- This couples a path-suffix string to a route shape. If the preview route is ever renamed, update `should_limit` and its test together.
- The preview endpoint keys by IP (no principal). Behind a proxy, IP keying depends on `INKWELL_TRUST_FORWARDED_HEADERS` — document that operators wanting accurate per-client preview limits must configure forwarded-header trust.
