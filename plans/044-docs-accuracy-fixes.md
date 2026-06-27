# Plan 044: Fix documentation drift — wrong revoke method, dead env var, missing endpoints

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `plans/README.md`.
>
> **Drift check (run first)**: `git diff --stat 0819727..HEAD -- docs/COMPATIBILITY.md docs/API.md .env.example src/http/router.rs`
> If any in-scope file changed, compare the "Current state" excerpts before proceeding.

## Status

- **Priority**: P2
- **Effort**: M
- **Risk**: LOW
- **Depends on**: none
- **Category**: docs
- **Planned at**: commit `0819727`, 2026-06-26

## Why this matters

Three documentation defects, in increasing severity:

1. **`.env.example` advertises `OPENAI_API_KEY`** which the code never reads — only `ANTHROPIC_API_KEY` is used. An operator may try to configure OpenAI and silently get nothing.
2. **`docs/COMPATIBILITY.md` lists the wrong token-revoke route.** It claims `DELETE /admin/tokens/:id` is a stable surface; the actual route is `POST /admin/tokens/{prefix}/revoke` (`src/http/router.rs:86`). COMPATIBILITY.md is the *binding* contract — an integrator who codes against it will build the wrong call.
3. **`docs/API.md` documents none of preview tokens, archive, or webmention** (grep counts: 0 each), despite all being live, stable routes. API consumers and MCP tool authors cannot discover them.

## Current state

**`.env.example:72`**:
```
OPENAI_API_KEY=
```
Confirmed: `grep -rn "OPENAI_API_KEY" src/` returns zero matches. The code reads `ANTHROPIC_API_KEY` (see `src/config.rs`).

**`docs/COMPATIBILITY.md:30`**:
```
| Admin | `POST /admin/tokens`, `GET /admin/tokens`, `DELETE /admin/tokens/:id` |
```
Actual admin routes (`src/http/router.rs:84-86`):
```rust
.route("/admin/tokens", any(admin::tokens))
.route("/admin/tokens/prune", any(admin::prune_tokens))
.route("/admin/tokens/{prefix}/revoke", any(admin::revoke_token))
```
So the stable revoke surface is `POST /admin/tokens/{prefix}/revoke`, and `POST /admin/tokens/prune` also exists. `docs/API.md:778` (per earlier audit) documents the revoke route correctly — only COMPATIBILITY.md is wrong.

**Routes missing from `docs/API.md`** (all exist in `src/http/router.rs`, none appear in API.md):
- `POST /documents/{slug}/preview-tokens` (mint), `GET /documents/{slug}/preview-tokens` (list), `DELETE /documents/{slug}/preview-tokens/{prefix}` (revoke)
- `GET /documents/{slug}/preview?token=<pvw_...>` (render draft with token)
- `GET /archive`, `GET /archive/{year}/{month}`, `GET /archive/{year}/{month}/page/{page}`
- `POST /webmention`

## Commands you will need

| Purpose      | Command                                                  | Expected on success |
|--------------|--------------------------------------------------------|---------------------|
| Confirm dead var | `grep -rn "OPENAI_API_KEY" src/`                   | zero matches        |
| Confirm gaps | `grep -c "preview-tokens\|/archive\|webmention" docs/API.md` | 0 (before fix) |
| Markdown sanity | (no build step for docs)                             | n/a                 |

## Scope

**In scope**:
- `.env.example` — remove the `OPENAI_API_KEY` line
- `docs/COMPATIBILITY.md` — fix the admin revoke route, add prune
- `docs/API.md` — add Preview tokens, Archive, and Webmention sections

**Out of scope**:
- `src/` — no code changes
- `docs/openapi.yaml` — keeping the OpenAPI contract in sync is valuable but is a separate, larger task; note it in maintenance notes, do not attempt here unless trivial
- Any env var that IS read by `src/config.rs` — only remove the confirmed-dead `OPENAI_API_KEY`

## Git workflow

- Branch: `advisor/044-docs-accuracy`
- Commit: `docs: fix revoke route, remove dead OPENAI_API_KEY, document preview/archive/webmention`

## Steps

### Step 1: Remove dead OPENAI_API_KEY from .env.example

Read `.env.example` around line 72. Confirm `OPENAI_API_KEY=` is present and that the surrounding comment refers to Anthropic. Remove the `OPENAI_API_KEY=` line (and fix the comment if it wrongly labels the Anthropic key).

First re-confirm it is unused: `grep -rn "OPENAI_API_KEY" src/` → must be zero matches. If it returns ANY match, STOP — the var may be used.

**Verify**: `grep -c "OPENAI_API_KEY" .env.example` → 0

### Step 2: Fix the admin revoke route in COMPATIBILITY.md

In `docs/COMPATIBILITY.md`, change line 30 from:
```
| Admin | `POST /admin/tokens`, `GET /admin/tokens`, `DELETE /admin/tokens/:id` |
```
to:
```
| Admin | `POST /admin/tokens`, `GET /admin/tokens`, `POST /admin/tokens/:prefix/revoke`, `POST /admin/tokens/prune` |
```

**Verify**: `grep -c "DELETE /admin/tokens/:id" docs/COMPATIBILITY.md` → 0; `grep -c "admin/tokens/:prefix/revoke" docs/COMPATIBILITY.md` → 1

### Step 3: Add Preview tokens section to API.md

Read `docs/API.md` to match its existing section formatting (heading level, request/response example style). Add a "Preview tokens" section documenting:
- `POST /documents/{slug}/preview-tokens` — mint a token; auth: admin or owner with `write` scope (x-api-key header); response includes `token` (format `pvw_<prefix>_<secret>`) and `prefix`. Note the token is shown only once.
- `GET /documents/{slug}/preview-tokens` — list tokens for a document (same auth).
- `DELETE /documents/{slug}/preview-tokens/{prefix}` — revoke a token (same auth).
- `GET /documents/{slug}/preview?token=<pvw_...>` — render the draft; no auth header needed (the token IS the credential); any failure returns 401 (never 404) so draft existence is not leaked.

Match the request/response example style used by existing API.md sections. Read `src/http/preview.rs` for the exact response field names if you include a response body example.

**Verify**: `grep -c "preview-tokens" docs/API.md` → ≥ 3

### Step 4: Add Archive section to API.md

Add an "Archive" subsection (these are public HTML routes — match how API.md documents other public HTML routes like `/tags`):
- `GET /archive` — year/month buckets of published documents
- `GET /archive/{year}/{month}` — paginated document list for that month
- `GET /archive/{year}/{month}/page/{page}` — additional pages

**Verify**: `grep -c "/archive" docs/API.md` → ≥ 1

### Step 5: Add Webmention section to API.md

Add a "Webmention" section:
- `POST /webmention` — receive a Webmention; form-encoded `source` + `target`; `target` must be a published note URL on this site; returns 202 Accepted on success, 400 on validation failure, 405 for non-POST. Read `src/http/webmention.rs` module doc (lines 1-16) for the precise contract.

**Verify**: `grep -c "webmention" docs/API.md` → ≥ 1

## Test plan

No code tests — documentation only. Verification is the `grep` assertions above. After editing, read each changed section once to confirm it is coherent and matches the live route signatures in `src/http/router.rs`.

## Done criteria

- [ ] `grep -c "OPENAI_API_KEY" .env.example` → 0
- [ ] `grep -c "DELETE /admin/tokens/:id" docs/COMPATIBILITY.md` → 0
- [ ] `docs/API.md` documents preview-tokens (≥3 mentions), /archive (≥1), webmention (≥1)
- [ ] Every newly documented route's method + path matches `src/http/router.rs` exactly
- [ ] No `src/` files modified
- [ ] `plans/README.md` status row updated

## STOP conditions

- `grep -rn "OPENAI_API_KEY" src/` returns a match (the var IS used) — do not remove it; report.
- `docs/API.md` turns out to already document one of these route families (drift since the audit). Skip that family and note it.
- A route's signature in `src/http/router.rs` differs from what is described here (drift). Document the actual signature and report the discrepancy.

## Maintenance notes

- `docs/openapi.yaml` should eventually gain these endpoints too — flag a follow-up to sync the OpenAPI contract with API.md.
- COMPATIBILITY.md is the binding contract; when a route changes, update COMPATIBILITY.md, API.md, and openapi.yaml together. Consider a CI check that greps router.rs route literals against COMPATIBILITY.md to catch this class of drift automatically (a possible future DX plan).
