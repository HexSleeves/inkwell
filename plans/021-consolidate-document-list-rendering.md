# Plan 021: Consolidate the duplicated document-list rendering into one helper

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `plans/README.md`.
>
> **Drift check (run first)**: `git diff --stat c7b0a46..HEAD -- src/views/index.rs src/views/tags.rs src/views/search.rs src/views/layout.rs`
> If any of these changed since this plan was written, compare the "Current
> state" excerpts against the live code before proceeding; on a mismatch, treat
> it as a STOP condition.

## Status

- **Priority**: P3
- **Effort**: S
- **Risk**: LOW
- **Depends on**: none (coordinate with plans/020-fix-excerpt-utf8-panic.md — see Maintenance notes)
- **Category**: tech-debt
- **Planned at**: commit `c7b0a46`, 2026-06-20

## Why this matters

The same per-document list-item HTML (title link, date line, optional excerpt,
tag chips) is rendered in three places: an inline closure in
`render_index_page` and a byte-for-byte identical private `render_doc_list`
function in both `tags.rs` and `search.rs`. Any change to list-item markup,
excerpt length, or tag rendering must be made in three places and kept in sync;
a fix applied to only one or two of them silently diverges. Consolidating to a
single shared helper removes that drift risk and shrinks the surface a reviewer
must check.

## Current state

Three copies of the same list rendering:

- `src/views/tags.rs:8-41` — `fn render_doc_list(documents: &[Document]) -> String`.
- `src/views/search.rs:8-41` — identical `fn render_doc_list(documents: &[Document]) -> String`.
- `src/views/index.rs:18-43` — the same logic inlined as a `.map(|doc| { ... })`
  closure inside `render_index_page` (wrapped in the same `<ul class="index">…</ul>`).

The duplicated body (from `tags.rs`/`search.rs`):

```rust
fn render_doc_list(documents: &[Document]) -> String {
    let items = documents
        .iter()
        .map(|doc| {
            let excerpt = derive_excerpt(doc.body_markdown(), 160);
            let excerpt_html = if excerpt.is_empty() {
                String::new()
            } else {
                format!(
                    r#"\n            <p class="excerpt">{}</p>"#,
                    escape_html(&excerpt)
                )
            };
            format!(
                r#"          <li>
            <a class="title" href="/{}">{}</a>
            <div class="meta">{}</div>{}{}
          </li>"#,
                urlencoding::encode(&doc.slug),
                escape_html(&doc.title),
                date_line("Published", doc.created_at),
                excerpt_html,
                render_tag_chips(&doc.tags)
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        r#"<ul class="index">
{}
        </ul>"#,
        items
    )
}
```

`index.rs` produces the **same** string but as an inline closure. All three use
the shared helpers from `super::layout` (`derive_excerpt`, `escape_html`,
`date_line`, `render_tag_chips`).

Conventions: view modules are `pub fn render_*` returning `String`; shared
helpers live in `src/views/layout.rs` and are imported via
`use super::layout::{...}`. Output is locked by `tests/view_layout_contract.rs`
and exercised end-to-end by `tests/http_caching.rs` (which fetches `/`,
`/tags`, `/tags/rust`, `/search`). Preserve the exact emitted bytes.

## Commands you will need

| Purpose   | Command                                                          | Expected on success |
|-----------|------------------------------------------------------------------|---------------------|
| Format    | `cargo fmt --check`                                              | exit 0              |
| Lint      | `cargo clippy --all-targets --all-features -- -D warnings`       | exit 0, no warnings |
| Tests     | `cargo test --all`                                               | all pass            |

(`cargo test --all` runs DB-backed contracts including `http_caching.rs`; set
`DATABASE_URL` per README, or run against the Compose Postgres. The output of
the consolidated helper must be byte-identical, so these tests are the guard.)

## Scope

**In scope** (the only files you should modify):
- `src/views/layout.rs` — add one shared `pub(crate) fn render_document_list`.
- `src/views/tags.rs` — delete local `render_doc_list`, call the shared one.
- `src/views/search.rs` — delete local `render_doc_list`, call the shared one.
- `src/views/index.rs` — replace the inline closure with a call to the shared one.

**Out of scope** (do NOT touch):
- The emitted HTML bytes — this is a pure refactor; output must not change.
- `derive_excerpt` itself (that is plan 020's concern).
- The pager / `HeadMeta` / page-title logic in each view — only the list-item
  block is being consolidated.

## Git workflow

- Branch: `advisor/021-consolidate-document-list-rendering`
- Commit message style: conventional commits, e.g.
  `refactor(views): extract shared render_document_list helper`
- Do NOT push or open a PR unless the operator instructed it.

## Steps

### Step 1: Add the shared helper to `layout.rs`

In `src/views/layout.rs`, add a `pub(crate) fn render_document_list(documents:
&[Document]) -> String` whose body is exactly the `render_doc_list` body shown
in "Current state" (it already references `derive_excerpt`, `escape_html`,
`date_line`, `render_tag_chips`, all defined in this module). Add
`use crate::domain::document::Document;` if `Document` is not already in scope in
`layout.rs`.

**Verify**: `cargo build` → exit 0.

### Step 2: Switch `tags.rs` and `search.rs` to the shared helper

In both `src/views/tags.rs` and `src/views/search.rs`: delete the local
`fn render_doc_list`, and update the call sites (`render_doc_list(documents)`) to
`render_document_list(documents)`. Add `render_document_list` to the
`use super::layout::{...}` import and remove any imports that are now unused
(`derive_excerpt`, `escape_html`, `date_line`, `render_tag_chips` may no longer
be referenced directly in these files — let clippy tell you).

**Verify**: `cargo clippy --all-targets --all-features -- -D warnings` → exit 0
(no unused-import warnings).

### Step 3: Switch `index.rs` to the shared helper

In `src/views/index.rs`, replace the inline `.map(|doc| { ... }).collect().join`
block that builds the `<ul class="index">…</ul>` with a call to
`render_document_list(documents)` for the non-empty branch. Keep the empty-state
branch (`<p class="empty">No documents published yet.</p>`) and all pager/title
logic unchanged. Update imports as in Step 2.

**Verify**: `cargo clippy --all-targets --all-features -- -D warnings` → exit 0.

### Step 4: Full check — output must be unchanged

**Verify**:
- `cargo fmt --check` → exit 0
- `cargo test --all` → all pass (especially `view_layout_contract` and
  `http_caching`, which assert on the rendered HTML; if any HTML assertion fails,
  the refactor changed bytes — STOP, the helper body diverged from the originals).

## Test plan

- No new test is required — this is a behavior-preserving refactor guarded by the
  existing `tests/view_layout_contract.rs` and `tests/http_caching.rs` assertions
  on rendered HTML.
- If you want extra safety, add a `#[test]` in `view_layout_contract.rs` calling
  `render_document_list` with a one-document slice and asserting the `<li>` /
  `<a class="title">` / `<ul class="index">` markers appear (model on existing
  tests). Optional.
- Verification: `cargo test --all` → all pass.

## Done criteria

ALL must hold:

- [ ] `grep -rn "fn render_doc_list" src/views/` returns no matches
- [ ] `grep -rn "render_document_list" src/views/` shows one definition (layout.rs)
      and three call sites (index.rs, tags.rs, search.rs)
- [ ] `cargo clippy --all-targets --all-features -- -D warnings` exits 0
- [ ] `cargo test --all` exits 0 (all HTML-assertion tests still pass — output unchanged)
- [ ] `cargo fmt --check` exits 0
- [ ] Only the four in-scope files are modified (`git status`)
- [ ] `plans/README.md` status row for 021 updated to DONE

## STOP conditions

Stop and report back (do not improvise) if:

- The `render_doc_list` bodies in `tags.rs` and `search.rs` are NOT identical to
  each other or to the `index.rs` inline block (a divergence already exists —
  report it; do not silently pick one).
- Any rendered-HTML assertion in `view_layout_contract.rs` or `http_caching.rs`
  fails after the refactor (means the consolidated helper changed output).
- Consolidation appears to require changing `HeadMeta`, the pager, or any
  out-of-scope logic.

## Maintenance notes

- Coordinate with plans/020-fix-excerpt-utf8-panic.md: both touch
  `src/views/layout.rs`. They are independent (020 edits inside `derive_excerpt`;
  021 adds a new helper and edits the view files), so either can land first. If
  021 lands first, 020's fix in `derive_excerpt` still benefits all callers
  through the single shared `render_document_list`. If both are open at once,
  expect a trivial merge in `layout.rs`.
- After this lands, future list-item markup changes happen in exactly one place.
- A reviewer should diff the rendered HTML (or trust the contract tests) to
  confirm the bytes are unchanged.
