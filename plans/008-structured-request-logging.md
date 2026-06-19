# Plan 008: Add minimal structured request logging + request id

> **Executor instructions**: Follow the steps in order; the tree must compile and pass tests after every step. Run every **Verify** command and do not proceed past a failing one. Obey every STOP condition. When done, update the status row for plan 008 in `plans/README.md` (create that file if it does not yet exist — see Step 5).
> **Drift check (run first)**: `git diff --stat 6bf6a27..HEAD -- src/server.ts src/main.ts src/api.test.ts` — if any in-scope file changed, compare the Current-state excerpts below to the live code; on mismatch, STOP and report the difference rather than guessing.

## Status
- **Priority**: P2
- **Effort**: M
- **Risk**: LOW
- **Depends on**: none
- **Category**: dx
- **Planned at**: commit `6bf6a27`, 2026-06-19

## Why this matters
Today the only logs Inkwell emits are the boot/shutdown `console.log` lines in `src/main.ts`. The transport's top-level `catch` in `src/server.ts` returns a 413 or a generic 500 to the client but logs **nothing** — a production error is invisible and impossible to correlate with a client report. Adding one structured JSON log line per request plus a per-request `x-request-id` (echoed in the response header and the log) makes 5xx failures observable and lets an operator tie a client-visible 500 to the exact server-side error. It stays dependency-free (this repo prizes minimal deps), and it becomes the seam a future observability stack (OTel, tracing) hooks into.

## Current state
Files and their role:
- `src/server.ts` — the `node:http` transport adapter. It is the only place that touches Node HTTP types: reads the body, normalizes the request, delegates to `handleApiRequest`/page/feed/sitemap/search handlers, serializes responses, and owns the top-level `try/catch`.
- `src/main.ts` — the runnable entrypoint. Wires a real `pg` pool to `createServer(db)` and listens; logs only on listen + shutdown.
- `src/api.test.ts` — the exemplar test file. Its final `describe` block (`documents HTTP API (node:http transport)`) binds a real `node:http` server to an ephemeral port and exercises it with `fetch`. **Model the new server test after this block.**

The `catch` block today logs nothing (`src/server.ts`, lines 210–218 at HEAD `6bf6a27`):

```ts
    } catch (error) {
      const tooLarge = error instanceof Error && error.message === 'Request body too large.';
      writeResponse(res, {
        status: tooLarge ? 413 : 500,
        body: {
          error: { message: tooLarge ? 'Request body too large.' : 'Internal server error.' },
        },
      });
    }
```

The request listener signature and its `try` opening (`src/server.ts`, lines 143–148):

```ts
export function createRequestListener(db: Queryable) {
  return async (req: IncomingMessage, res: ServerResponse): Promise<void> => {
    try {
      const segments = splitPath(req.url ?? '/');
      const method = (req.method ?? 'GET').toUpperCase();
```

Note: every success path inside the listener `return`s after calling one of the `write*` helpers (`writeXmlResponse`, `writeStringResponse`, `writeHtmlResponse`, `writeResponse`), and the `catch` calls `writeResponse`. There is no single shared exit point today.

`src/main.ts` logs only on listen + shutdown (lines 42–54):

```ts
  server.listen(port, host, () => {
    console.log(`Inkwell listening on http://${host}:${port}`);
  });

  // Drain connections cleanly on termination so deploys don't drop requests.
  const shutdown = (signal: string): void => {
    console.log(`Received ${signal}, shutting down.`);
    server.close(() => {
      void pool.end().finally(() => process.exit(0));
    });
  };
```

The server test harness to copy (`src/api.test.ts`, lines 641–667 at HEAD `6bf6a27`):

```ts
describe('documents HTTP API (node:http transport)', () => {
  let server: Server;
  let baseUrl: string;
  let previousApiKey: string | undefined;

  beforeEach(async () => {
    // The transport adapter reads the secret from the environment.
    previousApiKey = process.env.INKWELL_API_KEY;
    process.env.INKWELL_API_KEY = API_KEY;
    const db = createMemoryDatabase().db;
    await migrate(db);
    server = createServer(db);
    await new Promise<void>((resolve) => server.listen(0, '127.0.0.1', resolve));
    const { port } = server.address() as AddressInfo;
    baseUrl = `http://127.0.0.1:${port}`;
  });

  afterEach(async () => {
    if (previousApiKey === undefined) {
      delete process.env.INKWELL_API_KEY;
    } else {
      process.env.INKWELL_API_KEY = previousApiKey;
    }
    await new Promise<void>((resolve, reject) =>
      server.close((err) => (err ? reject(err) : resolve())),
    );
  });
```

Existing imports at the top of `src/api.test.ts` (lines 11–19) already include: `afterEach, beforeEach, describe, expect, it` from `vitest`; `createServer` from `./server.js`; `migrate` from `./db/migrate.js`; `createMemoryDatabase` from `./db/test-helpers.js`. `Server` and `AddressInfo` are imported from `node:http`/`node:net` near the top (lines 12–13). The constant `API_KEY` is defined in the file.

### Project conventions you MUST match
- ESM imports use explicit `.js` extensions (e.g. `import { randomUUID } from 'node:crypto'` is a built-in so no extension; local imports like `from './log.js'` DO need `.js`).
- `verbatimModuleSyntax` is on: type-only imports use `import type { ... }`.
- TS is strict with `noUncheckedIndexedAccess`, `exactOptionalPropertyTypes`. An optional field set to `undefined` is NOT the same as omitting it — build the log object so error-only fields are simply absent on success rather than `: undefined`.
- Tests use Vitest, run against in-memory Postgres via `createMemoryDatabase()` from `src/db/test-helpers.ts` — NO live DB.
- Commit style: Conventional Commits with scope (e.g. `feat(server): ...`).

## Commands you will need
| Purpose | Command | Expected |
| --- | --- | --- |
| Install | `pnpm install` | exit 0 |
| Typecheck | `pnpm run typecheck` | exit 0, no errors |
| Lint | `pnpm run lint` | exit 0 |
| Format check | `pnpm run format:check` | exit 0 |
| Tests (one file) | `pnpm test src/api.test.ts` | pass |
| Tests (log unit) | `pnpm test src/log.test.ts` | pass (if you create it) |
| Tests (all) | `pnpm test` | all pass (164+ currently) |
| Build | `pnpm run build` | exit 0 |
| Full gate | `pnpm run ci` | exit 0 |

## Scope
**In scope** (modify):
- `src/server.ts` — generate a `requestId`, emit one structured JSON log line per request, set the `x-request-id` response header, and log the caught error in the `catch`.
- `src/log.ts` *(create)* — a tiny dependency-free logger: a function that takes a structured record and writes one `console.log(JSON.stringify(...))` line. Keeping it in its own module makes it unit-testable and gives the future observability stack one seam to replace.
- `src/api.test.ts` — add cases to the existing `node:http transport` describe block (request-id header on success; generic message + request-id on a forced 500).
- `src/log.test.ts` *(create, optional but recommended)* — a pure unit test for the logger record shape (asserts no secret/body fields, correct level mapping).
- `plans/README.md` *(create if absent)* — status tracking row (Step 5).

**Out of scope** (do NOT touch):
- `src/api.ts` business logic — routing/validation lives there; logging is a transport concern, keep it in `server.ts`.
- `src/main.ts` boot/shutdown logs — may *optionally* adopt the logger, but keep this plan tight; prefer leaving it unchanged.
- `src/pages.ts`, `src/feed.ts`, `src/sitemap.ts`, `src/search.ts`, `src/rendering.ts` — handlers, not the transport.
- The `db/*` modules and migrations — no schema or persistence change here.

## Git workflow
- Branch: `advisor/008-structured-request-logging`.
- Conventional-commit example: `feat(server): structured per-request JSON logging with request id`.
- Do NOT push, commit, or open a PR unless the operator explicitly asks. Leave the work on the branch.

## Steps

1. **Create the logger module `src/log.ts`.**
   Define a `LogLevel` type and a `RequestLog` record type, and an `emit`/`logRequest` function that writes exactly one JSON line via `console.log(JSON.stringify(record))`. Required fields: `ts` (ISO string, `new Date().toISOString()`), `level` (`'info' | 'error'`), `method`, `path`, `status` (number), `durationMs` (number), `requestId` (string). Error-only field: `err` (string message) — present ONLY on the error path (omit the key entirely on success; do not set it to `undefined`, because of `exactOptionalPropertyTypes`). Do NOT include any header values, body, or the API key. Keep it dependency-free. Sketch:

   ```ts
   /**
    * Minimal dependency-free structured logger.
    *
    * Emits one JSON line per request to stdout. This is the single seam a future
    * observability stack (OTel, tracing) would replace; intentionally tiny.
    * NEVER log request/response bodies or the X-API-Key header.
    */
   export type LogLevel = 'info' | 'error';

   export interface RequestLog {
     ts: string;
     level: LogLevel;
     method: string;
     path: string;
     status: number;
     durationMs: number;
     requestId: string;
     err?: string;
   }

   export function logRequest(record: RequestLog): void {
     // eslint-disable-next-line no-console -- structured stdout logging is the point
     console.log(JSON.stringify(record));
   }
   ```
   (Check whether ESLint already permits `console` — `src/main.ts` calls `console.log` with no disable comment, so the `no-console` rule is likely off. If `pnpm run lint` passes without the disable comment, REMOVE it to avoid an "unused eslint-disable" error.)
   **Verify**: `pnpm run typecheck` -> exit 0.

2. **Wire request-id + logging into `src/server.ts`.**
   - Add `import { randomUUID } from 'node:crypto';` and `import { logRequest, type LogLevel } from './log.js';` (match the existing import grouping/style at the top of the file).
   - Inside the closure returned by `createRequestListener`, at the very start of the `async (req, res) => {`, before the `try`, capture `const requestId = randomUUID();` and `const startedAt = Date.now();`, compute `const path = (req.url ?? '/').split('?')[0] ?? '/';` and `const method = (req.method ?? 'GET').toUpperCase();` (the `method` currently computed inside the `try` can stay; just make sure the value used for logging is available in both the try and catch — declare it in the outer scope and assign once).
   - Set the response header on every path. The cleanest approach: set it once up front with `res.setHeader('x-request-id', requestId);` immediately after computing `requestId` (before any `write*` call, since `writeHead` flushes headers). Setting a header before `writeHead` is preserved by `res.writeHead`.
   - Replace the `catch` block so it logs at level `'error'` and still returns the same generic client message. Capture the error message for the log but DO NOT put it in the client body.
   - After both the success delegation and the catch, emit exactly one `logRequest` call. Because there are many early `return`s in the success paths, the simplest correct structure is: wrap the body so the status is captured. Recommended refactor — track the response status in an outer `let status = 500;` variable, have each `write*` call site still run as-is, and instead of logging at each return, log in a `finally` block. Use the actual HTTP status: read it from `res.statusCode` inside `finally` (Node sets `res.statusCode` when `writeHead` is called), which avoids threading a status through every branch. Concretely:

   ```ts
   export function createRequestListener(db: Queryable) {
     return async (req: IncomingMessage, res: ServerResponse): Promise<void> => {
       const requestId = randomUUID();
       const startedAt = Date.now();
       const method = (req.method ?? 'GET').toUpperCase();
       const path = (req.url ?? '/').split('?')[0] ?? '/';
       res.setHeader('x-request-id', requestId);
       let level: LogLevel = 'info';
       let errMessage: string | undefined;
       try {
         const segments = splitPath(req.url ?? '/');
         // ... existing routing/delegation unchanged, but reuse the `method`
         //     computed above instead of re-declaring it ...
       } catch (error) {
         const tooLarge = error instanceof Error && error.message === 'Request body too large.';
         if (!tooLarge) {
           level = 'error';
           errMessage = error instanceof Error ? error.message : String(error);
         }
         writeResponse(res, {
           status: tooLarge ? 413 : 500,
           body: {
             error: { message: tooLarge ? 'Request body too large.' : 'Internal server error.' },
           },
         });
       } finally {
         logRequest({
           ts: new Date().toISOString(),
           level,
           method,
           path,
           status: res.statusCode,
           durationMs: Date.now() - startedAt,
           requestId,
           ...(errMessage !== undefined ? { err: errMessage } : {}),
         });
       }
     };
   }
   ```
   Notes for the executor:
   - There is currently a `const method = ...` declared INSIDE the `try` (line 147). After moving `method` to the outer scope, REMOVE the inner re-declaration to avoid a shadow/redeclare error.
   - Keep all existing routing branches and `write*` calls byte-for-byte the same except for the removed inner `method` declaration.
   - The conditional-spread (`...(errMessage !== undefined ? { err } : {})`) is required by `exactOptionalPropertyTypes` so `err` is omitted (not `undefined`) on success.
   - 413 (body-too-large) is treated as a client error, so it stays `level: 'info'`. Document this choice in a one-line comment. (Logging policy for this plan: log EVERY request at `info`, and `error` for uncaught 5xx. If volume is a concern later, narrowing to `>= 400` is a one-line change — note it in Maintenance.)
   **Verify**: `pnpm run typecheck` -> exit 0; then `pnpm run lint` -> exit 0.

3. **Add server tests to `src/api.test.ts`.**
   In the existing `describe('documents HTTP API (node:http transport)', ...)` block (starts line 641), add:
   - **Case A — request-id on success**: `const res = await fetch(`${baseUrl}/documents/ghost`);` (a normal 404 page/handler response). Assert `res.headers.get('x-request-id')` is a non-empty string (e.g. `expect(res.headers.get('x-request-id')).toMatch(/[0-9a-f-]{36}/)`).
   - **Case B — forced 500 still returns the generic message AND the request id**: force the transport's `catch` by injecting a `db` whose `query` throws. The cleanest way without a live DB: create a server with a stub `Queryable` whose `query()` rejects, e.g. inside the test build `const failing = { query: () => Promise.reject(new Error('boom')) } as unknown as Queryable;` then `createServer(failing)`, listen on an ephemeral port (mirror the harness in lines 651–655), and `fetch` a `GET /documents` (an API path that hits `handleApiRequest` and queries the DB). Assert `res.status === 500`, the JSON body's `error.message === 'Internal server error.'` (generic — no leak of `'boom'`), and `res.headers.get('x-request-id')` is present. Close that server in the test.
     - If `GET /documents` does not reach a DB query, pick an API route that does (consult `src/api.ts` for which path issues a query); the goal is to drive the `catch` with a thrown error. Alternatively, capture `console.log` via `vi.spyOn(console, 'log')` and assert one JSON line with `level: 'error'` and `err` containing `'boom'` — but the header + generic-body assertion is the primary check.
   - Import `Queryable` if needed: `import type { Queryable } from './db/pool.js';` is already imported (line 19).
   **Verify**: `pnpm test src/api.test.ts` -> pass.

4. **(Optional) Add `src/log.test.ts`** unit test if you created the logger as a separate function: spy on `console.log`, call `logRequest({...})`, assert exactly one call, that the parsed JSON has the expected keys, that there is NO `apiKey`/`authorization`/`body` key, and that omitting `err` yields no `err` key. Model the spy pattern after standard Vitest (`vi.spyOn(console, 'log').mockImplementation(() => {})`, then `.mockRestore()`).
   **Verify**: `pnpm test src/log.test.ts` -> pass.

5. **Update `plans/README.md` status row.**
   The directory `plans/` currently contains no `README.md`. If it is still absent, create `plans/README.md` with a status table and a row for this plan. If a `README.md` already exists (another plan created it), just add/flip the row for plan 008. Minimal table:

   ```md
   # Inkwell Improvement Plans

   | Plan | Title | Priority | Status |
   | --- | --- | --- | --- |
   | 008 | Structured request logging + request id | P2 | Done |
   ```
   **Verify**: `git status --porcelain plans/README.md` shows the file staged/modified.

6. **Full gate.**
   **Verify**: `pnpm run ci` -> exit 0.

## Test plan
- **New tests in `src/api.test.ts`** (inside the `node:http transport` describe): (A) a normal response includes a non-empty `x-request-id` header; (B) **the regression for this exact finding** — a forced uncaught error returns HTTP 500 with body `error.message === 'Internal server error.'` (no internal detail leaked) AND an `x-request-id` header, proving a client-visible 500 is correlatable.
- **Optional `src/log.test.ts`**: logger record shape; asserts no secret/body fields; `err` omitted on success.
- **Model after**: the `documents HTTP API (node:http transport)` block in `src/api.test.ts` (lines 641–667) for the bind-and-fetch harness.
- **Verification commands**: `pnpm test`, `pnpm run typecheck`, `pnpm run lint`, then `pnpm run ci`.

## Done criteria
- [ ] `src/log.ts` created, dependency-free (no new entries in `package.json`).
- [ ] `src/server.ts`: every request sets an `x-request-id` response header and emits one structured JSON log line; the `catch` logs the error at `level: 'error'` while the client body stays generic.
- [ ] New regression test (forced 500 -> generic body + request-id header) passes.
- [ ] `pnpm run typecheck` -> exit 0.
- [ ] `pnpm run lint` -> exit 0.
- [ ] `pnpm run format:check` -> exit 0 (run `pnpm exec prettier --write` on touched files if needed).
- [ ] `pnpm test` -> all pass.
- [ ] `pnpm run ci` -> exit 0.
- [ ] `package.json`/`pnpm-lock.yaml` unchanged (no new dependency).
- [ ] No out-of-scope files modified (`git status` shows only `src/server.ts`, `src/log.ts`, `src/log.test.ts`, `src/api.test.ts`, `plans/README.md`, and this plan file).
- [ ] `plans/README.md` row for plan 008 updated to Done.

## STOP conditions
- STOP if implementing this requires adding ANY new dependency (pino, winston, etc.). The design is `console.log(JSON.stringify(...))` only — staying on Node built-ins (`node:crypto`) and `console`. A new dep means the plan is being done wrong.
- STOP if the drift check shows `src/server.ts` or `src/api.test.ts` changed since `6bf6a27` and the live code no longer matches the excerpts above — re-derive the edits against the live code or report the mismatch.
- STOP if a log line would ever contain the API key, an `authorization`/`x-api-key` header value, or a request/response body. Re-check the `RequestLog` fields and the `catch` error capture before proceeding.
- STOP if setting the `x-request-id` header throws "Cannot set headers after they are sent" — that means a `write*` ran before `setHeader`; move `res.setHeader('x-request-id', ...)` to before the `try`/any write, as specified.

## Maintenance notes
- This logger is the seam a future observability stack (OpenTelemetry, distributed tracing, log shipping) hooks into — keep it a single small module so it can be swapped without touching the transport routing.
- A reviewer must scrutinize that NO secret or body data is ever logged: check the `RequestLog` field list and the `catch` error-message capture (an error thrown from JSON parsing or the DAL should not embed request body content).
- Logging policy chosen here: log every request at `info`, 5xx at `error`. If log volume becomes a problem, narrowing to `status >= 400` is a one-line guard around `logRequest`; document the change if made.
- Deferred follow-up (not this plan): adopt the same logger for boot/shutdown lines in `src/main.ts`, and consider honoring an inbound `x-request-id` header (trust boundary — only accept it from a trusted proxy) instead of always generating a fresh UUID.
- `res.statusCode` is read in `finally` to capture the real status without threading it through every branch; if a future change starts writing responses without going through `writeHead`/the `write*` helpers, re-verify the logged status is still accurate.
