# Plan 035: Remove unused syntect dependency

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `plans/README.md`.
>
> **Drift check (run first)**: `git diff --stat 0819727..HEAD -- Cargo.toml src/rendering/highlight.rs`
> If either file changed, compare the "Current state" excerpts before proceeding.

## Status

- **Priority**: P3
- **Effort**: S
- **Risk**: LOW
- **Depends on**: none
- **Category**: tech-debt
- **Planned at**: commit `0819727`, 2026-06-26

## Why this matters

`Cargo.toml` declares `syntect = "5"` as a dependency. `src/rendering/highlight.rs` is 3 lines long — it returns the string `"hljs"` (indicating syntax highlighting is client-side via highlight.js, not server-side via syntect). A grep for `syntect` in all of `src/` returns zero results. The crate is compiled into the project but never called, adding compile time and binary size for nothing. Every unused dependency is also a supply-chain surface for no benefit.

## Current state

**`Cargo.toml:26`**:
```toml
syntect = "5"
```

**`src/rendering/highlight.rs`** (entire file):
```rust
pub fn highlight_classes() -> &'static str {
    "hljs"
}
```

No `use syntect` in any file under `src/`.

**Architecture docs** (`context/stack.md`) mention `syntect` as a dependency for syntax highlighting — this is stale documentation and should be updated.

## Commands you will need

| Purpose     | Command                                        | Expected on success |
|-------------|------------------------------------------------|---------------------|
| Verify unused | `grep -rn "syntect" src/`                   | zero matches        |
| Typecheck   | `cargo check --all-targets`                    | exit 0              |
| Tests       | `cargo nextest run`                            | all pass            |
| Lint        | `cargo clippy --all-targets -- -D warnings`    | exit 0              |
| Fmt         | `cargo fmt --check`                            | exit 0              |

## Scope

**In scope**:
- `Cargo.toml` — remove `syntect = "5"` line
- `.mex/context/stack.md` — update the "Key Libraries" section to remove the syntect entry and update the note about highlighting

**Out of scope**:
- `src/rendering/highlight.rs` — leave this file as-is. (Note: `highlight_classes()` is currently only *defined*, not called from anywhere — `grep -rn "highlight_classes" src/` shows the definition only. It is dead too, but removing it is a separate cleanup, NOT part of removing the `syntect` dependency. Do not delete it here; just do not let its presence stop you.)
- `Cargo.lock` — it updates automatically when you run `cargo check`

## Git workflow

- Branch: `advisor/035-remove-syntect`
- Commit: `chore(deps): remove unused syntect dependency`

## Steps

### Step 1: Confirm syntect is truly unused

Run: `grep -rn "syntect" src/`

Expected: zero matches. If any matches appear, **STOP** — the dependency may be in use. Report the matches.

**Verify**: zero matches returned.

### Step 2: Remove from Cargo.toml

Remove the line `syntect = "5"` from `[dependencies]` in `Cargo.toml`.

**Verify**: `cargo check --all-targets` → exit 0

### Step 3: Update stack.md

In `.mex/context/stack.md`, find the `syntect` entry under "Key Libraries" (it says something like "syntax highlighting for code blocks") and update it to reflect the actual approach:

Replace the syntect entry with:
```
- **highlight.js** (client-side) — syntax highlighting via CSS class names emitted by `src/rendering/highlight.rs`; no server-side rendering
```

Also update the `last_updated` date in the file's YAML frontmatter.

**Verify**: Stack.md no longer mentions syntect.

### Step 4: Run all tests

**Verify**: `cargo nextest run` → all pass

## Done criteria

- [ ] `grep -rn "syntect" src/ Cargo.toml` → zero matches
- [ ] `cargo check --all-targets` exits 0
- [ ] `cargo clippy --all-targets -- -D warnings` exits 0
- [ ] `cargo fmt --check` exits 0
- [ ] `cargo nextest run` exits 0
- [ ] `.mex/context/stack.md` no longer mentions syntect
- [ ] `plans/README.md` status row updated

## STOP conditions

- `grep -rn "syntect" src/` returns any matches (dependency is in use).
- `cargo check` fails after removing the line — unlikely but could indicate a transitive dep that was accidentally relying on syntect being resolved.

## Maintenance notes

- Syntax highlighting remains client-side via highlight.js. If server-side highlighting is needed in the future, add the chosen crate explicitly with a justification.
