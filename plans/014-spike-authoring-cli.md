# Plan 014: Design spike — first-class authoring CLI

> **Executor instructions**: This is a DESIGN/INVESTIGATE spike. The only deliverable is a draft ADR (a doc), plus one note in `plans/README.md`. Do NOT write any CLI code, do NOT touch `src/`, and do NOT add a `bin` to `package.json`. Follow the steps in order, run the single Verify command, obey the STOP condition, and when done update the `plans/README.md` status row for this plan. The ADR is a *draft* (`Status: proposed`) — you are recording trade-offs and a recommended direction, not shipping a CLI.
> **Drift check (run first)**: `git diff --stat 6bf6a27..HEAD -- README.md src/index.ts src/api.ts docs/adr` — if any in-scope reference file changed, compare the Current-state excerpts below to live code; on mismatch, STOP and report.

## Status
- **Priority**: P2
- **Effort**: M
- **Risk**: LOW
- **Depends on**: none (synergizes with Plan 010 if present)
- **Category**: direction
- **Planned at**: commit `6bf6a27`, 2026-06-19

## Why this matters
Inkwell's README promises an "API-first" platform "so Inkwell can back a CLI, a CMS UI, or someone else's tooling" (README ~line 17), but no CLI exists: every authoring example in getting-started is raw `curl` (README ~line 113+). A prior audit ranks authoring UX as the #1 adoption barrier — asking authors to hand-craft JSON and `curl` commands is the single biggest friction point for a Markdown publishing tool. The service surface is already cleanly exported from `src/index.ts` and exercised by tests, so a CLI is a thin, low-risk command layer over a tested API (the adjacent possible). This spike de-risks that work by deciding packaging, command set, and the one genuinely open design question — how a Markdown file's front-matter maps to `title`/`slug`/`status` — *before* anyone writes implementation code.

## Current state

**Reference files (read-only inputs for the ADR):**
- `README.md` — value prop + getting-started. The "API-first" promise and the all-`curl` authoring flow are the grounding for this spike.
- `src/index.ts` — public package exports; shows the service surface a CLI would sit on.
- `src/api.ts` — the HTTP request handler + route table the CLI would call (or, alternatively, import directly).
- `docs/adr/000{1..6}-*.md` — existing ADRs; **0006** is the format/length exemplar to match. Highest existing ADR number is **0006**; there is no 0007. The spec assigns this ADR **0008** (leaves 0007 free for an in-flight ADR); keep the assigned `0008` filename.

**README "Why Inkwell" excerpt (the promise this spike addresses), README ~line 17:**
```
- **API-first.** Every capability is reachable over a documented HTTP API, so
  Inkwell can back a CLI, a CMS UI, or someone else's tooling.
```

**README getting-started authoring excerpt (raw curl, no human authoring path), README ~line 113+:**
```bash
curl -sS -X POST http://localhost:3000/documents \
  -H 'content-type: application/json' \
  -H "x-api-key: $INKWELL_API_KEY" \
  -d '{"title":"Hello World","bodyMarkdown":"# Hello World\n\nMy first **Inkwell** page."}'
```
```bash
curl -sS -X POST http://localhost:3000/documents/hello-world/publish \
  -H "x-api-key: $INKWELL_API_KEY"
```

**`src/index.ts` (full) — the exported service surface, HEAD 6bf6a27:**
```ts
export const NAME = 'inkwell';
export const VERSION = '0.1.0';

export { renderMarkdown, renderDocumentHtml } from './rendering.js';
export { handleApiRequest, ApiError, type ApiRequest, type ApiResponse } from './api.js';
export { createServer, createRequestListener } from './server.js';
export {
  handlePageRequest,
  renderIndexPage,
  renderDocumentPage,
  renderNotFoundPage,
  escapeHtml,
  type PageRequest,
  type PageResponse,
} from './pages.js';
export { slugify } from './slug.js';
```

**`src/api.ts` route table (the verbs a CLI maps to), src/api.ts ~line 499:**
```
 *   - `GET    /documents`                 -> list (published-only unless authed)
 *   - `POST   /documents`                 -> create (defaults to draft)
 *   - `GET    /documents/:slug`           -> fetch (draft 404s unless authed)
 *   - `PATCH  /documents/:slug`           -> update (PUT is accepted as an alias)
 *   - `DELETE /documents/:slug`           -> delete
 *   - `POST   /documents/:slug/publish`   -> mark published (idempotent)
 *   - `POST   /documents/:slug/unpublish` -> mark draft (idempotent)
```
Auth facts to honor in the ADR (from `src/api.ts` ~line 514): mutating routes (`POST`/`PATCH`/`PUT`/`DELETE`, publish/unpublish) require the shared secret via the `X-API-Key` header, read from `INKWELL_API_KEY`. Reads are open but only show `published` documents unless authed. Create defaults to `draft`. Update body is `{ title?, bodyMarkdown?, tags? }`. Slug is derived server-side (or accepted on create as an optional `slug`).

**Exemplar/format to follow:** match `docs/adr/0006-content-discovery-and-seo.md` — a short `# 000N — Title`, `Status:` line, `## Context`, `## Decision` (with sub-sections), and trailing `## Consequences` / follow-up notes. Keep prose-first; this is a design doc, not code.

**CLI design rubric (from the `create-cli` skill / clig.dev) to cite for section (e):** primary output to stdout, diagnostics/errors to stderr; exit `0` success, `1` generic failure, `2` invalid usage; offer `--json` for machine output; never pass secrets via flags (use env/stdin/`--*-file`); config precedence high→low: flags > env > project config > user config > system; respect `NO_COLOR`/`--no-color` and disable color/animation when stdout isn't a TTY; `-h/--help` and `--version` standard; destructive ops (delete) need confirmation interactively and `--force`/`--yes` non-interactively; use network timeouts.

## Commands you will need
| Purpose | Command | Expected |
| --- | --- | --- |
| Format check | `pnpm run format:check` | exit 0 |
| Status (scope guard) | `git status --porcelain` | only `docs/adr/0008-authoring-cli.md` + `plans/README.md` |

(No typecheck/lint/test needed — this spike adds only Markdown. Run `pnpm install` first only if `pnpm` is not yet bootstrapped.)

## Scope
**In scope (create/edit only these):**
- `docs/adr/0008-authoring-cli.md` *(create)* — the draft ADR.
- `plans/README.md` *(edit, or create if absent)* — add/update this plan's status row. NOTE: there is no `plans/` directory at HEAD 6bf6a27 except for plan files; if `plans/README.md` does not exist, create a minimal one with a status table (columns: Plan, Title, Priority, Status) and a row for 014.

**Out of scope (do NOT touch):**
- `src/**` — no CLI code in this spike (implementation is deferred to the follow-up plans the ADR proposes).
- `package.json` — no `bin` field, no new dependency, no `commander`/`yargs` install. Recommending a parser lib in the ADR is fine; adding it is not.
- `README.md` — do not rewrite getting-started to use the CLI yet; that lands with the implementation.
- Existing ADRs `0001`–`0006` — read-only exemplars.

## Git workflow
- Branch: `advisor/014-spike-authoring-cli`.
- Conventional-commit example: `docs(adr): draft authoring CLI design (0008)`.
- Do NOT push, commit, or open a PR unless the operator explicitly asks.

## Steps

1. **Run the drift check** (command in the blockquote above). If any in-scope reference file changed since `6bf6a27`, diff it against the excerpts in Current state; on any mismatch, STOP and report what drifted.

2. **Create `docs/adr/0008-authoring-cli.md`** matching the `0006` ADR format. Header: `# 0008 — Authoring CLI` and `Status: proposed`. The body MUST cover all of the following (these are the Done gates):

   - **`## Context`** — restate the gap: API-first promise vs all-`curl` authoring; cite the `src/index.ts` exported surface and the `src/api.ts` verb table as the foundation a CLI sits on; note authoring UX is the #1 adoption barrier.

   - **`## Decision`** with these sub-sections:

     - **(a) Packaging** — decide and justify: a `bin` entry in *this* package vs a separate `@inkwell/cli` package. Recommend one with a clear rationale (e.g. monorepo-free repo today → a `bin` in-package is the lowest-friction start; a separate package decouples release cadence and dependency surface but adds publishing overhead). If you cannot make this call without maintainer input, see the STOP condition — capture the trade-offs in a small table and ask.

     - **(b) Command set** — define `inkwell create | update | publish | unpublish | list | delete`, each reading local `.md` file(s), and map each to the corresponding route in the `src/api.ts` table (create→`POST /documents`, update→`PATCH /documents/:slug`, publish→`POST /documents/:slug/publish`, unpublish→`.../unpublish`, list→`GET /documents`, delete→`DELETE /documents/:slug`). Include a **command-surface spec** with USAGE synopsis and an args/flags table per the `create-cli` skill template (global flags `-h/--help`, `--version`, `--json`, `--base-url`, `--quiet`/`--verbose`; per-command flags like `--status`, `--slug`, `--force`/`--yes` on delete).

     - **(c) Front-matter mapping — the key open question** — specify how a Markdown file's YAML (and optionally TOML) front-matter maps to `title`/`slug`/`status`. State the rule: front-matter keys (`title`, `slug`, `status`) populate the API fields; body below the front-matter becomes `bodyMarkdown`; precedence of CLI flags over front-matter; what happens when `title` is absent (derive from first `# H1`? require it?); how `status` in front-matter interacts with the explicit `publish`/`unpublish` verbs. **Also decide HTTP-API client vs library import**: recommend calling the HTTP API (keeps the CLI a thin client that honors `X-API-Key` and works against any running Inkwell instance) over importing the library directly (which couples the CLI to a co-located DB/process); justify.

     - **(d) Config** — base URL + API key resolution. Key comes from `INKWELL_API_KEY` (env) or a `--*-file`/stdin; **NEVER accept the key via a plain flag and NEVER echo or log its value** (note this explicitly). Base URL from `--base-url` flag → `INKWELL_BASE_URL` env → default `http://localhost:3000`. State precedence: flags > env > project config > user config. Note any dotfile config is an open question (see Open Questions).

     - **(e) Output / exit-code conventions** — primary output to stdout (created/updated document JSON or a human summary), diagnostics/errors to stderr; `--json` for machine output; exit `0` success, `1` runtime/API failure, `2` invalid usage; respect `NO_COLOR`/non-TTY (no color/spinners); network timeout on API calls. Reference the `create-cli`/clig.dev rubric.

   - **`## Follow-up plans`** — decompose implementation into 2–3 tracked plans, e.g.: **(1)** CLI scaffold + `create`/`publish` (arg parsing, config resolution, API client, the two highest-value verbs); **(2)** remaining verbs (`update`/`unpublish`/`list`/`delete`, including delete confirmation/`--force`); **(3)** front-matter parsing + the test suite (YAML/TOML parse, mapping precedence, error cases). Give each a one-line scope and note their suggested plan numbers.

   - **`## Open questions`** — at minimum: front-matter format (YAML-only vs YAML+TOML), dotfile/project config (`.inkwellrc` / `inkwell.config` — needed at all for v1?), and multi-file / bulk publish (glob support, partial-failure semantics, transactional vs best-effort).

   **Verify**: `pnpm run format:check` -> exit 0 (Prettier accepts the new Markdown).

3. **Update `plans/README.md`** — add or update the status row for Plan 014 (mark it the deliverable status your repo uses, e.g. `Done` / `Delivered`, once the ADR is written). If the file does not exist, create a minimal index table and add the 014 row.

   **Verify**: `pnpm run format:check` -> exit 0.

4. **Scope guard** — confirm only the two in-scope files changed.

   **Verify**: `git status --porcelain` -> shows only `docs/adr/0008-authoring-cli.md` and `plans/README.md` (and nothing under `src/` or `package.json`).

## Test plan
None — this is a design spike that adds only Markdown. There is no code to test. The "test" is the Done checklist below plus `pnpm run format:check`. (The ADR's follow-up plan (3) is where the real CLI test suite will be specified, modeled on `src/api.test.ts`'s `createMemoryDatabase()` + `handleApiRequest` pattern.)

## Done criteria
- [ ] `docs/adr/0008-authoring-cli.md` exists, `Status: proposed`, format matches `docs/adr/0006-*.md`.
- [ ] ADR covers all of (a) packaging, (b) command set + USAGE spec, (c) front-matter mapping + HTTP-vs-library decision, (d) config (with the explicit "never echo the key" note), (e) output/exit-code conventions.
- [ ] ADR includes a `## Follow-up plans` section with 2–3 decomposed plans.
- [ ] ADR includes an `## Open questions` section covering front-matter format, dotfile config, and multi-file/bulk publish.
- [ ] No secret VALUE appears anywhere in the ADR (only `INKWELL_API_KEY` referenced by name).
- [ ] `pnpm run format:check` exits 0.
- [ ] No out-of-scope files modified: `git status --porcelain` shows only `docs/adr/0008-authoring-cli.md` and `plans/README.md`.
- [ ] `plans/README.md` status row for Plan 014 updated/added.

## STOP conditions
- **Packaging needs a maintainer call**: if you cannot confidently choose between an in-package `bin` and a separate `@inkwell/cli` package (because it depends on release/versioning policy you don't have), do NOT guess and bury it — write the trade-off table in section (a), mark the decision `OPEN — needs maintainer input`, set the ADR `Status: proposed`, and report the open decision in your summary.
- **Drift**: any in-scope reference file changed since `6bf6a27` and the live code no longer matches the Current-state excerpts (especially the `src/api.ts` route table or `src/index.ts` exports) — STOP; the command-set mapping must reflect real routes.
- **Scope breach**: if completing the ADR seems to require editing `src/` or `package.json`, you've crossed from design into implementation — STOP; that belongs in the follow-up plans, not this spike.

## Maintenance notes
- This ADR is the contract the three follow-up implementation plans inherit; a reviewer should scrutinize the **front-matter mapping (c)** hardest — it's the one decision that's expensive to change once authors have `.md` files on disk, and it determines whether `slug`/`status`/`title` are author-controlled or CLI-controlled.
- The **HTTP-client vs library-import** choice (c) interacts with packaging (a): a separate package strongly implies the HTTP-client approach; an in-package `bin` could plausibly do either. Keep them consistent.
- When the CLI lands, the README getting-started (~line 113+) should be rewritten to show CLI authoring alongside (or instead of) the raw `curl` flow — note this as a doc follow-up in the ADR, but do not do it here.
- Deferred: shell-completion generation, bulk/glob publish semantics, and dotfile config are intentionally left to Open Questions; don't expand the spike to resolve them unless the maintainer asks.
