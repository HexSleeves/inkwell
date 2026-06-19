# Plan 015: Design spike — per-author identity, scoped tokens, and write audit

> **Executor instructions**: This is a DESIGN/INVESTIGATE plan. You write ONE markdown deliverable — a draft ADR — plus a status note in `plans/README.md`. You write NO source code and NO migration. Follow the steps in order, run the single Verify command (`pnpm run format:check`), obey the STOP conditions (escalate product/scoping decisions instead of guessing), and update the `plans/README.md` status row when done.
> **Drift check (run first)**: `git diff --stat 6bf6a27..HEAD -- src/api.ts src/db/migrations.ts src/feed.ts docs/audit-v0.1.md` — if any of these in-scope-grounding files changed, compare the Current-state excerpts below to live code; on mismatch, STOP and re-read before drafting (the ADR must describe the code as it actually is).

## Status

- **Priority**: P3
- **Effort**: L
- **Risk**: LOW
- **Depends on**: relates to Plan 004 (fail-closed auth); no hard dependency
- **Category**: direction / security
- **Planned at**: commit `6bf6a27`, 2026-06-19

## Why this matters

Today Inkwell authenticates every write against a single shared `INKWELL_API_KEY` (`src/api.ts`). One leak compromises every document, the only revocation is a redeploy with a new key, and there is no record of who wrote what. The `documents` table has no author/owner column (`src/db/migrations.ts`), so multi-author publishing is structurally impossible and the Atom feed cannot emit `<author>` (`src/feed.ts`). This is the highest-severity finding (S1, High) in `docs/audit-v0.1.md`. Implementing a token+author model touches the auth core, the schema, backfill of existing rows, and several output surfaces — too large and too risky to do without a decision record first. This spike produces that decision record and decomposes the work into safe follow-up plans, so the eventual implementation lands deliberately rather than as one sprawling, breaking change.

## Current state

Files involved in the grounding (the ADR must describe these accurately; you do NOT modify them):

- `src/api.ts` — JSON API handler + auth. Authentication is a single shared-secret comparison.
- `src/db/migrations.ts` — ordered immutable migrations; the `documents` table has no author/owner column.
- `src/feed.ts` — Atom feed builder; entries omit `<author>` because there is no author data.
- `docs/audit-v0.1.md` — prior security audit; S1 recommends a hashed-token table.
- `docs/adr/` — existing ADRs `0001`–`0006`. The new ADR is numbered `0009` per this plan's spec (note the `0007`/`0008` gap — do not renumber existing ADRs; if `0007`/`0008` already exist when you run, STOP and confirm the intended number).

### Excerpt — shared-secret auth (`src/api.ts:293`–`327`, verified at 6bf6a27)

```ts
/**
 * Constant-time comparison of two secrets. Both are SHA-256 hashed first so the
 * comparison runs over fixed-length digests — this avoids leaking the secret's
 * length and keeps `timingSafeEqual` (which throws on length mismatch) safe to
 * call with attacker-controlled input.
 */
function secretsMatch(provided: string, expected: string): boolean {
  const a = createHash('sha256').update(provided).digest();
  const b = createHash('sha256').update(expected).digest();
  return timingSafeEqual(a, b);
}

/**
 * Whether a request carries the valid shared secret in `X-API-Key`. A missing,
 * malformed (array-valued), or non-matching key — or an unconfigured server
 * secret — is unauthenticated. Used both to gate mutations (via
 * {@link requireApiKey}) and to decide whether reads may see drafts.
 */
function isAuthenticated(req: ApiRequest, configuredKey: string | undefined): boolean {
  const header = req.headers?.['x-api-key'];
  // A repeated header arrives as an array; reject the ambiguity outright.
  const provided = typeof header === 'string' ? header : undefined;
  return Boolean(configuredKey && provided && secretsMatch(provided, configuredKey));
}

/**
 * Enforce the shared-secret API key on a mutating request. The client must send
 * the configured secret in the `X-API-Key` header. A missing, malformed, or
 * non-matching key — or an unconfigured server secret — results in a 401.
 */
function requireApiKey(req: ApiRequest, configuredKey: string | undefined): void {
  if (!isAuthenticated(req, configuredKey)) {
    throw new ApiError(401, 'Missing or invalid API key.');
  }
}
```

Key facts the ADR must preserve: comparison is constant-time over fixed-length SHA-256 digests, and auth **fails closed** (unset/empty server key ⇒ all writes 401). `isAuthenticated` is reused to decide whether reads may see draft documents.

### Excerpt — `documents` table, no author column (`src/db/migrations.ts:28`–`64`, verified at 6bf6a27)

```ts
const createDocuments: Migration = {
  id: '0001',
  name: 'create_documents',
  up: `
    CREATE TABLE documents (
      id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
      slug text NOT NULL UNIQUE,
      title text NOT NULL,
      body_markdown text NOT NULL,
      rendered_html text NOT NULL,
      created_at timestamptz NOT NULL DEFAULT now(),
      updated_at timestamptz NOT NULL DEFAULT now()
    );
  `,
  down: `DROP TABLE documents;`,
};
```

Migration `0002` added a `status` column and backfilled existing rows to `published` — this is the existing precedent for **backfilling existing rows** in a migration (`UPDATE documents SET status = 'published';`). The migration system: immutable ordered `{ id, name, up, down }` records, applied in `id` order, recorded in `schema_migrations`; never edit a shipped migration, only add the next zero-padded id; the `MIGRATIONS` array at the end of the file lists them in order.

### Excerpt — feed entries omit `<author>` (`src/feed.ts:88`–`100`, verified at 6bf6a27)

```ts
  const entries = documents
    .map((doc) => {
      const url = `${base}/${encodeURIComponent(doc.slug)}`;
      return `  <entry>
    <title>${escapeXml(doc.title)}</title>
    <id>${escapeXml(url)}</id>
    <link rel="alternate" type="text/html" href="${escapeXml(url)}" />
    <published>${doc.createdAt.toISOString()}</published>
    <updated>${doc.updatedAt.toISOString()}</updated>
    <content type="html">${escapeXml(doc.renderedHtml)}</content>
  </entry>`;
    })
    .join('\n');
```

### Excerpt — audit finding S1 (`docs/audit-v0.1.md:83`, verified at 6bf6a27)

> S1 — High: One key grants _all_ write power to _all_ documents. A leak = full compromise, and the only revocation is redeploying with a new `INKWELL_API_KEY`. No per-author identity, no scopes, no audit of who wrote what. Recommendation: Move to a **token model**: hashed tokens stored in a table, each with an owner, scopes (e.g. publish vs author), and a revoked flag. Keep the shared key as a bootstrap/admin fallback.
>
> S2 — Med: **No write audit log.** Recommendation: Log mutations (key id, action, slug, timestamp) — pairs naturally with S1.

### Exemplar to follow for ADR shape (`docs/adr/0004-http-api.md`)

ADRs in this repo open with `# ADR NNNN: <title>`, then a bullet list `- **Status:** Draft` / `- **Date:** 2026-06-19`, then `## Context`, `## Decision`, and supporting sections. Match this structure and prose style. New ADRs should be **Status: Draft** (this spike does not decide for the team — it proposes).

No tests apply to this plan (design deliverable only).

## Commands you will need

| Purpose      | Command                 | Expected |
| ------------ | ----------------------- | -------- |
| Format check | `pnpm run format:check` | exit 0   |
| Drift check  | `git diff --stat 6bf6a27..HEAD -- src/api.ts src/db/migrations.ts src/feed.ts docs/audit-v0.1.md` | empty / unchanged |
| Status check | `git status`            | only in-scope files modified |

## Scope

**In scope**

- `docs/adr/0009-author-tokens.md` (create) — the design/ADR deliverable.
- `plans/README.md` (create if absent; otherwise edit) — add/update the status row for this plan. The `plans/` directory currently has no `README.md`; if you must create it, give it a short header (`# Plans`) and a markdown table with columns: Plan, Title, Priority, Status. Add the row for Plan 015 marked Done.

**Out of scope** (do NOT touch)

- `src/api.ts` — auth implementation lands in a follow-up plan, not here.
- `src/db/migrations.ts` — the `api_tokens`/`authors`/audit migrations are designed here but NOT written. No new migration in this spike.
- `src/feed.ts` — `<author>` emission is a follow-up plan.
- `docs/audit-v0.1.md` — source material; read-only.
- Any `*.test.ts` — no tests for a design spike.

## Git workflow

- Branch: `advisor/015-spike-author-token-model`.
- Conventional-commit example: `docs(adr): draft 0009 author token + scoped auth model`.
- Do NOT push, commit, or open a PR unless the operator explicitly asks.

## Steps

1. Run the drift check command. If any grounding file changed since `6bf6a27`, re-read it and reconcile the Current-state excerpts before drafting. **Verify**: `git diff --stat 6bf6a27..HEAD -- src/api.ts src/db/migrations.ts src/feed.ts docs/audit-v0.1.md` → empty, or you have reconciled differences.

2. Confirm the ADR number is free: check that `docs/adr/0009-author-tokens.md` does not already exist and note whether `0007`/`0008` exist. **Verify**: `ls docs/adr/` → `0009-author-tokens.md` not present (if it is, STOP).

3. Create `docs/adr/0009-author-tokens.md` with `# ADR 0009: Per-author tokens, scoped auth, and write audit`, `- **Status:** Draft`, `- **Date:** 2026-06-19`, then a `## Context` section summarizing the current single-shared-key state (cite `src/api.ts` constant-time + fail-closed behavior, the author-less `documents` table, the author-less feed, and audit S1/S2). Quote no secret values.

4. Write `## Decision` covering items (a)–(f) below as named subsections:
   - **(a) `api_tokens` table** — columns: `id uuid PK`, `token_hash text NOT NULL UNIQUE` (store ONLY a hash — never plaintext), `owner` (or `author_id` FK — see (b)), `label text`, `scopes text[]` (proposed values: `author`, `publish`, `admin`), `created_at timestamptz`, `revoked_at timestamptz NULL`. Document the proposed lookup flow: client sends opaque token in `X-API-Key`; server hashes it and looks up the row; reject if missing or `revoked_at IS NOT NULL`.
   - **(b) Authorship model** — present the trade-off explicitly: authorship **per-token** (simpler, but a person's identity == their token) vs a separate **`authors` table** with an `author_id` FK on both `api_tokens` and `documents` (clean identity, survives token rotation, but more schema + joins). Recommend a direction with reasoning; flag this as the decision most likely to need product input (see STOP).
   - **(c) Migration + BACKFILL strategy** — describe (do NOT write) the new migration(s): add `api_tokens` (and `authors` if chosen), add an author/owner FK column to `documents`. Existing `documents` rows have no author: backfill them to a default/bootstrap owner (e.g. a seeded `authors` row representing the shared-key admin), exactly as migration `0002` backfilled `status = 'published'`. Specify the next migration ids (e.g. `0007`, `0008`) and that they must be appended to the `MIGRATIONS` array, never editing shipped migrations.
   - **(d) Compat / bootstrap** — keep the shared `INKWELL_API_KEY` as a bootstrap/admin fallback so current deployments do not break: a valid shared key resolves to the bootstrap owner with `admin` scope. Tie to Plan 004 fail-closed semantics: if neither a valid token nor a configured shared key matches, writes still 401. Spell out the deprecation/exit path for the shared key.
   - **(e) Write audit log** — design an append-only audit table (proposed columns: `id`, `token_id`/`actor`, `action` (create/update/delete), `document_slug`, `occurred_at`) recording who/what/when on every mutation. Pairs with the token model (audit S2). Note it is append-only and that failures to write the audit row must not silently drop the audit (decide: fail the write vs best-effort log).
   - **(f) Auth evolution in `src/api.ts`** — describe how `isAuthenticated`/`requireApiKey` evolve to resolve a token → `{ owner, scopes }` while PRESERVING constant-time comparison (hash the provided token, compare digests; do not branch on early mismatch in a way that leaks timing) and fail-closed semantics. Note that `isAuthenticated` is also reused for draft-read visibility, so the resolved principal must still answer "may this request see drafts?". Mention scope enforcement points (publish vs author vs admin) per route.

5. Add a `## Threat & security notes` section: token generation must be high-entropy; storage is hash-only (recommend SHA-256 of a high-entropy random token, or a strong KDF — state the trade-off: KDF resists offline brute force of weak tokens, plain SHA-256 is fine only for high-entropy random tokens); rotation; revocation-without-redeploy (set `revoked_at`); never log or return plaintext tokens (return the plaintext exactly once at creation). This section, and the whole ADR, must contain NO real secret/token values.

6. Add a `## Follow-up implementation plans` section decomposing the work into ordered, independently-shippable plans: (1) schema + token storage migration; (2) auth resolution + scope enforcement in `src/api.ts`; (3) author FK on `documents` + backfill; (4) write audit log; (5) feed `<author>` emission in `src/feed.ts`. For each, one line on scope and dependency order.

7. Add an `## Open questions` section listing unresolved decisions (e.g. per-token vs `authors` table; shared-key deprecation timeline; audit write-failure policy; whether scopes are an enum-checked set or free `text[]`; multi-token-per-author).

8. Update `plans/README.md` (create if absent per Scope) with a row marking Plan 015 Done. **Verify**: `git status` shows only `docs/adr/0009-author-tokens.md` and `plans/README.md` modified/added.

9. Run the format check. **Verify**: `pnpm run format:check` → exit 0. (If Prettier reports the new file unformatted, run `pnpm run format` or hand-fix the markdown, then re-check.)

## Test plan

None — this is a design deliverable, no code lands. Done is defined by ADR content coverage (see Done criteria), not by tests. The only automated gate is `pnpm run format:check`.

## Done criteria

- [ ] `docs/adr/0009-author-tokens.md` exists, Status: Draft, and covers (a) `api_tokens` table, (b) authorship model trade-off + recommendation, (c) migration + backfill strategy, (d) shared-key compat/bootstrap tied to fail-closed, (e) write audit log, (f) `src/api.ts` auth evolution preserving constant-time + fail-closed.
- [ ] ADR includes `## Threat & security notes`, `## Follow-up implementation plans`, and `## Open questions` sections.
- [ ] No real secret or token value appears anywhere in the ADR (grep for an actual key value should find none; reference by name/location only).
- [ ] No source files modified: `git status` lists only `docs/adr/0009-author-tokens.md` and `plans/README.md`. No `src/**` changes, no new migration.
- [ ] `pnpm run format:check` → exit 0.
- [ ] `plans/README.md` row for Plan 015 updated to Done.

## STOP conditions

- If choosing between **per-token authorship** and a **separate `authors` table**, or fixing the **shared-key compat/deprecation boundary**, requires product input you cannot infer — capture the trade-offs in `## Open questions` and STOP; ask the operator rather than silently deciding.
- If `docs/adr/0009-author-tokens.md` already exists, or `0007`/`0008` ADRs already exist that conflict with the intended numbering — STOP and confirm the number.
- If the drift check shows `src/api.ts`, `src/db/migrations.ts`, or `src/feed.ts` changed materially since `6bf6a27` (e.g. auth already moved to tokens, or an author column was added) — STOP; the design premise may be stale.
- If you find yourself editing any `src/**` file or adding a migration — STOP; this spike produces design only.

## Maintenance notes

- The follow-up implementation plans will interact heavily: the auth-resolution plan depends on the token-storage migration; the audit-log plan depends on a resolved principal; the feed `<author>` plan depends on the author FK + backfill. Keep that ordering in the ADR so the tree never breaks mid-rollout.
- A reviewer should scrutinize: (1) that constant-time comparison and fail-closed semantics survive the token lookup (no early-return timing leak, no open-on-misconfig path); (2) that the backfill cannot orphan existing public documents; (3) that plaintext tokens are never persisted or logged and are surfaced exactly once at creation.
- Deferred: rate limiting (audit S3) is related but out of scope for this token model — note it as a separate future concern, not part of this ADR.
