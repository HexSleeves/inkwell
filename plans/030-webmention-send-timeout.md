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

`src/federation/fetch.rs` defines `FETCH_TIMEOUT = Duration::from_secs(10)` and uses it for **receive verification** (the SSRF-guarded fetch that confirms a source URL links back to the target). However, it is not confirmed that `send_webmention` in `src/federation/webmention.rs` uses `fetch_with_ssrf_guard` for its outbound calls. If it constructs its own `reqwest::Client` without a timeout, a slow or hostile target can hold the spawned task open for the default (infinite) TCP timeout, exhausting the Tokio thread pool over time.

This plan is **investigate-then-fix**: verify whether the send path already has a timeout via the guarded fetch, and if not, add one.

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
`wm` is `crate::federation::webmention`.

**`src/federation/fetch.rs:29–31`** — existing timeout constant:
```rust
/// Per-request total timeout. Short so a slow or hostile endpoint can't tie up resources.
const FETCH_TIMEOUT: Duration = Duration::from_secs(10);
```

**`src/federation/fetch.rs:130–137`** — timeout applied in guarded client:
```rust
let client = reqwest::Client::builder()
    .timeout(remaining)
    // ...
    .build()
    .context("building guarded HTTP client")?;
```

The question is whether `send_webmention` calls `fetch_with_ssrf_guard` or builds its own client.

## Commands you will need

| Purpose   | Command                                                 | Expected on success |
|-----------|---------------------------------------------------------|---------------------|
| Inspect   | `grep -n "reqwest\|timeout\|fetch_with\|Client" src/federation/webmention.rs` | shows client usage |
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
- Does `send_webmention` call `fetch_with_ssrf_guard` or `super::fetch::fetch_with_ssrf_guard`? If yes → it already has the 10s timeout → **this plan is DONE, mark REJECTED in README.md**.
- Does it build its own `reqwest::Client`? If yes, does it set `.timeout(Duration::from_secs(10))`?

If the timeout is already present: mark this plan as REJECTED with reason "send path uses fetch_with_ssrf_guard which has built-in 10s FETCH_TIMEOUT". Update `plans/README.md`. Done.

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

- [ ] `src/federation/webmention.rs` send path has a timeout ≤ 15 seconds (either via `fetch_with_ssrf_guard` or explicit `.timeout()`)
- [ ] `cargo check --all-targets` exits 0
- [ ] `cargo nextest run` exits 0
- [ ] `plans/README.md` status row updated (either DONE or REJECTED-with-reason)

## STOP conditions

- `send_webmention` calls a chain of 4+ internal helpers making it unclear where to add the timeout. Report the call chain.
- Adding a timeout breaks existing webmention tests.

## Maintenance notes

- If a per-target timeout is needed (vs. total task timeout), `tokio::time::timeout` wrapping each `wm::send_webmention` call in `webmention_send.rs` is cleaner than per-client configuration.
- `MAX_SEND_TARGETS = 50` already bounds the total number of outbound calls; with a 10s per-request timeout the worst case is 50 × 10s = 500s for one publish — acceptable for a detached background task.
