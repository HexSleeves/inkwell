# Plan 014: Design spike for a first-class authoring CLI

Executor instructions: This is a documentation/design spike. Do not write CLI code. Create one proposed ADR and update plans/README.md. Run every verification command. If a STOP condition occurs, stop and report.

Drift check: git diff --stat 8bcd1ea..HEAD -- README.md src/main.rs src/http/api.rs docs/adr Cargo.toml

## Status

- Priority: P2
- Effort: M
- Risk: LOW
- Depends on: none
- Category: direction
- Planned at: commit 8bcd1ea, 2026-06-19

## Why this matters

Inkwell is API-first, but authoring currently means hand-writing JSON or using raw HTTP. A publishing tool needs a human authoring path. The Rust binary already has serve and db subcommands, so the next decision is whether to extend that binary with authoring commands or provide a separate client CLI.

## Current state

- README lists HTTP routes but no authoring CLI.
- src/main.rs lines 17-45 supports only serve and db subcommands.
- src/http/api.rs supports create/update/delete/publish/unpublish.
- src/config.rs reads INKWELL_API_KEY, which a CLI would need for writes.
- Existing ADRs live under docs/adr; highest current file is 0007-rust-migration.md.

## Commands

- rg -n "Status: proposed|authoring CLI|front matter" docs/adr
- cargo fmt --check
- cargo clippy --all-targets --all-features -- -D warnings
- cargo test --all

## Scope

In scope: new docs/adr/0008-authoring-cli.md unless that number already exists, plans/README.md.
Out of scope: src, Cargo.toml, dependencies, command implementation.

## Steps

1. Create docs/adr/0008-authoring-cli.md with Status: proposed.

2. Cover:
   - current raw HTTP authoring,
   - existing Rust CLI subcommands,
   - option A: extend inkwell binary,
   - option B: separate client CLI,
   - option C: defer to web admin UI,
   - recommended first slice,
   - auth handling without putting secrets in shell history,
   - Markdown plus front matter file format,
   - follow-up implementation plans.

3. Update plans/README.md and run verification.

## Done criteria

- Proposed ADR exists and is self-contained.
- No source code changed.
- ADR names a recommended first implementation slice and open questions.
- Verification commands pass.
- plans/README.md marks plan 014 DONE.

## STOP conditions

- docs/adr/0008-*.md already exists; choose next number only after checking ADR sequence.
- The maintainer wants admin UI instead of CLI as the next authoring surface.
- Drafting requires product choices not inferable from current docs.

