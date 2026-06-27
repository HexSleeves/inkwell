# Plan 039: Split src/http/api.rs god module into focused sub-modules

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `plans/README.md`.
>
> **Drift check (run first)**: `git diff --stat 0819727..HEAD -- src/http/api.rs src/http/mod.rs src/http/router.rs`
> If any in-scope file changed, compare before proceeding.

## Status

- **Priority**: P3
- **Effort**: L
- **Risk**: MED
- **Depends on**: plans/036-document-columns-const.md, plans/037-garden-extract-db-queries.md (recommended to land first to reduce conflicts)
- **Category**: tech-debt
- **Planned at**: commit `0819727`, 2026-06-26

## Why this matters

`src/http/api.rs` at ~983 lines handles document CRUD, publish/unpublish, backlinks, graph, health check, and the shared `resolve_visibility` helper used by other modules. Any new document feature requires navigating and touching one large file. PR diffs that touch it are hard to review. Adding new patterns (e.g., bulk operations, scoped search) without clear module boundaries leads to more accumulation.

Splitting into focused sub-modules gives each concern its own file, makes ownership clear, and reduces merge conflicts. `resolve_visibility` is already imported from `api.rs` by `ai.rs` and `search.rs` — moving it to `auth.rs` fixes a layering oddity.

## Current state

**`src/http/api.rs`** — mixed concerns (confirmed by reading):
- Health: `api::health` handler
- Document CRUD: `api::documents`, `api::document` (GET/POST/PATCH/PUT/DELETE)
- Publishing: `api::publish_document`, `api::unpublish_document`
- Graph/links: `api::graph`, `api::document_backlinks`, `api::document_graph`
- Shared helper: `pub(crate) async fn resolve_visibility` (imported by `ai.rs`, `search.rs`)

**`src/http/mod.rs`** — declares `pub(crate) mod api;` and other modules. After split, it needs to declare sub-modules and re-export the public handler names used in `router.rs`.

**`src/http/router.rs`** — imports like `use super::{ ..., api, ... }` and routes like `any(api::documents)`. These will need updating to match the new module structure.

**Convention**: existing modules in `src/http/` use snake_case filenames (`admin.rs`, `auth.rs`, `media.rs`). New modules should follow the same pattern.

## Commands you will need

| Purpose   | Command                                        | Expected on success |
|-----------|------------------------------------------------|---------------------|
| Typecheck | `cargo check --all-targets`                    | exit 0              |
| Tests     | `cargo nextest run`                            | all pass            |
| Lint      | `cargo clippy --all-targets -- -D warnings`    | exit 0              |
| Fmt       | `cargo fmt --check`                            | exit 0              |

## Scope

**In scope**:
- `src/http/api.rs` — to be split
- `src/http/documents.rs` (NEW) — document CRUD handlers + types
- `src/http/graph.rs` (NEW) — graph and backlinks handlers
- `src/http/publish.rs` (NEW) — publish/unpublish handlers
- `src/http/mod.rs` — declare new modules
- `src/http/router.rs` — update import paths
- `src/http/auth.rs` — move `resolve_visibility` here
- `src/http/ai.rs`, `src/http/search.rs` — update import of `resolve_visibility`

**Out of scope**:
- `src/http/preview.rs` — already a separate module; no change
- `src/http/admin.rs` — already a separate module; no change
- Any change to handler logic — this is a pure restructuring, no behaviour changes

## Git workflow

- Branch: `advisor/039-api-module-split`
- Commit per step: use multiple commits — one per sub-module extracted
- Message style: `refactor(http): extract <concern> from api.rs into <module>.rs`

## Steps

### Step 1: Move resolve_visibility to auth.rs

`resolve_visibility` is already used by `ai.rs` and `search.rs` via `use crate::http::api::resolve_visibility`. Moving it to `auth.rs` is the right home (it is an auth concern).

1. Cut `resolve_visibility` from `src/http/api.rs`
2. Paste into `src/http/auth.rs` with `pub(crate)` visibility
3. Update `src/http/ai.rs` and `src/http/search.rs` imports: change `use crate::http::api::resolve_visibility` to `use crate::http::auth::resolve_visibility`
4. Update any remaining references in `api.rs` itself

**Verify**: `cargo check --all-targets` → exit 0

### Step 2: Extract graph/backlinks to graph.rs

Create `src/http/graph.rs`. Move from `api.rs`:
- `graph` handler (`GET /graph`)
- `document_backlinks` handler
- `document_graph` handler
- Any types used only by these handlers

Declare `pub(crate) mod graph;` in `src/http/mod.rs`.
Update `src/http/router.rs` imports: `use super::graph` and change `any(api::graph)` to `any(graph::graph)` etc.

**Verify**: `cargo check --all-targets` → exit 0

### Step 3: Extract publish/unpublish to publish.rs

Create `src/http/publish.rs`. Move from `api.rs`:
- `publish_document` handler
- `unpublish_document` handler
- Related types/enums

Update `mod.rs` and `router.rs` accordingly.

**Verify**: `cargo check --all-targets` → exit 0

### Step 4: Rename api.rs to documents.rs

What remains in `api.rs` after steps 1–3 should be document CRUD + health + shared utility types. Rename:
- `src/http/api.rs` → `src/http/documents.rs`
- In `src/http/mod.rs`: change `pub(crate) mod api;` → `pub(crate) mod documents;`; optionally keep `pub(crate) use documents as api;` re-export for a one-step migration
- In `router.rs` and any other importers: update `api::documents` → `documents::documents`, etc.

`health` handler can stay in `documents.rs` or move to a tiny `health.rs` — your call; keep it in `documents.rs` if the change is trivial, move if it's already isolated.

**Verify**: `cargo check --all-targets` → exit 0; `cargo nextest run` → all pass

### Step 5: Final cleanup

- Remove any `pub use` re-exports added for transitional compatibility
- Confirm `src/http/api.rs` no longer exists (or is a thin re-export module only)

**Verify**: `cargo clippy --all-targets -- -D warnings` → exit 0; `cargo fmt --check` → exit 0

## Test plan

No new tests — pure restructuring. All existing tests exercise the HTTP endpoints and will catch any import error or routing change.

## Done criteria

- [ ] `src/http/api.rs` either does not exist or is ≤ 50 lines (only re-exports)
- [ ] New files: `src/http/documents.rs`, `src/http/graph.rs`, `src/http/publish.rs`
- [ ] `resolve_visibility` lives in `src/http/auth.rs`
- [ ] `cargo check --all-targets` exits 0
- [ ] `cargo clippy --all-targets -- -D warnings` exits 0
- [ ] `cargo fmt --check` exits 0
- [ ] `cargo nextest run` exits 0 (all routes still work)
- [ ] `plans/README.md` status row updated

## STOP conditions

- A split requires moving a type that is shared between more than 2 new modules, requiring a new `src/http/types.rs` shared module. Report and stop — the plan needs to be extended.
- `cargo check` fails with circular import errors. Report the cycle and stop.
- The split would require touching more than 10 files total. Report scope and stop.

## Maintenance notes

- After this split, adding a new document feature means creating or editing `src/http/documents.rs` only — no more navigating 983 lines.
- `resolve_visibility` in `auth.rs` is the single source of truth for visibility derivation from HTTP headers. Any changes to auth semantics (new scopes, new token types) update it once.
