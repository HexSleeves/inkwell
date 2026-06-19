# Plan 006: Add a committed .env.example template

> **Executor instructions**: Follow the steps in order, top to bottom. Run every **Verify** command and confirm the expected result before moving on. Obey all STOP conditions. This plan creates a non-code template file and edits one doc; do NOT add any real secret value. When done, update the status row in `plans/README.md` (create that file from the template in Step 4 if it does not yet exist).
> **Drift check (run first)**: `git diff --stat 6bf6a27..HEAD -- .gitignore src/db/pool.ts src/main.ts src/server.ts src/api.ts src/site-url.ts src/feed.ts README.md` — if any in-scope file changed, compare the Current-state excerpts below to live code; on mismatch, STOP. NOTE: this plan was authored against working-tree HEAD `10ee86c` (the literal `6bf6a27` in the command may not resolve); if `git rev-parse 6bf6a27` fails, instead run `git diff --stat HEAD -- <paths>` to confirm the working tree is clean, then verify the excerpts below by reading the live files directly.

## Status

- **Priority**: P2
- **Effort**: S
- **Risk**: LOW
- **Depends on**: none
- **Category**: dx
- **Planned at**: commit `6bf6a27`, 2026-06-19 (authored against live HEAD `10ee86c`)

## Why this matters

`.gitignore` deliberately whitelists `!.env.example` so a template can be committed, but no such file exists. New contributors cloning the repo get no single, authoritative list of which environment variables Inkwell reads, which are required, and what safe placeholder values look like. The env knowledge is currently scattered across source files and a README table that is itself incomplete (it omits `INKWELL_SITE_URL`). A committed `.env.example` plus a `cp .env.example .env` pointer in the README closes that gap with zero runtime risk and no secrets.

## Current state

Files and their role:

- `.gitignore` — ignores `.env` / `.env.*` but whitelists `.env.example`. Verified excerpt (lines 15-18):
  ```
  # Env & secrets
  .env
  .env.*
  !.env.example
  ```
- `src/db/pool.ts` — reads `DATABASE_URL`; required, throws if unset. Verified excerpt (line 36):
  ```ts
  const url = connectionString ?? process.env.DATABASE_URL;
  ```
  (line 38 throws "No Postgres connection string: pass one to createPool() or s..." when neither is provided.)
- `src/main.ts` — reads `PORT` (default `3000` via `parsePort`, see line 25 `if (raw === undefined || raw === '') return 3000;`) and `HOST` (default `0.0.0.0`). Verified excerpt (lines 34-35):
  ```ts
  const port = parsePort(process.env.PORT);
  const host = process.env.HOST ?? '0.0.0.0';
  ```
- `src/server.ts` — reads `INKWELL_SITE_URL` (line 151) and `INKWELL_API_KEY` (line 203). Verified excerpt:
  ```ts
  const siteUrl = process.env.INKWELL_SITE_URL;   // line 151
  const apiKey = process.env.INKWELL_API_KEY;     // line 203
  ```
- `src/api.ts` — documents `INKWELL_API_KEY` as the write-auth secret; when `undefined` or empty, no key can match so all writes fail closed (doc comment near line 73).
- `src/site-url.ts` — `INKWELL_SITE_URL` is the absolute-URL origin; when unset it falls back to a localhost default. Verified excerpt (line 15):
  ```ts
  export const DEFAULT_SITE_URL = 'http://localhost';
  ```
- `src/feed.ts` — also reads `INKWELL_SITE_URL`, falling back to the same localhost default (doc comment near line 51).
- `README.md` — has an "## Environment variables" table at line 63 and a "## Run Inkwell" section starting at line 90. Verified excerpt of the env table (lines 63-68) — note it does NOT list `INKWELL_SITE_URL`:
  ```
  | Variable          | Required | Default   | Used by                | Description ... |
  | `DATABASE_URL`    | yes      | —         | server, `db:*` scripts | ... |
  | `PORT`            | no       | `3000`    | server                 | ... |
  | `HOST`            | no       | `0.0.0.0` | server                 | ... |
  | `INKWELL_API_KEY` | no       | —         | server                 | ... Unset locks down all writes. |
  ```
  The "## Run Inkwell" code block (lines 95-108) installs, builds, exports `DATABASE_URL`, migrates, and starts:
  ```bash
  # 1. Install dependencies and compile to ./dist
  pnpm install
  pnpm run build

  # 2. Point Inkwell at your database
  export DATABASE_URL=postgres://user:pass@localhost:5432/inkwell

  # 3. Create the schema
  pnpm run db:migrate

  # 4. Start the server (defaults to http://0.0.0.0:3000)
  pnpm start
  ```

Verified absent at authoring time: `.env.example` (does not exist), `plans/README.md` (does not exist).

Resolved env-var truth table (the values to template):

| Var | Required? | App default | Read in |
| --- | --- | --- | --- |
| `DATABASE_URL` | yes (throws if unset) | none | `src/db/pool.ts` |
| `PORT` | no | `3000` | `src/main.ts` |
| `HOST` | no | `0.0.0.0` | `src/main.ts` |
| `INKWELL_API_KEY` | no (fail closed when unset → all writes locked) | none | `src/server.ts`, `src/api.ts` |
| `INKWELL_SITE_URL` | no | `http://localhost` | `src/server.ts`, `src/site-url.ts`, `src/feed.ts` |

Exemplar/test pattern: there is no automated test for this plan (it touches no source). Formatting is enforced by Prettier via `pnpm run format:check`; the new files must satisfy it. Markdown/text formatting follows the existing README style.

## Commands you will need

| Purpose | Command | Expected |
| --- | --- | --- |
| Format check | `pnpm run format:check` | exit 0 |
| Format (if check fails) | `pnpm run format` | rewrites files; re-run format:check |
| Git status | `git status --short` | shows only in-scope files |

## Scope

**In scope:**

- `.env.example` (create) — the committed env template at repo root.
- `README.md` — add a one-line `cp .env.example .env` pointer in the "## Run Inkwell" section; add the missing `INKWELL_SITE_URL` row to the env table.
- `plans/README.md` (create if absent, else edit) — status tracking row for this plan.

**Out of scope:**

- Real `.env` — never create or commit it; it is git-ignored and would risk leaking secrets.
- `src/**` — no source change is needed; behavior is unchanged.
- `.gitignore` — already correctly whitelists `.env.example`; do not touch.

## Git workflow

- Branch: `advisor/006-add-env-example`.
- Conventional-commit example: `docs(dx): add committed .env.example template and README pointer`.
- Do NOT push, commit to a shared branch, or open a PR unless the operator explicitly asks.

## Steps

1. **Create `.env.example`** at the repo root with one commented entry per variable and a NON-SECRET placeholder for each. Use exactly this content:

   ```
   # Postgres connection string (REQUIRED). Startup fails loudly if unset.
   DATABASE_URL=postgres://inkwell:inkwell@localhost:5432/inkwell

   # HTTP server bind (optional). Defaults shown.
   PORT=3000
   HOST=0.0.0.0

   # Shared secret for write auth, sent as the X-API-Key header.
   # Generate with: openssl rand -hex 32
   # Leave UNSET/empty to keep all writes locked (the server fails closed).
   INKWELL_API_KEY=

   # Public origin for canonical / OpenGraph / sitemap / feed URLs (optional).
   # Defaults to http://localhost when unset.
   INKWELL_SITE_URL=http://localhost
   ```

   Do NOT put a real key after `INKWELL_API_KEY=` — it must stay empty.

   **Verify**: `test -f .env.example && echo OK` → prints `OK`.

2. **Confirm the template is tracked, not ignored.** The `!.env.example` whitelist must take effect.

   **Verify**: `git check-ignore .env.example; echo "exit=$?"` → prints `exit=1` (i.e. NOT ignored; `git check-ignore` exits 1 when the path is not ignored and prints nothing).

3. **Edit `README.md`:**

   a. In the "## Environment variables" table (around line 63), add a row for `INKWELL_SITE_URL` after the `INKWELL_API_KEY` row. Match the existing column layout exactly; suggested cells: Variable `` `INKWELL_SITE_URL` ``, Required `no`, Default `` `http://localhost` ``, Used by `server, feed, sitemap`, Description: public origin used for canonical / OpenGraph / sitemap / feed URLs.

   b. In the "## Run Inkwell" section, add a one-line pointer to the template near the top of the run flow (e.g. just before or as part of step "Point Inkwell at your database"), such as:
   ```
   # Copy the template and edit it, or export vars directly:
   cp .env.example .env
   ```
   Keep it a single added line of guidance plus the command; do not restructure the existing block.

   **Verify**: `grep -q 'cp .env.example .env' README.md && grep -q 'INKWELL_SITE_URL' README.md && echo OK` → prints `OK`.

4. **Update `plans/README.md` status tracking.** If the file does not exist, create it with this minimal table, then mark this plan done. If it already exists, just update/add the row for plan 006.

   Minimal template if creating:
   ```markdown
   # Implementation plans

   | Plan | Title | Priority | Status |
   | --- | --- | --- | --- |
   | 006 | Add a committed .env.example template | P2 | Done |
   ```
   If it exists, set plan 006's Status cell to `Done` (add the row if missing).

   **Verify**: `grep -q '006' plans/README.md && echo OK` → prints `OK`.

5. **Run formatting check** over the whole repo.

   **Verify**: `pnpm run format:check` → exit 0. If it fails on the new/edited files, run `pnpm run format`, re-run `pnpm run format:check`, and confirm exit 0.

## Test plan

No automated tests — this plan touches no source and adds no runtime behavior. Manual verification only:

- `.env.example` exists at repo root (Step 1 Verify).
- `.env.example` is tracked, not git-ignored, confirming the `!.env.example` whitelist works (Step 2 Verify) — this is the regression check for the exact finding (whitelist present but no file).
- README contains both the `cp .env.example .env` pointer and an `INKWELL_SITE_URL` reference (Step 3 Verify).
- `pnpm run format:check` passes (Step 5 Verify).

No existing test file to model after (none applies).

## Done criteria

- [ ] `.env.example` exists at repo root with all five vars and only non-secret placeholders; `INKWELL_API_KEY=` is empty.
- [ ] `git check-ignore .env.example` exits 1 (file is tracked, not ignored).
- [ ] `README.md` has a `cp .env.example .env` pointer in the Run section and an `INKWELL_SITE_URL` row in the env table.
- [ ] `pnpm run format:check` exits 0.
- [ ] No out-of-scope files modified: `git status --short` lists only `.env.example`, `README.md`, and `plans/README.md` (plus this plan file if newly added).
- [ ] `plans/README.md` row for plan 006 shows status `Done`.

## STOP conditions

- STOP if making the example "work" appears to require a real/working secret value (a real API key, a real DB password for a live DB). It must not — placeholders only. The `INKWELL_API_KEY` line stays empty.
- STOP if `git check-ignore .env.example` exits 0 (the file is being ignored): the `.gitignore` whitelist is not behaving as the Current-state excerpt describes — re-run the drift check and reconcile before proceeding.
- STOP if the drift check shows `.gitignore`, `src/main.ts`, `src/db/pool.ts`, `src/server.ts`, or `src/site-url.ts` changed such that the env-var names, defaults, or required-ness differ from the truth table above; update the template to match reality first.

## Maintenance notes

- Keep `.env.example` in sync with the README env-vars table and with `process.env.*` reads in source. Future env vars must be added in three places: source, README table, and `.env.example`.
- A reviewer should scrutinize that `INKWELL_API_KEY` carries no value and that no real connection string/credential leaked into the template.
- Deferred follow-up (out of scope here): the README env table omitting `INKWELL_SITE_URL` was a pre-existing gap; this plan adds the row, but a broader doc pass could also reconcile the `feed.ts` duplicated site-url default with `src/site-url.ts`.
