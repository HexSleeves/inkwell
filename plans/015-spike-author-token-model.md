# Plan 015: Design scoped author tokens and write audit

Executor instructions: This is a documentation/design spike. Do not write auth code or migrations. Create one proposed ADR and update plans/README.md. Run every verification command. If a STOP condition occurs, stop and report.

Drift check: git diff --stat 8bcd1ea..HEAD -- src/http/auth.rs src/http/api.rs src/config.rs migrations src/db/migrations.rs docs/adr docs/audit-v0.1.md

## Status

- Priority: P3
- Effort: L
- Risk: LOW
- Depends on: 004
- Category: direction / security
- Planned at: commit 8bcd1ea, 2026-06-19

## Why this matters

The current Rust service gates every write with one shared INKWELL_API_KEY. That is simple and fails closed, but it has no owner identity, scoping, revocation, or write audit trail. Multi-author publishing and safe key rotation need a deliberate token model and schema before implementation.

## Current state

- src/config.rs lines 26-29 reads a single optional INKWELL_API_KEY.
- src/http/auth.rs lines 5-26 compares exactly one x-api-key header to that configured key.
- src/http/api.rs uses require_api_key on create/update/delete/publish/unpublish.
- migrations/0001_create_documents.sql has no author/owner field.
- src/http/feed.rs has no Atom author metadata because the data model has none.
- docs/audit-v0.1.md records the single shared key and missing audit log as known gaps.

## Commands

- rg -n "Status: proposed|scoped token|write audit|revocation" docs/adr
- cargo fmt --check
- cargo clippy --all-targets --all-features -- -D warnings
- cargo test --all

## Scope

In scope: new docs/adr/0009-scoped-author-tokens.md unless that number already exists, plans/README.md.
Out of scope: src, migrations, token generation code, current auth behavior changes.

## Steps

1. Create docs/adr/0009-scoped-author-tokens.md with Status: proposed.

2. Cover:
   - current shared-key behavior and bootstrap/admin fallback,
   - proposed authors table,
   - proposed API tokens table with hashed token secret, scopes, revoked timestamp, last-used metadata,
   - write audit events,
   - scopes such as documents:write, documents:publish, admin,
   - draft read visibility and ownership choices,
   - migration/backfill path for existing documents,
   - rotation and revocation requirements,
   - no plaintext token storage,
   - follow-up implementation slices.

3. Update plans/README.md and run verification.

## Done criteria

- Proposed ADR exists and is self-contained.
- No source code or migrations changed.
- ADR defines follow-up implementation slices and product questions.
- Verification commands pass.
- plans/README.md marks plan 015 DONE.

## STOP conditions

- docs/adr/0009-*.md already exists; choose next number only after checking ADR sequence.
- The maintainer does not want multi-author/account semantics.
- The design would require storing plaintext API tokens.

