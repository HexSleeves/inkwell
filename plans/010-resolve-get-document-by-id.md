# Plan 010: Resolve the unused get_document_by_id helper

Executor instructions: Follow the steps in order. Run every verification command. If a STOP condition occurs, stop and report. When done, update this plan's row in plans/README.md.

Drift check: git diff --stat 8bcd1ea..HEAD -- src/db/documents.rs src/http tests

## Status

- Priority: P3
- Effort: S
- Risk: LOW
- Depends on: none
- Category: tech debt
- Planned at: commit 8bcd1ea, 2026-06-19

## Why this matters

src/db/documents.rs exports get_document_by_id, but no route or test uses it. Keeping unused exported helpers makes the DAL surface look larger than the product needs.

## Current state

- src/db/documents.rs lines 72-83 defines get_document_by_id.
- src/http/router.rs exposes slug routes only.
- README documents slug routes only.

## Commands

- rg -n "get_document_by_id" src tests
- cargo fmt --check
- cargo clippy --all-targets --all-features -- -D warnings
- cargo test --all

## Scope

In scope: src/db/documents.rs, tests only if keeping the helper with coverage.
Out of scope: public ID routes, slug route behavior, API response shapes.

## Steps

1. Default action: remove get_document_by_id and the now-unused uuid::Uuid import if no current code uses it.

2. If a real current caller exists by execution time, keep the helper and add/adjust tests instead.

3. Run verification.

## Done criteria

- Either the helper is removed, or it has a real caller and tests.
- No unused import remains.
- Verification commands pass.
- plans/README.md marks plan 010 DONE.

## STOP conditions

- The maintainer wants ID-based fetch to become public API.
- Removing the helper breaks a caller not visible in src or tests.

