# Plan 001: Add the Rust Postgres index for status-ordered document lists

Executor instructions: Follow the steps in order. Run every verification command. If a STOP condition occurs, stop and report. When done, update this plan's row in plans/README.md.

Drift check: git diff --stat 8bcd1ea..HEAD -- migrations src/db/migrations.rs src/db/documents.rs tests

## Status

- Priority: P2
- Effort: S
- Risk: LOW
- Depends on: none
- Category: perf / migration
- Planned at: commit 8bcd1ea, 2026-06-19

## Why this matters

The hot read path lists published documents ordered by created_at DESC, id DESC. The table currently has the primary-key index, the unique slug index, and a GIN index for tags, but no B-tree index that supports WHERE status = ? ORDER BY created_at DESC, id DESC.

## Current state

- migrations/0001_create_documents.sql creates documents.
- migrations/0002_add_document_status.sql adds status.
- migrations/0003_add_document_tags.sql adds tags and documents_tags_idx.
- src/db/migrations.rs has MIGRATIONS: [MigrationDef; 3]; rollback metadata must be extended.
- src/db/documents.rs lines 85-102 builds the list query and orders by created_at DESC, id DESC.

## Commands

- cargo fmt --check
- cargo clippy --all-targets --all-features -- -D warnings
- cargo test --all
- cargo build --release --bin inkwell

## Scope

In scope: migrations/0004_add_documents_list_index.sql, src/db/migrations.rs, tests if migration coverage exists or is added.
Out of scope: existing migration SQL files, query behavior in src/db/documents.rs, EXPLAIN assertions without a live Postgres harness.

## Steps

1. Create migrations/0004_add_documents_list_index.sql with:
   CREATE INDEX IF NOT EXISTS documents_status_created_at_id_idx
     ON documents (status, created_at DESC, id DESC);

2. Update src/db/migrations.rs:
   - Change MIGRATIONS length from 3 to 4.
   - Append version 4 with description add_documents_list_index.
   - Set down_sql to DROP INDEX IF EXISTS documents_status_created_at_id_idx;.
   - Do not edit versions 1-3.

3. Run the commands above.

## Done criteria

- Migration 0004 exists.
- Rollback metadata for version 4 exists.
- Existing migrations 1-3 are unchanged.
- Verification commands pass.
- plans/README.md marks plan 001 DONE.

## STOP conditions

- A version 4 migration already exists.
- SQLx migration naming differs from existing 000N_description.sql convention.
- Adding this index requires changing list query semantics.

