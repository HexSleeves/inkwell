# Plan 030: Add timeout to outbound Webmention send path

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `plans/README.md`.
>
> **Drift check (run first)**: `git diff --stat 0819727..HEAD -- src/http/webmention_send.rs src/federation/webmention.rs src/federation/fetch.rs`
> If any in-scope file changed, compare the "Current state" excerpts before proceeding.

## Status

- **Priority**: P3
- **Effort**: S
- **Risk**: LOW
- **Depends on**: none
- **Category**: security/reliability
- **Planned at**: commit `0819727`, 2026-06-26

## Why this matters

When `INKWELL_WEBMENTION_SEND=true`, publishing a note spawns a detached `tokio::task` that sends a Webmention to each external URL in the note's body. The send path calls `federation::webmention::send_webmention`, which in turn makes outbound HTTP requests to discover Webmention endpoints and POST to them.

`src/federation/fetch.rs` defines `FETCH_TIMEOUT = Duration::from_secs(10)` and applies it inside the guarded HTTP helpers `guarded_get` / `guarded_post` (via `guarded_request`, which sets `.timeout(remaining)`). The receive path uses these; the send path in `src/federation/webmention.rs` (`send_webmention`) calls `guarded_get` then `guarded_post` (imported from `super::fetch`), so it almost certainly **already inherits the 10s timeout**.

This plan is therefore **investigate-then-confirm**: verify that `send_webmention` routes through `guarded_get`/`guarded_post` (it appears to), and if so mark this plan **REJECTED — no fix needed**. Only if the send path builds its own un-timed `reqwest::Client` is there anything to do. The expected outcome is REJECTED; the plan exists to make that determination explicit rather than assume it.

## Current state

**`src/http/webmention_send.rs:49–63`** — spawned send task:
```rust
tokio::spawn(async move {
    for target in targets.into_iter().take(MAX_SEND_TARGETS) {
        match wm::send_webmention(&source_url, &target).await {
            // ...
        }
    }
});
```
`wm` is `crate::federation::webmention`. `send_webmention` imports and calls `guarded_get` then `guarded_post` from `super::fetch` (import at the top of `webmention.rs`).

**`src/federation/fetch.rs:29–31`** — existing timeout constant:
```rust
/// Per-request total timeout. Short so a slow or hostile endpoint can't tie up
/// a worker — federation fetches are best-effort and must never block for long.
const FETCH_TIMEOUT: Duration = Duration::from_secs(10);
```

**`src/federation/fetch.rs:130–137`** — timeout applied in the guarded client built by `guarded_request` (the shared helper behind `guarded_get`/`guarded_post`):
```rust
let client = reqwest::Client::builder()
    .timeout(remaining)
    // ...
    .build()
    .context("building guarded HTTP client")?;
```

The question is whether `send_webmention` routes through `guarded_get`/`guarded_post` (which apply this timeout — evidence says yes) or builds its own un-timed client.

## Commands you will need

| Purpose   | Command                                                 | Expected on success |
|-----------|---------------------------------------------------------|---------------------|
| Inspect   | `grep -n "guarded_\|reqwest\|timeout\|Client" src/federation/webmention.rs` | shows `guarded_get`/`guarded_post` usage |
| Typecheck | `cargo check --all-targets`                              | exit 0              |
| Tests     | `cargo nextest run --test webmention_contract`           | all pass            |
| All tests | `cargo nextest run`                                      | all pass            |

## Scope

**In scope**:
- `src/federation/webmention.rs` — ensure `send_webmention` uses a client with timeout
- `src/http/webmention_send.rs` — if needed, pass timeout or use guarded fetch

**Out of scope**:
- `src/http/webmention.rs` (receive path) — SSRF guard already applied there
- `src/federation/fetch.rs` — do not change the existing FETCH_TIMEOUT value

## Git workflow

- Branch: `advisor/030-webmention-timeout`
- Commit: `fix(federation): add timeout to webmention send path`

## Steps

### Step 1: Investigate — does send_webmention already have a timeout?

Read `src/federation/webmention.rs` in full. Check:
- Does `send_webmention` call `guarded_get` / `guarded_post` (from `super::fetch`)? If yes → those route through `guarded_request`, which applies the 10s `FETCH_TIMEOUT` (`src/federation/fetch.rs:130-137`) → it **already has a timeout** → **this plan is DONE, mark REJECTED in README.md**. (This is the expected outcome.)
- Only if it instead builds its own `reqwest::Client` directly: check whether that builder sets `.timeout(...)`. If not, proceed to Step 2.

If the timeout is already present (expected): mark this plan REJECTED with reason "send path uses guarded_get/guarded_post which apply the 10s FETCH_TIMEOUT". Update `plans/README.md`. Done.

**Verify**: You have read `src/federation/webmention.rs` and determined whether a timeout exists.

### Step 2 (only if no timeout found): Add timeout to send path

If `send_webmention` builds its own `reqwest::Client` without a timeout, change the client builder to add:
```rust
.timeout(std::time::Duration::from_secs(10))
```

If `send_webmention` does not use `reqwest::Client` directly but calls an internal helper, trace the call chain and add the timeout at the lowest HTTP-making level.

Use the same `FETCH_TIMEOUT` constant from `src/federation/fetch.rs` by importing it, or define a local `const SEND_TIMEOUT: Duration = Duration::from_secs(10)` in `federation/webmention.rs`.

**Verify**: `cargo check --all-targets` → exit 0

### Step 3 (only if step 2 ran): Run tests

**Verify**: `cargo nextest run` → all pass

## Test plan

If the send path had no timeout, add a comment in the relevant function documenting the timeout. No new integration test is required (testing outbound HTTP timeouts in CI would require a mock server or sleeping, which is flaky).

## Done criteria

- [ ] `src/federation/webmention.rs` send path has a timeout ≤ 15 seconds (via `guarded_get`/`guarded_post` → `FETCH_TIMEOUT`, or an explicit `.timeout()` if a bespoke client is found)
- [ ] `cargo check --all-targets` exits 0
- [ ] `cargo nextest run` exits 0
- [ ] `plans/README.md` status row updated (either DONE or REJECTED-with-reason)

## STOP conditions

- `send_webmention` calls a chain of 4+ internal helpers making it unclear where to add the timeout. Report the call chain.
- Adding a timeout breaks existing webmention tests.

## Maintenance notes

- If a per-target timeout is needed (vs. total task timeout), `tokio::time::timeout` wrapping each `wm::send_webmention` call in `webmention_send.rs` is cleaner than per-client configuration.
- `MAX_SEND_TARGETS = 50` already bounds the total number of outbound calls; with a 10s per-request timeout the worst case is 50 × 10s = 500s for one publish — acceptable for a detached background task.
