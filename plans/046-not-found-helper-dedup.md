# Plan 046: Extract the repeated "No document with slug" error to a helper

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `plans/README.md`.
>
> **Drift check (run first)**: `git diff --stat 0819727..HEAD -- src/http/api.rs src/error.rs`
> If either file changed, compare the "Current state" excerpts before proceeding.
> NOTE: if plan 039 (api.rs split) has landed, the `NotFound` sites may now live
> in `src/http/documents.rs` — adjust the file target accordingly and proceed.

## Status

- **Priority**: P3
- **Effort**: S
- **Risk**: LOW
- **Depends on**: none (but coordinate with 039 — see Drift check)
- **Category**: tech-debt
- **Planned at**: commit `0819727`, 2026-06-26

## Why this matters

The exact error construction `AppError::NotFound(format!("No document with slug \"{slug}\"."))` is repeated at 5 sites in `src/http/api.rs` (lines 257, 311, 338, 378, 519). If the message wording or error shape ever changes, all five must change in lockstep; missing one yields inconsistent client-facing errors. A single helper makes the message canonical.

## Current state

**`src/http/api.rs`** — the repeated pattern (5 occurrences, lines 257-258, 311-312, 338-339, 378-379, 519-520):
```rust
return Err(AppError::NotFound(format!(
    "No document with slug \"{slug}\"."
)));
```

**`src/error.rs:18-19`** — the variant:
```rust
#[error("{0}")]
NotFound(String),
```

There is no existing constructor helper on `AppError`. The cleanest home is a small free function in `src/http/api.rs` (or an associated function on `AppError` in `src/error.rs`). Given the message is document-specific and api.rs is the only caller, a private helper in api.rs is the lower-risk choice.

## Commands you will need

| Purpose   | Command                                                       | Expected on success |
|-----------|--------------------------------------------------------------|---------------------|
| Count before | `grep -c "No document with slug" src/http/api.rs`         | 5                   |
| Typecheck | `cargo check --all-targets`                                  | exit 0              |
| Tests     | `cargo nextest run`                                          | all pass            |
| Lint      | `cargo clippy --all-targets -- -D warnings`                 | exit 0              |

## Scope

**In scope**:
- `src/http/api.rs` (or `src/http/documents.rs` if 039 landed) — add helper, replace 5 call sites

**Out of scope**:
- `src/error.rs` — do not change the `AppError` enum (unless you choose the associated-function approach; if so, keep it minimal and additive)
- Other `NotFound` messages elsewhere (e.g. preview, media) — those have different wording and are out of scope

## Git workflow

- Branch: `advisor/046-not-found-helper`
- Commit: `refactor(http): extract document-not-found error to a helper`

## Steps

### Step 1: Add the helper

In `src/http/api.rs`, add a private function near the top (after imports, before the handlers):
```rust
/// The canonical 404 for a document addressed by slug. Centralizes the message
/// so every handler returns an identical not-found error.
fn document_not_found(slug: &str) -> AppError {
    AppError::NotFound(format!("No document with slug \"{slug}\"."))
}
```

**Verify**: `cargo check --all-targets` → exit 0

### Step 2: Replace the 5 call sites

Replace each occurrence of:
```rust
return Err(AppError::NotFound(format!(
    "No document with slug \"{slug}\"."
)));
```
with:
```rust
return Err(document_not_found(&slug));
```

Note: confirm the local variable is named `slug` at each site (it is, per the excerpts). If a site has the slug under a different binding, pass that binding.

**Verify**: `grep -c "No document with slug" src/http/api.rs` → 1 (only the helper's `format!` remains)

### Step 3: Run tests

**Verify**: `cargo nextest run` → all pass (the error message is byte-for-byte identical, so contract tests asserting on the 404 body stay green)

## Test plan

No new tests — the message is unchanged, so existing tests that assert on the not-found response continue to pass. If `cargo check` and the existing suite pass, the refactor is correct.

## Done criteria

- [ ] `grep -c "No document with slug" src/http/api.rs` → 1
- [ ] `document_not_found` helper exists and is used at all 5 former sites
- [ ] `cargo check --all-targets` exits 0
- [ ] `cargo clippy --all-targets -- -D warnings` exits 0
- [ ] `cargo fmt --check` exits 0
- [ ] `cargo nextest run` exits 0
- [ ] `plans/README.md` status row updated

## STOP conditions

- The number of `NotFound(format!("No document with slug ...` sites is not 5 (drift, or 039 moved them). Find them all (`grep -rn "No document with slug" src/`), and replace every one; report the actual count.
- A call site is in a different module after the 039 split — apply the helper in whichever module now owns the document handlers.

## Maintenance notes

- If the not-found wording needs to change (e.g. to omit the slug for privacy), change `document_not_found` once.
- This is intentionally scoped to the document-by-slug 404 only; other resources keep their own messages.
