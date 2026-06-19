# Plan 011: Document the non-transactional list/count behavior of GET /documents

> **Executor instructions**: Follow the steps in order. This is a LOW-value, documentation-and-pinning-test plan — do NOT add transaction machinery. Run every **Verify** command and only proceed when it matches the expected result. Obey every STOP condition. When finished, update the status row for Plan 011 in `plans/README.md` (create that file per Step 0 if it does not yet exist).
> **Drift check (run first)**: `git diff --stat 6bf6a27..HEAD -- src/api.ts src/api.test.ts` — if either in-scope file changed since `6bf6a27`, compare the Current-state excerpts below to the live code. If the `handleList` body or the `GET /documents` list tests differ from what is shown here, STOP and report the drift instead of editing.

## Status
- **Priority**: P3
- **Effort**: S
- **Risk**: LOW
- **Depends on**: none
- **Category**: bug
- **Planned at**: commit `6bf6a27`, 2026-06-19

## Why this matters
`GET /documents` returns a page of documents plus an unpaged `total` for the same filter. Those two values come from two separate, non-transactional queries fired with `Promise.all`, so under concurrent writes the `total` can momentarily disagree with the page contents (e.g. a doc inserted between the two reads). The impact is benign — pagination already tolerates minor skew — but the behavior is undocumented, so a future reader could mistake it for a bug or "fix" it with disproportionate transaction plumbing. This plan records the accepted trade-off in a code comment and pins the no-concurrency invariant (`total === number of matching docs`) with a regression test, so the decision is explicit and protected.

## Current state
Files and their roles:
- `src/api.ts` — JSON API handler + auth. Contains `handleList`, which builds the paginated list envelope.
- `src/api.test.ts` — co-located Vitest suite for the API handler. Uses an in-memory Postgres (`createMemoryDatabase()` + `migrate(db)`) and a `call(...)` helper that wraps `handleApiRequest`.

The relevant `handleList` body, `src/api.ts:383-396` (verified at HEAD `6bf6a27`):

```ts
async function handleList(
  db: Queryable,
  req: ApiRequest,
  configuredKey: string | undefined,
): Promise<ApiResponse> {
  const status = resolveListStatus(req, configuredKey);
  const { limit, offset } = parsePagination(req.query);
  const filter: StatusFilter = status ? { status } : {};
  const [documents, total] = await Promise.all([
    listDocuments(db, { ...filter, limit, offset }),
    countDocuments(db, filter),
  ]);
  return { status: 200, body: { documents, total, limit, offset } };
}
```

`listDocuments` and `countDocuments` are imported from the data-access layer at the top of `src/api.ts` (around lines 24-26). Both accept a `Queryable` (a pool OR a client) and a `StatusFilter`.

Test harness pattern to follow, `src/api.test.ts:35-54` (verified at HEAD `6bf6a27`):

```ts
const API_KEY = 'test-secret-key';

describe('documents HTTP API (handler)', () => {
  let db: Queryable;

  beforeEach(async () => {
    db = createMemoryDatabase().db;
    await migrate(db);
  });

  const call = (req: ApiRequest): Promise<ApiResponse> =>
    handleApiRequest(db, { headers: { 'x-api-key': API_KEY }, ...req }, { apiKey: API_KEY });

  const createSample = (overrides: Record<string, unknown> = {}): Promise<ApiResponse> =>
    call({
      method: 'POST',
      segments: ['documents'],
      body: { title: 'Hello World', bodyMarkdown: '# Hi', ...overrides },
    });
```

Exemplar list test to model the new test after, `src/api.test.ts:184-194` (verified at HEAD `6bf6a27`):

```ts
  describe('GET /documents', () => {
    it('lists documents in a paginated envelope', async () => {
      await createSample({ slug: 'a' });
      await createSample({ slug: 'b' });
      const res = await call({ method: 'GET', segments: ['documents'] });
      expect(res.status).toBe(200);
      const page = res.body as ListBody;
      expect(page.documents.map((d) => d.slug).sort()).toEqual(['a', 'b']);
      expect(page.total).toBe(2);
      expect(page.limit).toBe(20);
      expect(page.offset).toBe(0);
    });
```

The `ListBody` type and `createSample` helper already exist in `src/api.test.ts`; reuse them.

Note: `pg-mem` (used by `createMemoryDatabase()`) is single-threaded and serializes operations, so you CANNOT meaningfully simulate a real concurrent write in the test. The regression test therefore pins the *no-concurrency* invariant only (which is exactly the behavior the comment documents as guaranteed). Do not attempt to fabricate concurrency.

## Commands you will need
| Purpose | Command | Expected |
| --- | --- | --- |
| Typecheck | `pnpm run typecheck` | exit 0, no errors |
| Lint | `pnpm run lint` | exit 0 |
| Format check | `pnpm run format:check` | exit 0 |
| Tests (one file) | `pnpm test src/api.test.ts` | pass |
| Tests (all) | `pnpm test` | all pass |

## Scope
**In scope:**
- `src/api.ts` — add an explanatory comment inside / above `handleList` documenting the intentional non-transactional two-read behavior. No logic change.
- `src/api.test.ts` — add one regression test pinning the no-concurrency invariant (`total` equals the number of matching docs).

**Out of scope:**
- `src/db/documents.ts` and all of `src/db/*` — no transaction/client plumbing; that is the escalation path (see STOP conditions), not this plan.
- `src/server.ts`, `src/pages.ts`, `src/feed.ts`, `src/sitemap.ts` — unrelated to the list/count read path.
- The actual `listDocuments` / `countDocuments` query SQL — leave untouched.

## Git workflow
- Branch: `advisor/011-document-list-count-consistency`.
- Conventional-commit example: `docs(api): document non-transactional list/count behavior and pin invariant`.
- Do NOT push, commit, or open a PR unless the operator explicitly asks. Leave the work on the branch.

## Steps

1. **Create or update `plans/README.md` status tracking.** If `plans/README.md` does not exist, create it with a table that has columns `| Plan | Title | Priority | Status |` and a row for this plan with status `In progress`. If it already exists, add (or update) the Plan 011 row. Leave the status as `In progress` until the final step.
   **Verify**: `test -f plans/README.md && grep -q '011' plans/README.md && echo OK` -> prints `OK`.

2. **Add the explanatory comment in `handleList`.** In `src/api.ts`, immediately above the `const [documents, total] = await Promise.all([` line (currently `src/api.ts:391`), insert a comment that states: the page and the unpaged total are read in two separate, non-transactional queries; under concurrent writes the `total` may momentarily disagree with the page contents; this skew is acceptable for pagination and is intentionally NOT wrapped in a transaction (a real snapshot would require a checked-out client + BEGIN/COMMIT, which the pool-based wiring does not exercise). Keep it concise (2-4 lines). Do not change any code, only add the comment. Match the existing comment style in the file (`//` for inline, `/** */` for doc blocks; an inline `//` block is appropriate here).
   **Verify**: `pnpm run typecheck` -> exit 0, no errors.

3. **Confirm formatting and lint of the comment.**
   **Verify**: `pnpm run format:check` -> exit 0; then `pnpm run lint` -> exit 0. If `format:check` fails, run `pnpm run format` (or `pnpm exec prettier --write src/api.ts`) and re-verify.

4. **Add the regression test in `src/api.test.ts`.** Inside the existing `describe('GET /documents', ...)` block (currently opening at `src/api.test.ts:183`), add one `it(...)` that seeds a known number of documents with no concurrent writes and asserts `page.total` equals exactly the number of matching documents and that `page.documents.length` is consistent with the page window. Model it on the `'lists documents in a paginated envelope'` test (`src/api.test.ts:184-194`). Reuse the existing `createSample`, `call`, and `ListBody`. Suggested name: `'reports a total equal to the matching-document count with no concurrent writes'`. Example body:
   ```ts
   it('reports a total equal to the matching-document count with no concurrent writes', async () => {
     await createSample({ slug: 'one' });
     await createSample({ slug: 'two' });
     await createSample({ slug: 'three' });
     const res = await call({ method: 'GET', segments: ['documents'] });
     expect(res.status).toBe(200);
     const page = res.body as ListBody;
     // No concurrent writes: the unpaged total must match the page contents exactly.
     expect(page.total).toBe(3);
     expect(page.documents).toHaveLength(3);
   });
   ```
   **Verify**: `pnpm test src/api.test.ts` -> pass (new test included).

5. **Run the full suite to confirm nothing regressed.**
   **Verify**: `pnpm test` -> all pass (164+ tests).

6. **Flip the `plans/README.md` Plan 011 status row to `Done`.**
   **Verify**: `grep -q '011' plans/README.md && echo OK` -> prints `OK`; manually confirm the status cell reads `Done`.

## Test plan
- **New test**: in `src/api.test.ts`, inside `describe('GET /documents', ...)`, add `'reports a total equal to the matching-document count with no concurrent writes'` (the exact regression for this finding — pinning that under no concurrency the `total` equals the page/matching count). Model it after the existing `'lists documents in a paginated envelope'` test at `src/api.test.ts:184-194`.
- **Existing tests to keep green**: the whole `GET /documents` / `pagination` group (`src/api.test.ts:183-253`) already asserts `total` semantics across limit/offset; they must continue to pass unchanged.
- **Verification commands**: `pnpm test src/api.test.ts` then `pnpm run typecheck`.

## Done criteria
- [ ] `git diff --stat 6bf6a27..HEAD -- src/api.ts src/api.test.ts` drift check performed; excerpts matched (or STOP triggered).
- [ ] `handleList` in `src/api.ts` has a clear comment explaining the intentional non-transactional two-read behavior; no logic changed.
- [ ] New regression test added in `src/api.test.ts` and passing.
- [ ] `pnpm run typecheck` exits 0.
- [ ] `pnpm run lint` exits 0.
- [ ] `pnpm run format:check` exits 0.
- [ ] `pnpm test` — all pass (164+).
- [ ] No out-of-scope files modified (`git status` shows only `src/api.ts`, `src/api.test.ts`, and `plans/README.md`).
- [ ] `plans/README.md` Plan 011 row updated to `Done`.

## STOP conditions
- **STOP if the maintainer wants true snapshot consistency.** That is the escalation path, not this plan. A real transaction needs a checked-out client (`client.connect()` / `BEGIN` ... `COMMIT`) threaded through the data-access layer (`listDocuments` / `countDocuments` both run on one client), which the current pool-based wiring and `pg-mem` tests do not exercise. Do not start that work here — confirm scope with the operator first and open a separate plan.
- **STOP if the drift check fails** — if the live `handleList` body or the `GET /documents` tests differ from the excerpts above, report the mismatch rather than editing blindly.
- **STOP if you find yourself editing any `src/db/*` file** — that means you have crossed into the escalation/transaction path, which is explicitly out of scope.
- **STOP if a test you did not add starts failing** — investigate and report; do not paper over it by relaxing assertions.

## Maintenance notes
- This plan deliberately documents rather than fixes. If the list endpoint ever needs exact snapshot totals (for example to support cursor-based pagination or strict count invariants under load), revisit it: wrap both reads in a single READ-ONLY / REPEATABLE READ transaction on one checked-out client, and add real concurrency tests against a live Postgres (pg-mem cannot model concurrent writes).
- A reviewer should scrutinize that (a) no production logic in `handleList` changed — only a comment was added — and (b) the new test pins the no-concurrency invariant and does not falsely claim to test concurrency.
- Deferred follow-up: the transaction escalation is intentionally not done; track it only if a maintainer requests snapshot consistency.
