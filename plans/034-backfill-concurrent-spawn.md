# Plan 034: Bound and concurrentize backfill fan-out with tokio::spawn + semaphore

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `plans/README.md`.
>
> **Drift check (run first)**: `git diff --stat 0819727..HEAD -- src/garden.rs`
> If the file changed since this plan was written, compare the "Current state"
> excerpts against the live code; on a mismatch, treat it as a STOP condition.

## Status

- **Priority**: P2
- **Effort**: M
- **Risk**: MED
- **Depends on**: plans/031-spawn-blocking-render.md (recommended first; not strictly required)
- **Category**: perf
- **Planned at**: commit `0819727`, 2026-06-26

## Why this matters

`rerender_sources` in `src/garden.rs:301–307` loops sequentially through all affected source documents and calls `rerender_one` for each. For a popular "hub" note with 50+ inbound `[[links]]`, one slug rename triggers 50 sequential re-renders in the same async context, blocking the response and the thread for the duration. The function is called as best-effort after a successful write, but "best-effort" does not mean "can run indefinitely before the response goes out."

Two changes together fix this:
1. **Cap**: limit the fan-out to `MAX_BACKFILL` (e.g., 50) documents per call; log a warning when truncated.
2. **Concurrent**: replace the sequential loop with bounded concurrent `tokio::spawn` tasks (semaphore at N=4 concurrent re-renders), so the slowest single re-render doesn't serialize all of them.

The `best-effort` contract is preserved: individual failures are logged and dropped. The response is sent before the background tasks complete.

## Current state

**`src/garden.rs:297–330`** — sequential fan-out:
```rust
pub async fn rerender_sources(pool: &PgPool, ids: &[Uuid]) {
    for &id in ids {
        if let Err(error) = rerender_one(pool, id).await {
            tracing::warn!(note_id = %id, %error, "re-render failed; stub may be stale until next save");
        }
    }
}

async fn rerender_one(pool: &PgPool, id: Uuid) -> Result<(), sqlx::Error> {
    let Some((_slug, body_markdown)) = sqlx::query_as::<Postgres, (String, String)>(
        "SELECT slug, body_markdown FROM documents WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?
    else { return Ok(()); };
    let (html, refs) = render_and_resolve(pool, &body_markdown).await?;
    documents::set_rendered_html(pool, id, &html).await?;
    persist_source_edges(pool, id, &refs).await
}

pub async fn backfill_after_change(pool: &PgPool, note_id: Uuid, slug: &str) {
    let affected = affected_sources(pool, note_id, slug).await;
    rerender_sources(pool, &affected).await;
}
```

**Convention**: `backfill_after_change` is called after a successful write in handlers (`src/http/api.rs`). It is already structured as best-effort (handler does not await its result — or if it does, failures are ignored). Confirm the call site pattern before changing.

**Tokio semaphore pattern**:
```rust
use std::sync::Arc;
use tokio::sync::Semaphore;

let sem = Arc::new(Semaphore::new(4));
let mut handles = Vec::with_capacity(ids.len());
for &id in ids.iter().take(MAX_BACKFILL) {
    let pool = pool.clone();
    let permit = sem.clone().acquire_owned().await.unwrap();
    handles.push(tokio::spawn(async move {
        let _permit = permit; // held for duration of task
        rerender_one(&pool, id).await
    }));
}
for handle in handles {
    if let Err(e) = handle.await.unwrap_or(Err(sqlx::Error::RowNotFound)) {
        tracing::warn!(%e, "backfill re-render failed");
    }
}
```

## Commands you will need

| Purpose   | Command                                        | Expected on success |
|-----------|------------------------------------------------|---------------------|
| Typecheck | `cargo check --all-targets`                    | exit 0              |
| Tests     | `cargo nextest run --test links_contract`      | all pass            |
| All tests | `cargo nextest run`                            | all pass            |
| Lint      | `cargo clippy --all-targets -- -D warnings`    | exit 0              |

## Scope

**In scope**:
- `src/garden.rs` — `rerender_sources` and `backfill_after_change`

**Out of scope**:
- `src/http/api.rs` call sites — do not change how handlers call `backfill_after_change`
- `rerender_one` — keep the function, just call it concurrently
- `affected_sources` — keep as-is

## Git workflow

- Branch: `advisor/034-backfill-concurrent`
- Commit: `perf(garden): bound and concurrentize backfill fan-out`

## Steps

### Step 1: Add cap constant and semaphore-based concurrent loop

Add to `src/garden.rs` (top of file, near other constants):
```rust
const MAX_BACKFILL: usize = 50;
const BACKFILL_CONCURRENCY: usize = 4;
```

Replace `rerender_sources` sequential loop with the semaphore pattern (see Current state section). The function signature `async fn rerender_sources(pool: &PgPool, ids: &[Uuid])` stays the same.

Key changes:
- `ids.iter().take(MAX_BACKFILL)` — cap at 50
- Log a warning if `ids.len() > MAX_BACKFILL`:
  ```rust
  if ids.len() > MAX_BACKFILL {
      tracing::warn!(total = ids.len(), cap = MAX_BACKFILL, "backfill fan-out truncated");
  }
  ```
- Use `Arc<Semaphore>` for bounded concurrency
- Collect handles and join them

`PgPool` is `Clone` — clone per task is correct (`pool.clone()` shares the connection pool, not creating new connections).

**Verify**: `cargo check --all-targets` → exit 0

### Step 2: Verify backfill_after_change call sites

Run: `grep -n "backfill_after_change" src/http/api.rs`

Confirm the call is best-effort (either `tokio::spawn`'d or the result is not propagated to the response). If it is awaited and its error would fail the response: **STOP and report** — the plan's concurrency change could change error behaviour.

If call sites are `if let Err(e) = garden::backfill_after_change(…).await { warn!… }` or similar: no change needed. Continue.

**Verify**: Confirmed call pattern.

### Step 3: Run all tests

**Verify**: `cargo nextest run` → all pass

## Test plan

The existing `tests/links_contract.rs` tests exercise backfill behaviour (stub → resolved transitions). These should continue to pass. No new concurrency-specific tests needed (race conditions in test environments are flaky).

## Done criteria

- [ ] `cargo check --all-targets` exits 0
- [ ] `cargo clippy --all-targets -- -D warnings` exits 0
- [ ] `cargo fmt --check` exits 0
- [ ] `cargo nextest run` exits 0
- [ ] `grep -n "for &id in ids" src/garden.rs` returns no matches (sequential loop removed)
- [ ] `src/garden.rs` has `MAX_BACKFILL` and `BACKFILL_CONCURRENCY` constants
- [ ] `plans/README.md` status row updated

## STOP conditions

- `backfill_after_change` call sites in `api.rs` propagate the error to the response (not best-effort). Changing to concurrent would need the call site restructured too.
- `PgPool` is not `Clone` (it is — this is just a safety check).
- `tokio::sync::Semaphore` is not available (it is in the `tokio` crate with `sync` feature — `rt-multi-thread` implies it).

## Maintenance notes

- `MAX_BACKFILL = 50` means a note with 100 inbound links will have 50 stale stubs after a rename. They self-heal on the next write of those notes. This is documented behavior — log it at `warn` level so operators can see when truncation occurs.
- `BACKFILL_CONCURRENCY = 4` balances throughput vs. DB connection pressure. Each concurrent task uses one pool connection for the duration of its re-render.
- Future: if backfill latency still matters post-031 (spawn_blocking), consider moving the entire `rerender_sources` call into a `tokio::spawn` from the handler, so the response is sent immediately and backfill runs fully detached.
