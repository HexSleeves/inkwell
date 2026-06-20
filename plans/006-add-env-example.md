# Plan 006: Add a committed .env.example

Executor instructions: Follow the steps in order. Run every verification command. Do not include real secrets. If a STOP condition occurs, stop and report. When done, update this plan's row in plans/README.md.

Drift check: git diff --stat 8bcd1ea..HEAD -- .gitignore README.md docker-compose.yml src/config.rs

## Status

- Priority: P2
- Effort: S
- Risk: LOW
- Depends on: none
- Category: DX
- Planned at: commit 8bcd1ea, 2026-06-19

## Why this matters

.gitignore already allows .env.example, but the file does not exist. Contributors must reconstruct required env vars from src/config.rs, README, and Compose.

## Current state

- .gitignore lines 9-12 ignores .env and .env.* but allows !.env.example.
- src/config.rs lines 17-33 reads DATABASE_URL, HOST, PORT, INKWELL_API_KEY, and INKWELL_SITE_URL.
- README lists the same variables in brief form.

## Commands

- rg -n "changeme-local-dev-key|sk_|AKIA|BEGIN PRIVATE|password=" .env.example README.md docker-compose.yml
- cargo fmt --check
- cargo clippy --all-targets --all-features -- -D warnings
- cargo test --all

## Scope

In scope: .env.example, README.md, docker-compose.yml only if Plan 004 has not already updated docs.
Out of scope: new runtime env vars, checking in .env, real credentials.

## Steps

1. Create .env.example with placeholders for:
   - DATABASE_URL
   - PORT
   - HOST
   - INKWELL_API_KEY
   - INKWELL_SITE_URL
   - POSTGRES_USER
   - POSTGRES_PASSWORD
   - POSTGRES_DB
   - POSTGRES_PORT

2. Use comments to explain that INKWELL_API_KEY must be replaced before exposed deployments.

3. Add README guidance:
   cp .env.example .env
   Mention that .env is gitignored and full integration tests require DATABASE_URL.

4. Run verification.

## Done criteria

- .env.example exists and is tracked.
- .env.example contains only placeholders.
- README points users to copy .env.example.
- Verification commands pass.
- plans/README.md marks plan 006 DONE.

## STOP conditions

- A real secret is discovered in existing docs or examples; report location/type only and do not copy the value.
- Plan 004 changes Compose requirements in a conflicting way; reconcile wording with live Compose.

