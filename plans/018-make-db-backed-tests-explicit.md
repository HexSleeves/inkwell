# Plan 018: Make database-backed tests explicit instead of silently skipped

> **Executor instructions**: Follow this plan step by step. Run every verification command and confirm the expected result before moving on. If a STOP condition occurs, stop and report instead of improvising. When done, update this plan's row in `plans/README.md`.
>
> **Drift check (run first)**: `git diff --stat 8bcd1ea..HEAD -- tests/common/mod.rs tests/api_contract.rs .github/workflows/ci.yml README.md`
> If any in-scope file changed, compare the excerpts below with live code before editing. On mismatch, treat it as a STOP condition.

## Status

- **Priority**: P2
- **Effort**: S
- **Risk**: LOW
- **Depends on**: none
- **Category**: tests / DX
- **Planned at**: commit `8bcd1ea`, 2026-06-19

## Why this matters

`cargo test --all` exits 0 without `DATABASE_URL`, but the only API contract test silently returns early. That makes local verification look stronger than it is: API routing, auth, SQL, and migrations are not exercised unless the environment happens to provide Postgres. CI does provide `DATABASE_URL`, so the project has a real integration path; the gap is that the test output does not make skipped database coverage explicit and CI does not assert that database-backed tests actually ran.

## Current state

```rust
// tests/common/mod.rs:9-18
pub async fn maybe_pool() -> Result<Option<PgPool>> {
    let Ok(database_url) = std::env::var("DATABASE_URL") else {
        return Ok(None);
    };
    let pool = create_pool(&database_url)?;
    migrations::migrate(&pool).await?;
    sqlx::query("TRUNCATE TABLE documents RESTART IDENTITY")
        .execute(&pool)
        .await?;
    Ok(Some(pool))
}
```

```rust
// tests/api_contract.rs:7-11
#[tokio::test]
async fn create_and_fetch_document() -> anyhow::Result<()> {
    let Some(router) = common::maybe_router().await? else {
        return Ok(());
    };
```

CI has a real Postgres service and sets `DATABASE_URL`:

```yaml
# .github/workflows/ci.yml:31-33
- run: cargo test --all
  env:
    DATABASE_URL: postgres://inkwell:inkwell@localhost:5432/inkwell_test
```

Verification observed at plan time with no `DATABASE_URL`: `cargo test --all` passed, and `tests/api_contract.rs` reported one passing test even though it returned before issuing HTTP requests.

## Commands you will need

| Purpose | Command | Expected on success |
|---|---|---|
| Format | `cargo fmt --check` | exit 0 |
| Lint | `cargo clippy --all-targets --all-features -- -D warnings` | exit 0, no warnings |
| Tests | `cargo test --all` | exit 0 |
| DB tests | `INKWELL_REQUIRE_DB_TESTS=1 cargo test --all` with no `DATABASE_URL` | fails with a clear message |

## Scope

**In scope**:

- `tests/common/mod.rs`
- `tests/api_contract.rs`
- New tests under `tests/` if needed
- `.github/workflows/ci.yml`
- `README.md` test instructions, if they need clarification

**Out of scope**:

- Introducing Docker/testcontainers.
- Replacing SQLx/Postgres integration tests with mocks.
- Changing application code.

## Git workflow

- Branch: `advisor/018-explicit-db-tests`
- Commit style: `test: require db-backed contracts in ci` or similar.
- Do not push or open a PR unless instructed.

## Steps

### Step 1: Add a required-DB mode to the test helper

Update `tests/common/mod.rs` so missing `DATABASE_URL` behaves as follows:

- Default local mode: return `Ok(None)` but print a clear skip message to stderr or stdout once per helper call.
- Required mode: when `INKWELL_REQUIRE_DB_TESTS=1`, return an error explaining that `DATABASE_URL` is required for database-backed contract tests.

Keep existing callers using `maybe_router()` so local no-DB runs remain possible.

**Verify**: `cargo test --all` without `DATABASE_URL` -> exit 0 and includes a visible skip message when not captured with `--nocapture` only if the Rust harness shows it on failure; do not force noisy passing output if the harness captures it.

### Step 2: Require DB-backed contracts in CI

Update `.github/workflows/ci.yml` so the test step sets:

```yaml
INKWELL_REQUIRE_DB_TESTS: "1"
```

Keep the existing `DATABASE_URL` value. This turns any future CI misconfiguration into a clear failure instead of a false green.

**Verify**: `INKWELL_REQUIRE_DB_TESTS=1 cargo test --all` with no `DATABASE_URL` -> nonzero exit with the new clear error.

### Step 3: Document the local behavior

If README still tells users to run `cargo test --all` before exporting `DATABASE_URL`, clarify that full integration coverage needs Postgres and `DATABASE_URL`; otherwise the DB-backed contracts are skipped locally.

**Verify**: `cargo fmt --check` -> exit 0.

### Step 4: Run normal verification

Run lint and tests.

**Verify**:

- `cargo clippy --all-targets --all-features -- -D warnings` -> exit 0.
- `cargo test --all` -> exit 0.

## Test plan

- Existing `tests/api_contract.rs` remains the primary DB-backed contract.
- Add a small unit test for the helper behavior only if the environment-variable branching can be tested without racing global process state. If not, rely on the command-level verification above and do not add brittle env-var tests.

## Done criteria

- [ ] CI sets `INKWELL_REQUIRE_DB_TESTS=1` on `cargo test --all`.
- [ ] Missing `DATABASE_URL` fails clearly when `INKWELL_REQUIRE_DB_TESTS=1`.
- [ ] Local `cargo test --all` remains possible without a database, but the skip is explicit in code/docs.
- [ ] `cargo fmt --check`, `cargo clippy --all-targets --all-features -- -D warnings`, and `cargo test --all` exit 0 in the default local environment.

## STOP conditions

Stop and report if:

- The project owner wants `cargo test --all` to fail locally without Postgres.
- CI no longer provides Postgres by the time this plan is executed.
- Making the skip visible requires unstable or noisy harness behavior.

## Maintenance notes

Any future DB-backed test should use the same helper so CI remains fail-closed while local development can still run unit tests without Docker/Postgres.

