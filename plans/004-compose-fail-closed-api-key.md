# Plan 004: Make Docker Compose require an explicit API key

Executor instructions: Follow the steps in order. Run every verification command. If a STOP condition occurs, stop and report. When done, update this plan's row in plans/README.md.

Drift check: git diff --stat 8bcd1ea..HEAD -- docker-compose.yml README.md src/http/auth.rs src/config.rs

## Status

- Priority: P1
- Effort: S
- Risk: LOW
- Depends on: none
- Category: security
- Planned at: commit 8bcd1ea, 2026-06-19

## Why this matters

Rust auth fails closed when INKWELL_API_KEY is unset. docker-compose.yml defeats that property by providing the known default changeme-local-dev-key, so a user who starts Compose without setting a real key gets write access protected by a public placeholder.

## Current state

- docker-compose.yml line 26 sets INKWELL_API_KEY with fallback changeme-local-dev-key.
- src/config.rs lines 26-33 trims and filters env values.
- src/http/auth.rs lines 5-26 returns false for missing or empty configured keys.
- README line 18 says writes fail closed when INKWELL_API_KEY is unset.

## Commands

- env -u INKWELL_API_KEY docker compose config
- INKWELL_API_KEY=dev-secret docker compose config
- cargo fmt --check
- cargo clippy --all-targets --all-features -- -D warnings
- cargo test --all

## Scope

In scope: docker-compose.yml, README.md if Compose docs need a one-line note, .env.example only if Plan 006 already landed.
Out of scope: Rust auth behavior, token scopes, user identity.

## Steps

1. Change docker-compose.yml app env entry to required-variable syntax:
   INKWELL_API_KEY: required with a clear message to set INKWELL_API_KEY before running Compose.

2. Verify Compose fails without the key:
   env -u INKWELL_API_KEY docker compose config
   Expected: nonzero with the required-variable message.

3. Verify Compose works with a key:
   INKWELL_API_KEY=dev-secret docker compose config
   Expected: exit 0.

4. Update README under Docker Compose if needed: set INKWELL_API_KEY in shell or .env first; Compose intentionally refuses to start without it.

5. Run normal verification.

## Done criteria

- docker-compose.yml no longer contains changeme-local-dev-key.
- Compose fails before startup when INKWELL_API_KEY is missing.
- Docs or .env.example explain the required key.
- Verification commands pass.
- plans/README.md marks plan 004 DONE.

## STOP conditions

- The maintainer wants Compose to remain one-command insecure local startup.
- The local Compose version does not support required-variable syntax.
- The fix appears to require changing runtime auth behavior.

