---
name: database-migration
description: Add a new database migration or extend the document schema. Covers both schema changes and the code updates they require.
triggers:
  - "migration"
  - "new column"
  - "new table"
  - "schema change"
  - "add field"
  - "alter table"
edges:
  - target: context/architecture.md
    condition: when the schema change affects the link graph, embeddings, or garden flow
  - target: context/conventions.md
    condition: when updating domain types and DB queries to match the new schema
  - target: context/stack.md
    condition: when checking SQLx migration mechanics
last_updated: 2026-06-23
---

# Database Migration

## Context

Migrations live in `migrations/`, named `NNNN_<description>.sql` (zero-padded 4-digit sequence). SQLx runs them via `cargo run --bin inkwell -- db migrate` (`src/cli/migrate.rs`). All queries in `src/db/` enumerate columns explicitly — adding a column to `documents` requires updating every `SELECT` query.

## Task: Add a New Column to `documents`

### Steps

1. **Create migration file** `migrations/NNNN_add_<column>_to_documents.sql`:
   ```sql
   ALTER TABLE documents ADD COLUMN my_field text NOT NULL DEFAULT 'value';
   ```
   Use `DEFAULT` so the migration doesn't lock the table on large datasets.

2. **Update `Document` struct** in `src/domain/document.rs`:
   ```rust
   pub struct Document {
       // ... existing fields ...
       pub my_field: String,
   }
   ```

3. **Update every `SELECT` query** in `src/db/documents.rs` — all `SELECT id, slug, title, body_markdown, rendered_html, status, growth, tags, version, created_at, updated_at FROM documents` queries must include the new column. There are ~8 such queries; grep for `SELECT id, slug` to find them all.

4. **Update `NewDocument`** if the field is set on create:
   ```rust
   pub struct NewDocument {
       // ...
       pub my_field: Option<String>,
   }
   ```

5. **Update `DocumentPatch`** if the field is patchable:
   ```rust
   pub struct DocumentPatch {
       // ...
       pub my_field: Option<String>,
   }
   ```

6. **Update INSERT in `create_document`** (`src/db/documents.rs`) to bind the new field.

7. **Update UPDATE in `update_document_by_slug` and `update_document_by_slug_if_version`** to include the field with `COALESCE`.

8. **Update `DocumentEnvelope`** in `src/http/api.rs` and `From<Document>` impl.

9. **Update MCP tool argument types** in `src/mcp/mod.rs` if agents should be able to set/update the field.

### Gotchas

- Every `SELECT` that returns a `Document` via `sqlx::query_as::<Postgres, Document>` must include ALL fields — SQLx will fail at runtime with a column mismatch if any are missing
- New enum columns stored as `text` need `#[sqlx(type_name = "text", rename_all = "lowercase")]` on the Rust enum and a Postgres `CHECK` constraint in the migration (see `GrowthStage` / migration 0007 as the template)
- `version` and `updated_at` are bumped automatically by `update_document_by_slug` — never manually update them in a patch
- `set_document_status` does NOT return all columns in its current form — if you add a column, update its `RETURNING` list too
- pgvector extension must be installed before migration 0009; any migration that creates a `vector(N)` column has the same prereq

### Verify

- [ ] Migration file named `NNNN_description.sql` with next sequence number
- [ ] `Document` struct updated
- [ ] ALL SELECT queries in `src/db/documents.rs` updated (grep `SELECT id, slug`)
- [ ] `NewDocument` / `DocumentPatch` updated if applicable
- [ ] `DocumentEnvelope` and `From<Document>` updated
- [ ] `cargo run --bin inkwell -- db migrate` succeeds on a local DB
- [ ] `cargo test --all` passes

## Task: Add a New Table

### Steps

1. **Create migration** `migrations/NNNN_create_<table>.sql` with `CREATE TABLE`, indexes, and foreign keys.

2. **Create `src/db/<table>.rs`** with typed query functions following the pattern in `src/db/documents.rs`.

3. **Register in `src/db/mod.rs`**:
   ```rust
   pub mod my_table;
   ```

4. **Use from handlers** — only call from `src/http/` or `src/garden.rs`; never from `src/domain/`.

### Gotchas

- If the table has a `note_id` foreign key to `documents`, use `ON DELETE CASCADE` (see `links`, `note_chunks`) so note deletes don't leave orphan rows.
- Indexes on foreign keys are not created automatically in Postgres — add them explicitly.

### Verify

- [ ] Foreign key constraints and ON DELETE behavior correct
- [ ] Indexes on FK columns added
- [ ] `cargo run --bin inkwell -- db migrate` succeeds
- [ ] Module declared in `src/db/mod.rs`

## Update Scaffold
- [ ] Update `.mex/ROUTER.md` "Current Project State" if the schema change completes a feature
- [ ] Update `context/architecture.md` if the new table changes the system's data model
