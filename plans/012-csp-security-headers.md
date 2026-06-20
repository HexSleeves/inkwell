# Plan 012: Add CSP and hardening headers to HTML pages

Executor instructions: Run this after Plan 017. Follow the steps in order. Run every verification command. If a STOP condition occurs, stop and report. When done, update this plan's row in plans/README.md.

Drift check: git diff --stat 8bcd1ea..HEAD -- src/http src/views tests

## Status

- Priority: P3
- Effort: S-M
- Risk: MED
- Depends on: 017
- Category: security
- Planned at: commit 8bcd1ea, 2026-06-19

## Why this matters

Markdown is rendered with Comrak and sanitized with Ammonia before persistence. A Content-Security-Policy is still useful defense-in-depth for future sanitizer or template regressions. The policy should be designed after Plan 017 removes the Tailwind runtime.

## Current state

- src/views/layout.rs emits inline CSS.
- src/views/layout.rs emits JSON-LD script when metadata is present.
- src/rendering/sanitize.rs allows normal Markdown HTML plus safe links/images.
- src/http/router.rs has CompressionLayer and TraceLayer, no security header layer.

## Commands

- rg -n "tailwindcss|tailwind\.config|TAILWIND_CONFIG" src tests
- cargo fmt --check
- cargo clippy --all-targets --all-features -- -D warnings
- cargo test --all

## Scope

In scope: src/http/router.rs or small src/http/security_headers.rs, tests, src/views/layout.rs only if JSON-LD/CSP interaction needs a nonce/hash decision.
Out of scope: asset pipeline, sanitizer allowlist changes, broad JSON API behavior changes.

## Steps

1. Confirm Plan 017 landed:
   rg -n "tailwindcss|tailwind\.config|TAILWIND_CONFIG" src tests
   Expected: no matches.

2. Add HTML security headers:
   - Content-Security-Policy with default-src self, object-src none, base-uri self, frame-ancestors none, img-src self http https, style-src self unsafe-inline.
   - Decide explicitly how JSON-LD scripts are allowed. Prefer nonce/hash or documented narrow allowance. Do not add broad unsafe-inline script policy without a comment and test.
   - X-Content-Type-Options: nosniff
   - Referrer-Policy: strict-origin-when-cross-origin
   - Permissions-Policy disabling unused powerful features.

3. Add tests for an HTML route proving headers exist and do not allow the old Tailwind CDN.

4. Run verification.

## Done criteria

- HTML responses include CSP and hardening headers.
- CSP does not allow Tailwind CDN.
- JSON/API behavior remains unchanged except safe global non-CSP headers if deliberately applied.
- Verification commands pass.
- plans/README.md marks plan 012 DONE.

## STOP conditions

- Plan 017 has not removed scripts.
- JSON-LD support requires a product/security decision about inline scripts.
- The chosen header implementation affects content negotiation or route matching.

