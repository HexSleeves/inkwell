# Plan 020: `derive_excerpt` truncates on a char boundary instead of panicking on multibyte UTF-8

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `plans/README.md`.
>
> **Drift check (run first)**: `git diff --stat c7b0a46..HEAD -- src/views/layout.rs src/views/index.rs src/views/tags.rs src/views/search.rs src/views/document.rs`
> If `src/views/layout.rs` changed since this plan was written, compare the
> "Current state" excerpt against the live code before proceeding; on a
> mismatch, treat it as a STOP condition.

## Status

- **Priority**: P1
- **Effort**: S
- **Risk**: LOW
- **Depends on**: none
- **Category**: bug
- **Planned at**: commit `c7b0a46`, 2026-06-20

## Why this matters

`derive_excerpt` slices a `&str` by a **byte** offset (`&text[..max_length]`).
Rust's `str` byte-indexing panics if the offset is not on a UTF-8 character
boundary. The input is the document body markdown (author-controlled), and the
derived excerpt is rendered on every public listing surface — the index, every
tag page, search results, and the per-document `<meta name="description">`. A
single published document whose stripped text exceeds 160 bytes and happens to
have a multibyte character (emoji, accented Latin, CJK, etc.) straddling byte
160 will make the handler **panic and return 500** for every page that lists
it. That turns ordinary non-ASCII content into a denial-of-service of the
public site. The fix is to truncate on a character boundary instead of a raw
byte offset; behavior for ASCII content is unchanged.

## Current state

- `src/views/layout.rs` — `derive_excerpt` is the shared excerpt helper; all
  list/description rendering calls it with `max_length = 160`.
  - Call sites: `src/views/index.rs:21`, `src/views/tags.rs:12`,
    `src/views/search.rs:12`, `src/views/document.rs:15`.
- Current code (`src/views/layout.rs:280-302`):

```rust
pub fn derive_excerpt(markdown: &str, max_length: usize) -> String {
    let stripped = markdown
        .replace("```", " ")
        .replace('`', "")
        .replace("**", "")
        .replace("__", "")
        .replace(['*', '_', '~'], "");
    let text = stripped
        .lines()
        .map(|line| line.trim_start_matches('#').trim())
        .collect::<Vec<_>>()
        .join(" ");
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.len() <= max_length {
        return text;
    }
    let clipped = &text[..max_length];          // <-- PANICS if max_length is mid-codepoint
    let clipped = clipped
        .rsplit_once(' ')
        .map(|(head, _)| head)
        .unwrap_or(clipped);
    format!("{}…", clipped.trim_end())
}
```

Note `text.len()` is a **byte** length, so the `<= max_length` guard and the
slice are both byte-based and consistent — only the slice is unsafe. Keep the
guard and the trailing word-trim (`rsplit_once(' ')`) behavior intact.

Conventions: layout helpers are plain `pub fn` returning `String`; they are
unit-tested directly from `tests/view_layout_contract.rs` (integration test
file that imports `inkwell::views::layout::*`). Match that test style.

## Commands you will need

| Purpose   | Command                                                          | Expected on success |
|-----------|------------------------------------------------------------------|---------------------|
| Format    | `cargo fmt --check`                                              | exit 0              |
| Lint      | `cargo clippy --all-targets --all-features -- -D warnings`       | exit 0, no warnings |
| Tests     | `cargo test --test view_layout_contract`                         | all pass            |
| Full test | `cargo test --all`                                               | all pass            |

(`view_layout_contract` tests are pure unit tests and do not require a database.
`cargo test --all` exercises DB-backed contracts; set `DATABASE_URL` if you run
it — see README. This plan's new test does NOT need a database.)

## Scope

**In scope** (the only files you should modify):
- `src/views/layout.rs` — fix the slice in `derive_excerpt`.
- `tests/view_layout_contract.rs` — add a regression test.

**Out of scope** (do NOT touch):
- The four call sites (`index.rs`, `tags.rs`, `search.rs`, `document.rs`) — they
  call the shared function and need no change once it is fixed.
- `date_line`'s `&text[..10]` (`src/views/layout.rs:310`) — that slice is over an
  RFC3339 timestamp whose first 10 bytes are always the ASCII `YYYY-MM-DD` date;
  it is safe by construction. Do not change it as part of this plan.
- Any change to the excerpt's visible output for ASCII input or its `max_length`
  semantics.

## Git workflow

- Branch: `advisor/020-fix-excerpt-utf8-panic`
- Commit message style: conventional commits, e.g.
  `fix(views): truncate excerpt on char boundary to avoid UTF-8 panic`
- Do NOT push or open a PR unless the operator instructed it.

## Steps

### Step 1: Truncate on a character boundary

In `src/views/layout.rs`, replace the unsafe slice line:

```rust
    let clipped = &text[..max_length];
```

with a boundary-safe truncation that walks down to the nearest valid char
boundary at or below `max_length`:

```rust
    let mut end = max_length;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    let clipped = &text[..end];
```

Leave the subsequent `rsplit_once(' ')` word-trim and the `format!("{}…", ...)`
line exactly as they are. `str::is_char_boundary` is std (no new dependency).

**Verify**: `cargo build` → exit 0. `cargo clippy --all-targets --all-features -- -D warnings` → exit 0.

### Step 2: Add a regression test

In `tests/view_layout_contract.rs`, add a `#[test]` that proves the function no
longer panics and returns valid UTF-8 for a multibyte boundary case. Model it on
the existing `#[test]` functions in that file. Import `derive_excerpt`
(`use inkwell::views::layout::derive_excerpt;` or extend the existing `use`).

The test must construct an input whose **stripped, whitespace-collapsed** text
exceeds 160 bytes and places a multibyte character across the byte-160 boundary.
A reliable construction: a string of 159 ASCII `a` characters followed by a
multibyte char (e.g. `"é"`, which is 2 bytes, or `"😀"`, 4 bytes) and then more
text, so that byte index 160 falls inside that character. Example shape:

```rust
#[test]
fn derive_excerpt_truncates_on_char_boundary_without_panicking() {
    let body = format!("{}{}", "a".repeat(159), "😀 trailing words here");
    let excerpt = derive_excerpt(&body, 160);
    // Must not panic, must be valid UTF-8 (guaranteed by String), and must be
    // no longer than the input.
    assert!(excerpt.ends_with('…'));
    assert!(excerpt.len() <= body.len());
}
```

Also keep/confirm an ASCII case still ends with `…` and trims at a word
boundary, to lock in unchanged behavior (add a second short assertion or a
second test).

**Verify**: `cargo test --test view_layout_contract` → all pass, including the
new test. Before the Step 1 fix this test would panic; after it, it passes.

### Step 3: Full check

**Verify**:
- `cargo fmt --check` → exit 0
- `cargo clippy --all-targets --all-features -- -D warnings` → exit 0
- `cargo test --test view_layout_contract` → all pass

## Test plan

- New test in `tests/view_layout_contract.rs`: multibyte-boundary input does not
  panic and produces a valid truncated excerpt ending in `…`.
- Optional second assertion/test: ASCII input still truncates at a word boundary
  (regression guard for unchanged behavior).
- Structural pattern: the existing `#[test] fn render_page_*` functions in the
  same file.
- Verification: `cargo test --test view_layout_contract` → all pass, 1+ new test.

## Done criteria

ALL must hold:

- [ ] `grep -n "&text\[\.\.max_length\]" src/views/layout.rs` returns no matches
- [ ] `src/views/layout.rs` `derive_excerpt` uses `is_char_boundary` (or another
      boundary-safe truncation)
- [ ] `cargo clippy --all-targets --all-features -- -D warnings` exits 0
- [ ] `cargo test --test view_layout_contract` exits 0; the new multibyte test exists and passes
- [ ] `cargo fmt --check` exits 0
- [ ] Only `src/views/layout.rs` and `tests/view_layout_contract.rs` are modified (`git status`)
- [ ] `plans/README.md` status row for 020 updated to DONE

## STOP conditions

Stop and report back (do not improvise) if:

- The `derive_excerpt` code at `src/views/layout.rs` does not match the "Current
  state" excerpt (the file has drifted since this plan was written).
- The new test passes even *without* the Step 1 fix (means the boundary case was
  not actually constructed — re-derive the input so byte 160 lands inside a
  multibyte char).
- `cargo clippy` flags the boundary loop and a reasonable rewrite still fails twice.

## Maintenance notes

- If `max_length` is ever made caller-configurable or the stripping logic
  changes, the boundary truncation still holds — it depends only on `text` and
  `max_length`.
- A reviewer should confirm the fix is in the shared `derive_excerpt` (so all
  four call sites benefit) and that no call site re-implements its own slice.
- Plan 021 consolidates the duplicated list-rendering callers; it does not change
  `derive_excerpt` and is independent of this fix. Either order is safe.
