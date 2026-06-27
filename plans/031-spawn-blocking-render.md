# Plan 031: Wrap CPU-heavy Markdown rendering in spawn_blocking

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `plans/README.md`.
>
> **Drift check (run first)**: `git diff --stat 0819727..HEAD -- src/garden.rs src/rendering/`
> If any in-scope file changed, compare the "Current state" excerpts before proceeding.

## Status

- **Priority**: P1
- **Effort**: S
- **Risk**: LOW
- **Depends on**: none
- **Category**: perf
- **Planned at**: commit `0819727`, 2026-06-26

## Why this matters

`render_and_resolve` in `src/garden.rs` calls `render_markdown_with_embeds` → Comrak `parse_document` + HTML formatting, then `sanitize_html` (Ammonia). Both are CPU-heavy synchronous operations called directly from an async context (the Tokio multi-thread runtime). Calling blocking CPU work without `tokio::task::spawn_blocking` prevents the worker thread from being used for other futures while the CPU-heavy work runs. Under concurrent writes with large documents (or documents with deep embed trees), this can starve the Tokio runtime.

Comrak and Ammonia are not Tokio-aware. The fix is standard: wrap the CPU-intensive work in `tokio::task::spawn_blocking` so Tokio can schedule other work on the thread pool while the blocking call runs on a dedicated blocking thread.

## Current state

**`src/garden.rs:57–80`** — `render_and_resolve` signature and early logic (async fn calling sync work):
```rust
pub async fn render_and_resolve(
    pool: &PgPool,
    markdown: &str,
) -> Result<(String, Vec<ResolvedRef>), sqlx::Error> {
    let refs = extract_wikilinks(markdown);
    // ... resolve slugs (DB call, correctly awaited) ...
    let embeds = resolve_embeds(pool, &embed_slugs, Visibility::Public, ...).await?;
    // ... then render_markdown_with_embeds (CPU-heavy, called inline) ...
}
```

**The CPU-heavy call** is in `render_markdown_with_embeds` from `src/rendering/wikilink.rs`. It calls Comrak + Ammonia internally. The entire rendering step runs on the async Tokio thread, blocking it.

**Convention**: The `garden.rs` module already uses async/await for DB calls. `tokio::task::spawn_blocking` returns a `JoinHandle`; `.await?` on it returns `Result<T, JoinError>` — map the `JoinError` to the appropriate error type (see Step 2).

**Exemplar**: The pattern is: collect owned data, move it into `spawn_blocking`, return the result:
```rust
let result = tokio::task::spawn_blocking(move || {
    // synchronous CPU work here
    heavy_sync_function(&owned_data)
}).await.expect("render task panicked");
```

## Commands you will need

| Purpose   | Command                                  | Expected on success |
|-----------|------------------------------------------|---------------------|
| Typecheck | `cargo check --all-targets`              | exit 0, no errors   |
| Tests     | `cargo nextest run --test links_contract` | all pass            |
| All tests | `cargo nextest run`                       | all pass            |
| Lint      | `cargo clippy --all-targets -- -D warnings` | exit 0           |

## Scope

**In scope**:
- `src/garden.rs` — wrap the rendering step in `spawn_blocking`
- `src/rendering/wikilink.rs` — may need to ensure types are `Send` (owned `String`, not `&str`)

**Out of scope**:
- `src/rendering/markdown.rs`, `src/rendering/sanitize.rs`, `src/rendering/highlight.rs` — do not change the rendering functions themselves, only how they are called
- Any change to the async/await structure of DB calls in `garden.rs`

## Git workflow

- Branch: `advisor/031-spawn-blocking-render`
- Commit: `perf(garden): wrap Comrak/Ammonia rendering in spawn_blocking`

## Steps

### Step 1: Identify the exact call site to wrap

Read `src/garden.rs` and find the call to `render_markdown_with_embeds` (or equivalent rendering call). Note:
- What are the inputs? They must be `'static + Send` to move into `spawn_blocking`.
- What does it return? It must be `'static + Send` to come back from the closure.

In most cases the inputs are `String` (owned Markdown body) and `HashMap<String, String>` (embed bodies) — both `Send`. The output is `String` (rendered HTML) — also `Send`.

**Verify**: You know the exact line and the types of all inputs/outputs.

### Step 2: Wrap the rendering call

Extract the CPU-only rendering step into a `spawn_blocking` call. The DB I/O (embed fetches via `resolve_embeds`) must remain outside `spawn_blocking` because it is async. Pattern:

```rust
// DB calls first (async, must stay outside spawn_blocking):
let embed_bodies: HashMap<String, String> = fetch_embed_bodies(pool, ...).await?;

// CPU rendering (sync, wrap in spawn_blocking):
let (rendered_html, refs) = tokio::task::spawn_blocking(move || {
    render_markdown_with_embeds(&markdown, &embed_bodies)
})
.await
.unwrap_or_else(|e| panic!("render task panicked: {e}"));
```

If the rendering function returns a `Result`, handle the inner error before unwrapping the JoinHandle:
```rust
.await
.expect("render task did not panic")  // JoinError: only on task panic
?                                       // inner Result error
```

The function signature of `render_and_resolve` does not change — callers see the same interface.

**Verify**: `cargo check --all-targets` → exit 0

### Step 3: Run all tests

**Verify**: `cargo nextest run` → all pass (especially `links_contract` which exercises `render_and_resolve`)

## Test plan

No new tests needed — this is a correctness-preserving refactor. The existing tests in `tests/links_contract.rs`, `tests/api_contract.rs`, and `tests/rendering_contract.rs` already exercise the render path and will catch regressions.

## Done criteria

- [ ] `cargo check --all-targets` exits 0
- [ ] `cargo clippy --all-targets -- -D warnings` exits 0
- [ ] `cargo fmt --check` exits 0
- [ ] `cargo nextest run` exits 0; all existing tests pass
- [ ] `grep -n "render_markdown_with_embeds\|render_and_resolve" src/garden.rs` shows the rendering call is inside a `spawn_blocking` closure
- [ ] No `spawn_blocking` is called for DB I/O (only for CPU work)
- [ ] `plans/README.md` status row updated

## STOP conditions

- Rendering inputs are not `'static + Send` (e.g., they contain `&PgPool` or a non-Send type). Report the specific type and stop.
- `cargo check` reports a `Send` bound failure. Report the type and stop.
- `rerender_one` in `garden.rs` (which also calls `render_and_resolve`) needs separate attention — it should benefit automatically since `render_and_resolve` is what changes.

## Maintenance notes

- All callers of `render_and_resolve` (create, update, `rerender_one`) benefit automatically from this change.
- If embed depth causes multiple sequential DB fetches inside `render_and_resolve`, the `spawn_blocking` wraps only the CPU part — DB fetches remain async and are called before/after the blocking step.
- Future: if rendering latency under load is still a concern, the blocking thread pool size can be configured via `TOKIO_BLOCKING_THREADS` env var (default: 512 in tokio multi-thread).
