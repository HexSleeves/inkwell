# Plan 004: Remove the default API key from docker-compose so it fails closed

> **Executor instructions**: Follow the steps in order, top to bottom. This is a tiny, inspection-only change — do not touch app code. Run every **Verify** command exactly as written and confirm the expected result before moving on. Obey every STOP condition. When finished, update the status row for Plan 004 in `plans/README.md` (create that file if it does not yet exist — see Step 4).
>
> **Drift check (run first)**: `git diff --stat 6bf6a27..HEAD -- docker-compose.yml src/api.ts README.md` — if any in-scope file changed, compare the Current-state excerpts below to the live code; on mismatch, STOP and re-derive the edit against the live file. (At planning time the advisor read HEAD `6bf6a27`; the working HEAD may have advanced — the excerpts are the source of truth for the exact lines you must change.)

## Status

- **Priority**: P1
- **Effort**: S
- **Risk**: LOW
- **Depends on**: none
- **Category**: security
- **Planned at**: commit `6bf6a27`, 2026-06-19

## Why this matters

`docker-compose.yml` ships a publicly-known default value (`changeme-local-dev-key`) for `INKWELL_API_KEY`. The Inkwell server is deliberately designed to **fail closed**: when no key is configured, every mutating request returns `401`. The compose default silently defeats that protection — anyone who runs the published compose file without overriding the key gets full write access (create/update/delete/publish documents) using the well-known default value. Making the variable **required** (compose refuses to start without an explicit key) restores the fail-closed property and turns a silent insecure default into a loud, actionable startup error.

## Current state

Files and roles:

- `docker-compose.yml` — local/self-host stack definition (the `app` service plus a Postgres `db` service). Contains the insecure default. **This is the file you edit.**
- `src/api.ts` — JSON API handler + auth gate. **Read-only context** for this plan; it establishes the fail-closed contract the compose default undermines. Do not edit.
- `README.md` — project docs. **Read-only context**; its quickstart does NOT use docker compose (it uses `pnpm start`), and its env-var table already documents `INKWELL_API_KEY` correctly as fail-closed when unset. No README edit is expected (see Scope / STOP conditions).

Verified excerpts at HEAD `6bf6a27`:

`docker-compose.yml` header comment (lines ~1-7):

```yaml
# Local/self-host stack: the Inkwell app plus its Postgres database.
#
#   docker compose up --build
#
# Override the non-secret defaults below via a local `.env` file (gitignored)
# or your shell. At minimum set INKWELL_API_KEY to a real secret before
# exposing this beyond localhost.
```

`docker-compose.yml` app `environment:` block (lines ~33-39):

```yaml
    environment:
      # `db` is the compose service hostname. Matches createPool()'s DATABASE_URL.
      DATABASE_URL: postgres://${POSTGRES_USER:-inkwell}:${POSTGRES_PASSWORD:-inkwell}@db:5432/${POSTGRES_DB:-inkwell}
      PORT: '3000'
      HOST: 0.0.0.0
      # Shared secret required on mutating requests (X-API-Key). Override in .env.
      INKWELL_API_KEY: ${INKWELL_API_KEY:-changeme-local-dev-key}
```

The fail-closed contract this protects, `src/api.ts` `isAuthenticated` (lines ~311-316):

```ts
function isAuthenticated(req: ApiRequest, configuredKey: string | undefined): boolean {
  const header = req.headers?.['x-api-key'];
  // A repeated header arrives as an array; reject the ambiguity outright.
  const provided = typeof header === 'string' ? header : undefined;
  return Boolean(configuredKey && provided && secretsMatch(provided, configuredKey));
}
```

With no `configuredKey`, `isAuthenticated` is always `false`, so `requireApiKey` rejects every mutation with `401`. The compose default supplies a `configuredKey`, defeating this.

README env-var table row (line ~68) already documents the correct behavior — no change needed:

```
| `INKWELL_API_KEY` | no       | —         | server                 | Shared secret required on mutating requests via the `X-API-Key` header (see below). Unset locks down all writes. |
```

Pattern to follow: this is a config/inspection change. There is **no application test** for compose. Verification is by `grep`/`docker compose config`, described below. (For context on how the repo tests auth in code, see `src/api.test.ts`, but you will not add or change any test here.)

## Commands you will need

| Purpose | Command | Expected |
| --- | --- | --- |
| Drift check | `git diff --stat 6bf6a27..HEAD -- docker-compose.yml src/api.ts README.md` | empty, or review per drift-check instructions |
| Confirm no insecure default remains | `grep -n "changeme" docker-compose.yml` | exit 1 (no matches) |
| Confirm required-var form is present | `grep -n 'INKWELL_API_KEY:.*:?' docker-compose.yml` | one match |
| Show no out-of-scope edits | `git status --porcelain` | only `docker-compose.yml` (and `plans/...`) listed |
| Optional: compose validates & errors when unset | `INKWELL_API_KEY= docker compose config` | non-zero exit, prints the required-var error message (only if Docker is installed; NOT required) |
| Optional: compose validates when set | `INKWELL_API_KEY=test docker compose config` | exit 0 (only if Docker is installed; NOT required) |

(No `pnpm`/typecheck/test step is needed — this plan touches no TypeScript. App tests are unaffected, but you may run `pnpm test` once at the end if you want a sanity check; it is not required to pass criteria for this plan beyond "still green".)

## Scope

**In scope** (edit):

- `docker-compose.yml` — change the `INKWELL_API_KEY` env entry from a defaulted value to a required variable, and update the adjacent comment.
- `plans/README.md` *(create if missing)* — add/update the Plan 004 status row (see Step 4).

**Out of scope** (do NOT touch):

- `src/api.ts` — the auth logic is correct; it is only context.
- `README.md` — its quickstart uses `pnpm start`, not compose, and its env-var docs already describe fail-closed behavior. Only touch it if the STOP condition below actually fires.
- `Dockerfile` — image build; does not set the key.
- `.env` / `.env.example` — committing an example env file is a separate concern (Plan 006). Do not create or edit one here.

## Git workflow

- Branch: `advisor/004-compose-fail-closed-api-key` (create from the current default branch).
- Conventional-commit example:
  - `fix(compose): require INKWELL_API_KEY so the stack fails closed`
- Do **not** push, open a PR, or merge unless the operator explicitly asks. Commit locally only.

## Steps

1. **Create the working branch.**
   - `git checkout -b advisor/004-compose-fail-closed-api-key`
   - **Verify**: `git branch --show-current` -> `advisor/004-compose-fail-closed-api-key`

2. **Replace the defaulted env value with a required-variable form, and update the adjacent comment.** In `docker-compose.yml`, change these two lines:

   ```yaml
      # Shared secret required on mutating requests (X-API-Key). Override in .env.
      INKWELL_API_KEY: ${INKWELL_API_KEY:-changeme-local-dev-key}
   ```

   to:

   ```yaml
      # Shared secret required on mutating requests (X-API-Key). REQUIRED: compose
      # refuses to start if unset, so the server keeps its fail-closed default.
      # Set it in your .env, e.g. `INKWELL_API_KEY=$(openssl rand -hex 32)`.
      INKWELL_API_KEY: ${INKWELL_API_KEY:?Set INKWELL_API_KEY (e.g. openssl rand -hex 32) in your .env before starting}
   ```

   Notes:
   - Preserve the existing indentation (the env keys are indented 6 spaces under `environment:`). Match the surrounding lines exactly.
   - The `${VAR:?message}` form makes `docker compose` error out with `message` when `VAR` is empty or unset. Do NOT invent or hard-code any real secret value anywhere.
   - **Verify**: `grep -n "changeme" docker-compose.yml` -> exit 1 (no matches).
   - **Verify**: `grep -n 'INKWELL_API_KEY:.*:?' docker-compose.yml` -> exactly one match showing the `:?` form.

3. **(Optional, only if Docker is available) Confirm compose now fails closed.** If `docker` is installed and usable in this environment:
   - **Verify**: `INKWELL_API_KEY= docker compose config` -> non-zero exit, and the output contains the message `Set INKWELL_API_KEY`.
   - **Verify**: `INKWELL_API_KEY=test docker compose config` -> exit 0.
   - If Docker is NOT available, skip this step — it is not required to complete the plan. The `grep` verifications in Step 2 are sufficient.

4. **Update the plans index.** If `plans/README.md` does not exist, create it with a simple status table; otherwise add/update the Plan 004 row. Minimal acceptable content if creating fresh:

   ```markdown
   # Plans

   | Plan | Title | Priority | Status |
   | ---- | ----- | -------- | ------ |
   | 004 | Remove the default API key from docker-compose so it fails closed | P1 | Done |
   ```

   If the file already exists, only add or flip the Plan 004 row to `Done` and match the existing table's column style.
   - **Verify**: `grep -n "004" plans/README.md` -> shows the Plan 004 row marked Done.

5. **Confirm only in-scope files changed.**
   - **Verify**: `git status --porcelain` -> lists only `docker-compose.yml` and `plans/README.md` (modified/added). Nothing under `src/` or `Dockerfile` or `README.md`.

6. **Commit locally (do not push).**
   - `git add docker-compose.yml plans/README.md`
   - `git commit -m "fix(compose): require INKWELL_API_KEY so the stack fails closed"`
   - **Verify**: `git log --oneline -1` -> shows the commit; `git status --porcelain` -> empty.

## Test plan

No application/unit test is added — this change is to a Docker Compose config file, which the Vitest suite does not exercise, and pg-mem-based tests are unrelated. The regression for this exact finding is enforced by inspection:

- **Regression assertion**: `grep -n "changeme" docker-compose.yml` returns no matches, and `grep -n 'INKWELL_API_KEY:.*:?' docker-compose.yml` returns the required `:?` form. Together these prove the well-known default is gone and the variable is now required.
- **Optional behavioral check** (Docker only): `INKWELL_API_KEY= docker compose config` exits non-zero with the guard message; `INKWELL_API_KEY=test docker compose config` exits 0.

There is no existing test to model after for compose; the closest auth-behavior test is `src/api.test.ts` (unauthenticated writes -> 401), which already covers the in-code fail-closed contract and should remain green untouched. Run `pnpm test` once if you want a sanity check that nothing regressed.

## Done criteria

- [ ] `git diff` shows only the two `docker-compose.yml` lines changed (comment + env value) as specified.
- [ ] `grep -n "changeme" docker-compose.yml` -> exit 1 (no matches).
- [ ] `grep -n 'INKWELL_API_KEY:.*:?' docker-compose.yml` -> exactly one match (required-variable form).
- [ ] No real secret value appears anywhere in the diff.
- [ ] No out-of-scope files modified — `git status --porcelain` lists only `docker-compose.yml` and `plans/README.md`.
- [ ] `plans/README.md` Plan 004 row updated to Done (file created if it did not exist).
- [ ] (If Docker present) `INKWELL_API_KEY= docker compose config` errors; `INKWELL_API_KEY=test docker compose config` succeeds.
- [ ] Local commit made on branch `advisor/004-compose-fail-closed-api-key`; not pushed.

## STOP conditions

- **A documented quickstart relies on the default key.** Before committing, re-scan the repo for the old default and any "just run compose, it works out of the box" instruction: `grep -rni "changeme-local-dev-key\|changeme" .` and check README's compose/`.env` references. At planning time the README quickstart used `pnpm start` (not compose) and did NOT depend on the default, so no doc change was needed. If the live repo now has docs that tell users to run `docker compose up` and assume writes work without setting a key, you MUST update that doc in the same change (add the `INKWELL_API_KEY=$(openssl rand -hex 32)` step) — do not just remove the default and leave a broken quickstart.
- **Drift mismatch.** If the drift check shows `docker-compose.yml` already changed and the `INKWELL_API_KEY` line no longer matches the excerpt (e.g. someone already removed the default or restructured the env block), STOP and reconcile against the live file rather than blindly applying the edit.
- **You are tempted to invent a secret value.** Never. The `${VAR:?...}` form is intentional — it has no default and errors loudly. If something seems to "need" a concrete key to start, that is the desired fail-closed behavior, not a bug to paper over.

## Maintenance notes

- Pairs with **Plan 015** (per-author token model): once per-author tokens exist, the single shared-key bootstrap story changes and this required-variable may be superseded or supplemented. Revisit then.
- Pairs with **Plan 006** (committed `.env.example`): that plan should add `INKWELL_API_KEY` with a placeholder plus a generation hint (`openssl rand -hex 32`) — **never** a real value. Keep the guidance consistent between the compose `:?` message and the example env file.
- A reviewer should scrutinize: (1) that the `:?` message is helpful and matches the README/`.env.example` guidance; (2) that no real secret leaked into the diff or git history; (3) that the change did not accidentally make any *non-secret* default (POSTGRES_*, PORT, HOST) required too — only `INKWELL_API_KEY` should switch to the required form.
- Deferred follow-up: consider applying the same fail-closed audit to any future deployment manifests (k8s, Helm, systemd units) so the well-known default does not reappear elsewhere.
