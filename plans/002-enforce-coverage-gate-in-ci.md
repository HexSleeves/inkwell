# Plan 002: Make the GitHub Actions CI enforce coverage thresholds

> **Executor instructions**: Follow the steps in order. Run every **Verify** command and confirm the expected result before moving on. Obey all STOP conditions — if `pnpm run test:coverage` fails the thresholds, STOP and report (do NOT weaken thresholds). When done, update the status row for Plan 002 in `plans/README.md` (create that file if it does not exist; see Steps). Do NOT push, commit, or open a PR unless the operator explicitly asks.

> **Drift check (run first)**: `git diff --stat 6bf6a27..HEAD -- .github/workflows/ci.yml package.json vitest.config.ts` — if any in-scope file changed, compare the Current-state excerpts below to the live code; on any mismatch, STOP and report the drift instead of editing.

## Status
- **Priority**: P1
- **Effort**: S
- **Risk**: LOW
- **Depends on**: none
- **Category**: dx
- **Planned at**: commit `6bf6a27`, 2026-06-19

## Why this matters
Coverage thresholds are already defined in `vitest.config.ts` (statements/lines/functions 80, branches 75) and the local `ci` npm script runs `pnpm run test:coverage`, which evaluates them. But the GitHub Actions workflow runs plain `pnpm run test` (`vitest run`, no `--coverage`), so the thresholds are NEVER checked on pull requests. Coverage can silently regress below the floor and CI stays green. This one-line change makes the merge-blocking CI actually enforce the gate that the project already decided on, closing a real hole with near-zero risk.

## Current state
Files and their role:
- `.github/workflows/ci.yml` — GitHub Actions workflow. Single `build` job, matrix `node-version: [20.x, 22.x]`, runs each script as its own named step (Lint / Check formatting / Typecheck / Test / Build). It does NOT call `pnpm run ci`; it invokes the individual scripts directly.
- `package.json` — defines the npm scripts the workflow calls.
- `vitest.config.ts` — single source of truth for the coverage thresholds. Thresholds are only evaluated when coverage is collected (i.e. `vitest run --coverage`).

Verified excerpts at HEAD `6bf6a27`:

`.github/workflows/ci.yml` (the relevant tail of the steps list):
```yaml
      - run: pnpm install --frozen-lockfile
      - name: Lint
        run: pnpm run lint
      - name: Check formatting
        run: pnpm run format:check
      - name: Typecheck
        run: pnpm run typecheck
      - name: Test
        run: pnpm run test
      - name: Build
        run: pnpm run build
```
The job header for context:
```yaml
jobs:
  build:
    runs-on: ubuntu-latest
    strategy:
      matrix:
        node-version: [20.x, 22.x]
```

`package.json` scripts (relevant lines):
```json
    "test": "vitest run",
    "test:watch": "vitest",
    "test:coverage": "vitest run --coverage",
    ...
    "ci": "pnpm run lint && pnpm run format:check && pnpm run typecheck && pnpm run test:coverage && pnpm run build"
```

`vitest.config.ts` (coverage block):
```ts
    coverage: {
      provider: 'v8',
      reporter: ['text', 'lcov'],
      include: ['src/**/*.ts'],
      exclude: ['src/**/*.test.ts'],
      thresholds: {
        statements: 80,
        branches: 75,
        functions: 80,
        lines: 80,
      },
    },
```

Note: `plans/` does not yet exist at HEAD; this plan file and `plans/README.md` are new.

Test/exemplar pattern to follow: there is NO app code to test here — this is a workflow/config change. The only "test" is running `pnpm run test:coverage` locally and confirming it exits 0 with the coverage table meeting thresholds. The `coverage` v8 provider requires `@vitest/coverage-v8`; if `pnpm run test:coverage` errors with a missing-dependency message about that package, STOP and report (do not install or change deps as part of this plan).

## Commands you will need
| Purpose | Command | Expected |
| Install | `pnpm install` | exit 0 |
| Coverage (the gate) | `pnpm run test:coverage` | exit 0, coverage table printed, all thresholds met |
| Full local gate | `pnpm run ci` | exit 0 |

## Scope
**In scope**:
- `.github/workflows/ci.yml` — change the Test step to run coverage.
- `plans/README.md` (create if absent) — add/update the Plan 002 status row.

**Out of scope** (do NOT touch):
- `vitest.config.ts` — do NOT lower or alter the thresholds; it stays the single source of truth.
- `package.json` — the `test:coverage` and `ci` scripts are already correct; no change needed.
- Any `src/**` files — no application code change.

## Git workflow
- Branch: `advisor/002-enforce-coverage-gate-in-ci`.
- Conventional-commit example (only if the operator asks you to commit):
  `ci: run vitest coverage in GitHub Actions to enforce thresholds`
- Do NOT push or open a PR unless explicitly asked.

## Steps

1. Run the drift check (top of this file). If any in-scope file differs from the excerpts above, STOP.
   **Verify**: `git diff --stat 6bf6a27..HEAD -- .github/workflows/ci.yml package.json vitest.config.ts` -> no unexpected in-scope changes (or matching live code).

2. Confirm the gate currently passes locally BEFORE changing CI. Run coverage.
   **Verify**: `pnpm run test:coverage` -> exit 0, coverage table printed, no "ERROR: Coverage ... does not meet threshold" lines. If it FAILS the thresholds, STOP (see STOP conditions).

3. Edit `.github/workflows/ci.yml`: change the Test step so it collects coverage. Replace:
   ```yaml
      - name: Test
        run: pnpm run test
   ```
   with:
   ```yaml
      - name: Test (with coverage gate)
        run: pnpm run test:coverage
   ```
   Leave every other step (Lint, Check formatting, Typecheck, Build), the matrix, and the install step unchanged.
   **Verify**: `git diff .github/workflows/ci.yml` -> shows exactly the Test step name and `run:` line changed, nothing else.

4. Re-confirm the local gate end-to-end (mirrors what CI now runs).
   **Verify**: `pnpm run ci` -> exit 0.

5. Update `plans/README.md` with the Plan 002 status row. If the file does not exist, create it with a header and a status table, then add the row. Suggested minimal content:
   ```markdown
   # Plans

   | Plan | Title | Priority | Status |
   | ---- | ----- | -------- | ------ |
   | 002 | Enforce coverage gate in CI | P1 | Done |
   ```
   If the file already exists, just set Plan 002's status to `Done` (add the row if missing), matching the existing table format.
   **Verify**: `grep -n "002" plans/README.md` -> shows the Plan 002 row with status Done.

## Test plan
No application tests are added or changed — this is a CI/config change.
- Regression coverage for this exact finding is the workflow change itself: CI now runs `pnpm run test:coverage`, so a future coverage regression below the thresholds will fail the merge-blocking job on both Node 20.x and 22.x.
- Local verification (model after: the `ci` npm script in `package.json`, which already chains `test:coverage`): `pnpm run test:coverage` exits 0 with the thresholds met, and `pnpm run ci` exits 0.
- Real verification happens in CI once merged: the Test step must pass on both matrix Node versions.

## Done criteria
- [ ] Drift check ran; no unexplained in-scope drift.
- [ ] `.github/workflows/ci.yml` Test step runs `pnpm run test:coverage` (not `pnpm run test`).
- [ ] `pnpm run test:coverage` exits 0 with thresholds met.
- [ ] `pnpm run ci` exits 0.
- [ ] `vitest.config.ts` thresholds unchanged; `package.json` unchanged.
- [ ] No out-of-scope files modified (`git status` shows only `.github/workflows/ci.yml` and `plans/README.md` plus this plan file).
- [ ] `plans/README.md` Plan 002 row updated to Done.

## STOP conditions
- `pnpm run test:coverage` FAILS the thresholds at HEAD (before your edit): coverage is already below the floor. Report the exact failing metric(s) and current vs. required percentages. Do NOT lower thresholds in `vitest.config.ts` to make it pass, and do NOT make the CI change (it would just make CI red).
- `pnpm run test:coverage` errors because `@vitest/coverage-v8` (the v8 provider) is not installed/resolvable: STOP and report; do not add or change dependencies under this plan.
- Drift check shows `.github/workflows/ci.yml` no longer matches the excerpt (e.g. the workflow was refactored to call `pnpm run ci` directly, which would already close this gap): STOP and report — the change may be unnecessary or need rework.

## Maintenance notes
- `vitest.config.ts` remains the single source of truth for thresholds. Reviewers should reject PRs that lower `statements`/`branches`/`functions`/`lines` without explicit justification.
- The thresholds carry intentional headroom (~86% lines/stmts, ~80% branches currently) so thin process/CLI entrypoints (`src/main.ts`, `src/db/cli.ts`, `src/db/pool.ts`) don't make the gate flaky. If a future change adds substantial untested code in those files, prefer adding tests over loosening the floor.
- Coverage now runs on both Node 20.x and 22.x; if the two ever diverge in coverage, investigate version-specific branches rather than relaxing the gate.
- Deferred follow-up (out of scope here): consider collapsing the individual CI steps into a single `pnpm run ci` invocation so the workflow and local gate can't drift again — but that loses per-step annotations in the Actions UI, so it's a separate decision.
