---
name: debug-request-failures
description: Diagnose failures at the HTTP, auth, DB, and garden/AI layers. The single debug pattern for inkwell's request pipeline.
triggers:
  - "debug"
  - "error"
  - "500"
  - "401"
  - "404"
  - "403"
  - "failing"
  - "broken"
  - "not working"
  - "request fails"
edges:
  - target: context/architecture.md
    condition: when tracing the failure through the system layers
  - target: context/conventions.md
    condition: when the fix involves adding error handling or changing a pattern
  - target: patterns/add-endpoint.md
    condition: when the bug is in a handler
last_updated: 2026-06-23
---

# Debug Request Failures

## Context

The inkwell request pipeline has four distinct failure layers. Each has a different error type and log pattern. Identify which layer failed first.

```
HTTP (Axum routing) → Auth (bearer token check) → Validation (body/params) → DB (SQLx) → Garden/AI (best-effort)
```

## Layer Diagnosis

### Layer 1 — HTTP / Routing (405 Method Not Allowed, 404 Route Not Found)

**Symptom:** `405` even though the route exists, or `{"error":{"message":"Method not allowed."}}`.

**Check:**
- Route uses `any(handler)` in `router.rs` but handler method-dispatches internally. Missing method in the handler's `match method { }` block → `AppError::MethodNotAllowed`.
- Route path pattern mismatch — Axum 0.8 uses `{slug}` not `:slug`.

**Log signal:** No log line at all (Axum rejects before the handler runs) OR `method_not_allowed` in the handler.

---

### Layer 2 — Auth (401 Unauthorized)

**Symptom:** `{"error":{"message":"Missing or invalid API key."}}`.

**Check:**
1. `INKWELL_API_KEY` set in env/`.env`?
2. Request sends the `x-api-key: <key>` header (single, ASCII)?
3. Using the admin `INKWELL_API_KEY` or a scoped `ink_<prefix>_<secret>` token — both are accepted via `authenticate`/`require_principal`. A revoked or unknown token is 401; a token lacking the needed scope, or a non-owner mutating another's note, is 403 (`/admin/*` needs `admin`). (The old `INKWELL_MCP_KEY` was retired in slice 4.)
4. Key has leading/trailing whitespace — `Config::from_env` trims and rejects blank keys.

**Code path:** `src/http/auth.rs` → `authenticate` (static-key constant-time compare via `subtle`, else scoped-token lookup by prefix in `src/db/tokens.rs` + constant-time hash compare).

---

### Layer 3 — Validation / DB (400, 409, 404)

**Symptom:** `{"error":{"message":"Field \"bodyMarkdown\" is required..."}}` or `409 Conflict` or `404 Not Found`.

**Check (400):**
- Body exceeds `MAX_REQUEST_BODY_BYTES` (1 MB) → `AppError::PayloadTooLarge`
- Missing required field, wrong type, or field value too long — see `required_string` / `resolve_*` helpers in `api.rs`
- Slug invalid — must match `[a-z0-9][a-z0-9-]*[a-z0-9]`, no consecutive hyphens

**Check (409):**
- Duplicate slug on create → `DbError::DuplicateSlug` → 409 with `slug` field in error body
- Stale `If-Match` on update → `ConditionalUpdate::VersionMismatch` → 409 with current version message; re-read the note and retry

**Check (404):**
- Document exists but caller is unauthenticated and it's a draft → Public visibility filter excludes it; appears as 404 (intentional, no draft leak)

---

### Layer 4 — DB Errors (500)

**Symptom:** `{"error":{"message":"Internal server error."}}` + `tracing::error!` log line with `database error`.

**Check:**
1. `DATABASE_URL` reachable? `GET /health` returns `{"status":"ok","db":"up"}` when the pool is healthy.
2. pgvector extension installed? Migration 0009 creates `vector(1024)` column — fails without `CREATE EXTENSION vector`.
3. SQLx query column mismatch (rare — happens after adding a column without updating all SELECT queries) — look for `ColumnNotFound` in the error log.

**Log:** `tracing::error!(error = %error, "database error")` in `error.rs::IntoResponse`.

---

### Layer 5 — Garden / AI side-effects (best-effort, never 500)

**Symptom:** Write succeeds (200/201) but backlinks not updated, or embeddings stale, or wikilinks render as stubs.

**Check logs for:**
```
WARN persist_source_edges failed; edges rebuild on next save
WARN index_note failed; embeddings rebuild on next save
WARN notes_to_rerender failed; skipping re-render fan-out
```

These are best-effort — the write already succeeded. To force re-index: re-save the note with an identical body (triggers index_note + edge persist with the new version).

**Wikilinks render as stubs:**
- Target slug doesn't exist yet (correct behavior — stubs light up on publish)
- Target is a draft and caller is public (correct — no draft leak)
- `backfill_after_change` failed silently — check WARN logs

## Quick Reference

| Status | Most likely cause | Where to look |
|--------|-------------------|---------------|
| 401 | Missing/wrong `Authorization: Bearer` | `src/http/auth.rs`, env vars |
| 404 | Draft + unauthenticated, or wrong slug | Visibility filter in handler |
| 405 | Handler method dispatch missing | handler's `match method { }` |
| 409 | Duplicate slug OR stale `If-Match` version | `DbError::DuplicateSlug`, `ConditionalUpdate::VersionMismatch` |
| 500 | DB unreachable, pgvector missing, or column mismatch | `GET /health`, migration 0009, SELECT column list |
| 200 but stale content | Best-effort side-effect failed | WARN logs for persist_source_edges / index_note |

## Update Scaffold
- [ ] If this was a recurring failure pattern, update this debug pattern with the new check
- [ ] If the fix touched a convention (e.g., "always apply Visibility"), update `context/conventions.md`
