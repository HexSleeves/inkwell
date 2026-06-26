---
name: decisions
description: Key architectural and technical decisions with reasoning. Load when making design choices or understanding why something is built a certain way.
triggers:
  - "why do we"
  - "why is it"
  - "decision"
  - "alternative"
  - "we chose"
  - "best-effort"
  - "visibility"
  - "optimistic"
edges:
  - target: context/architecture.md
    condition: when a decision relates to system structure
  - target: context/stack.md
    condition: when a decision relates to technology choice
  - target: context/ai.md
    condition: when a decision relates to the AI/RAG layer
  - target: patterns/add-endpoint.md
    condition: when implementing a new endpoint that touches visibility or concurrency decisions
  - target: patterns/debug-request-failures.md
    condition: when a design decision (e.g., best-effort side-effects, visibility) caused a confusing failure
last_updated: 2026-06-25
---

# Decisions

## Decision Log

### Best-effort post-write side-effects (never 500 a successful write)
**Date:** 2026-01-01
**Status:** Active
**Decision:** Edge persistence, embedding indexing, and backlink re-render are all best-effort after a document write: failures warn via `tracing::warn!` but never cause the write to return 500.
**Reasoning:** A note was created/updated successfully — that is the durable fact. Stale links or missing embeddings self-heal on the next save. Failing the write for a side-effect failure would make note creation brittle against transient AI API errors.
**Alternatives considered:** Transactional side-effects (rollback write on failure) — rejected because Voyage API calls can't participate in a Postgres transaction. Queue-based async — rejected as over-engineering for a single-node publishing tool.
**Consequences:** The link graph and embedding index may lag by one save after a provider error. Callers must not assume edges or embeddings are immediately consistent.

### Visibility enum instead of boolean `published_only` flag
**Date:** 2026-01-01
**Status:** Active
**Decision:** `Visibility::Public` / `Visibility::All` controls what content any read operation can see; derived from `is_authenticated` in handlers and threaded into every DB query.
**Reasoning:** A single centralized predicate prevents draft-leak bugs — it's impossible to accidentally show a draft to an anonymous caller if every read path goes through `Visibility`. A boolean flag would be easy to forget or invert.
**Alternatives considered:** Checking `status = 'published'` inline in each handler — rejected because it's repetitive and has no structural guarantee that all paths apply it consistently.
**Consequences:** Every new read endpoint must derive `Visibility` from `is_authenticated` and pass it to the DB layer. See `src/db/links.rs` for `Visibility::status_filter()`.

### Raw SQLx queries, no ORM
**Date:** 2026-01-01
**Status:** Active
**Decision:** All database access uses raw `sqlx::query_as` / `sqlx::query` with explicit SQL strings and column lists.
**Reasoning:** SQLx gives compile-time query checking without the abstraction overhead of an ORM. The schema is stable and hand-crafted; ORM mapping would add complexity without benefit for a single-entity publishing domain.
**Alternatives considered:** Diesel — rejected for complexity of schema migrations workflow. SeaORM — rejected for same reason. Pure `tokio-postgres` — rejected because SQLx's typed query macros are a win for safety.
**Consequences:** Every query must enumerate the column list explicitly (`SELECT id, slug, title, body_markdown, rendered_html, status, growth, tags, version, created_at, updated_at FROM documents ...`). Adding a column to `Document` requires updating every SELECT query.

### Optimistic concurrency via `version` + `If-Match`
**Date:** 2026-01-01
**Status:** Active
**Decision:** Documents carry a monotonic `version` counter; PATCH/PUT accepts an `If-Match` header; the SQL UPDATE guards on `WHERE slug = $1 AND version = $2`; mismatch → 409 Conflict.
**Reasoning:** MCP agents can read and write notes concurrently. Without a concurrency guard, a slow agent with stale data would silently clobber a newer edit. The ETag/If-Match pattern is HTTP-standard and requires zero extra round-trips on the happy path.
**Alternatives considered:** Database-level locks — rejected (holds a connection during AI round-trips). Last-write-wins — rejected (silent data loss for concurrent agents).
**Consequences:** Clients must re-read on 409 and retry. The `update_document_by_slug_if_version` function and `ConditionalUpdate` enum in `src/db/documents.rs` implement this. MCP `update_note` always sends `expected_version`.

### Two separate auth tokens (`INKWELL_API_KEY` + `INKWELL_MCP_KEY`)
**Date:** 2026-01-01
**Status:** SUPERSEDED by scoped tokens — `INKWELL_MCP_KEY` was retired in slice 4 (2026-06-23). The MCP server now authenticates with `INKWELL_API_KEY` set to a scoped token; `INKWELL_API_KEY` alone is the admin/bootstrap key. Kept for history.
**Decision:** Human authoring and MCP agent access use separate bearer tokens, both resolved to an admin `Principal` by `authenticate` for reads and `require_principal` for writes.
**Reasoning:** Allows granting/revoking MCP access independently of the human authoring credential. In production you can rotate the MCP key after an agent breach without locking out the human author.
**Alternatives considered:** Single shared key — simpler but no independent revocation. OAuth — rejected as over-engineering for a personal publishing tool.
**Consequences:** Both keys must be set in Railway env vars. `Config` holds both. `authenticate` accepts either as the bootstrap-admin principal. Superseded-in-part by scoped tokens (below): `INKWELL_MCP_KEY` is retired for a scoped token in slice 4.

### Scoped author tokens, per-author audit, admin token surface (ADR 0009, plan 023, slice 2)
**Date:** 2026-06-23
**Status:** Active
**Decision:** Request auth resolves a `Principal` (`author_id`, `label`, `scopes`) via `authenticate(headers, &Config, &PgPool)`. The shared/MCP keys map to the bootstrap-admin principal; a scoped token `ink_<prefix>_<secret>` is looked up by its public `prefix` then verified by a constant-time SHA-256 hash compare (only the hash is stored). Tokens are minted/listed/revoked over HTTP at `/admin/tokens` (admin-gated), kept on the existing `x-api-key` header, and managed by `inkwell author token …`. Writes are audited against the resolving principal. The audit insert is awaited inline (bounded, non-fatal) so the trail is durable on success.
**Reasoning:** Per-author identity + revocable tokens without sessions/OAuth. Admin-gating the token surface from day one prevents a `write` token minting an `admin` token even though document-route scope/ownership enforcement is deferred to slice 3. Reusing `x-api-key` avoids a transport break for existing clients. A security audit trail must not silently drop rows, so the slice-1 detached `tokio::spawn` insert was changed to an awaited insert.
**Alternatives considered:** Direct-DB token CLI — rejected: operators manage prod (Railway) over HTTP and have no DB access. `Authorization: Bearer` transport — rejected: needless break from the existing `x-api-key`. Storing the raw token — rejected: only the hash is ever persisted. Keeping the detached audit insert — rejected: lost rows under load/shutdown defeat the audit.
**Consequences:** New `src/domain/token.rs`, `src/db/tokens.rs`, `src/http/admin.rs`; `AppError::Forbidden` (403). Mutating handlers take `require_principal(...).await?`; reads/visibility use `authenticate(...).await` (anonymous requests short-circuit with no DB hit). Slice 3 turns on scope/ownership enforcement; slice 4 tightens `owner_id NOT NULL` and retires `INKWELL_MCP_KEY`.

### Scope + ownership enforcement; coarse read gate (ADR 0009, plan 023, slice 3)
**Date:** 2026-06-23
**Status:** Active
**Decision:** Mutations enforce a scope — `write` for create/update/delete, `publish` for publish/unpublish (`require_scope`; missing scope → 403). Ownership is enforced **atomically inside the mutating query**: handlers pass `owner_filter(&principal)` (admin → `None` = no constraint; non-admin → the author id) into `update_document_by_slug`/`update_document_by_slug_if_version`/`set_document_status`/`delete_document_by_slug`, whose `WHERE` carries `AND ($n::uuid IS NULL OR owner_id = $n)`. A non-owner matches no row → the handler's normal 404. `create` stamps `owner_id` from the principal. Draft READ visibility requires the `read` scope (`can_see_drafts`); admin implies all. Per-owner draft read ISOLATION is **deferred to slice 3b**.
**Reasoning:** The write/publish ownership boundary is the real privilege win — a leaked `write` token cannot touch others' notes or escalate. Enforcing ownership in the write itself (not a separate read-then-write) closes the TOCTOU window where a slug is deleted+recreated between an ownership check and the mutation (raised by CodeRabbit on PR #21), and makes a non-owner mutation a 404 that doesn't even confirm the note exists. Read-isolation (an author seeing only their own drafts) needs the binary `Visibility` (Public/All) reworked into an owner-aware filter threaded through ~6 query modules — a disproportionate change for today's effectively single-author+admin garden, so it is its own slice (3b).
**Alternatives considered:** Separate `get_document_owner` check-then-write (the first cut) — rejected: TOCTOU window + relies on the NOT-NULL invariant; the atomic owner-scoped write is strictly safer. Add `owner_id` to the `Document` struct + all SELECTs — rejected: broad churn. 403 (not 404) on ownership mismatch — rejected: 404 hides existence (no cross-author enumeration oracle) and falls out naturally from the owner-scoped write. Make `write` imply `read` — rejected: scopes stay orthogonal per ADR (request `read,write` for a read-write agent).
**Consequences:** `NewDocument` gains `owner_id`; the four mutating DB fns take an `owner: Option<Uuid>`; new `require_scope`/`owner_filter`/`can_see_drafts` helpers in `src/http/api.rs`; `src/http/ai.rs` read gate mirrors them. A non-owner write returns 404 (not 403). A `write`-only token can create a draft it cannot read back (no `read` scope) — grant `read,write` for read-write agents. Slice 3b will add owner-aware read visibility.

### Tighten ownership; retire `INKWELL_MCP_KEY` (ADR 0009, plan 023, slice 4)
**Date:** 2026-06-23
**Status:** Active
**Decision:** `documents.owner_id` is `NOT NULL` (migration 0017) — every note has an owner. The **DB default (bootstrap admin) is KEPT**, not dropped: the write API stamps `owner_id` explicitly, but other insert paths (seed, tests, maintenance) legitimately omit it, and the default makes those attribute to the admin rather than violating NOT NULL. The separate `INKWELL_MCP_KEY` credential is **removed** from `Config`/`AuthorConfig`/`auth`; the MCP server (`run_mcp`/`run_stdio`) authenticates with `INKWELL_API_KEY`, which operators set to a scoped token. Only the shared `INKWELL_API_KEY` remains a static admin credential.
**Reasoning:** NOT NULL is the real goal (no orphan notes); dropping the default added fragility (any raw insert omitting `owner_id` would 500) for no correctness gain — deviation from the plan's "drop default", kept deliberately. Retiring the bespoke MCP key collapses two static admin keys into one and pushes MCP onto the scoped-token model (least-privilege, revocable) that slices 2–3 built.
**Alternatives considered:** Drop the DB default per the plan — rejected: breaks seed/test/maintenance inserts; NOT NULL already guarantees ownership. Keep `INKWELL_MCP_KEY` as a deprecated fallback — rejected (user chose full retirement); the scoped-token path supersedes it. A dedicated `INKWELL_MCP_TOKEN` var — rejected: reusing `INKWELL_API_KEY` in the MCP's own environment is simpler and the client sends whatever key it's given.
**Consequences:** **BREAKING for deploys** — Railway (and any MCP host) must set the MCP server's `INKWELL_API_KEY` to a scoped token (mint via `inkwell author token create --scopes read,write`) before the next deploy, or MCP auth fails. `.env.example`/`.mcp.json` updated; `TEST_MCP_KEY` removed (tests authenticate MCP with the shared key or a minted token). Slice 3b (owner-aware read visibility) is the only remaining token work.

### MCP server as a separate CLI process over stdio
**Date:** 2026-01-01
**Status:** Active
**Decision:** `inkwell mcp` runs as a separate process over stdio (`rmcp::transport::io::stdio()`), delegates to `InkwellClient` (HTTP), and never opens a DB connection.
**Reasoning:** The MCP server is a client of the HTTP API, not a peer. This keeps the server the single gatekeeper for auth, validation, and write ordering. The MCP process can be killed/restarted without affecting the running HTTP server.
**Alternatives considered:** Embedding MCP in the HTTP server on a `/mcp` endpoint — rejected because it would require MCP clients to speak HTTP rather than the standard stdio transport.
**Consequences:** `inkwell mcp` requires a running `inkwell serve` (or Railway deploy) to talk to. Set `INKWELL_API_URL` + `INKWELL_API_KEY` (a scoped token) before running. (Pre-slice-4 this used `INKWELL_MCP_KEY`.)

### MockEmbedder + MockLlm for CI/tests
**Date:** 2026-01-01
**Status:** Active
**Decision:** `MockEmbedder` (SHA-256 hash → deterministic 1024-dim vector) and `MockLlm` (deterministic canned answer) are used in tests and when API keys are absent. The real providers are only activated by keys.
**Reasoning:** CI must run without Voyage or Anthropic credentials. The mock embedder is designed to be semantically meaningful (related text is closer than unrelated) so retrieval tests pass without mocking at the query level.
**Alternatives considered:** Mocking HTTP calls (wiremock) — rejected because it tests the HTTP layer, not the RAG retrieval logic. Skipping AI tests — rejected because the retrieval surface is a core feature.
**Consequences:** All tests that exercise `/ask` or related endpoints use `build_router_with_providers` with `MockEmbedder`/`MockLlm`. The deterministic embedding hash depends on SHA-256; changing `mock_embedding` would break existing test fixtures.

### Pragmatic write rate limiting (CIL-128) — reverses the "not separately planned" stance
**Date:** 2026-06-25
**Status:** Active
**Decision:** A single process-wide GCRA limiter (the `governor` crate) throttles *mutation* traffic — every `POST`/`PUT`/`PATCH`/`DELETE` plus the expensive `/ask` endpoint (which is `GET|POST` and drives Voyage + Anthropic). Requests are bucketed by the **authenticated credential** (SHA-256 of the single `x-api-key`, never the raw secret, no DB hit) when present, else by **client IP** (forwarded headers from the platform proxy first, then the `ConnectInfo` peer address). The limit is `INKWELL_WRITE_RATE_LIMIT` requests/minute (default `60`; `0` disables). Over-limit requests get `429 Too Many Requests` with a `Retry-After` header. Read paths and the public HTML site are never consulted by the limiter. Implemented as an axum `from_fn` middleware in `src/http/rate_limit.rs`, layered inside `security_headers` (so a 429 still carries security headers) but outside the handlers (so an over-limit write is rejected before any DB/AI work). `main.rs` now serves with `into_make_service_with_connect_info` so the peer IP is available.
**Reasoning:** A prior security audit judged rate-limiting urgency *blunted by the 256 KiB body cap + constant-time key compare* — those blunt resource-exhaustion and timing-oracle vectors, so rate limiting was **not separately planned**. That stance is reversed here: the body cap bounds a single request's size but not the *count* of requests, and constant-time compare hardens auth but does nothing against a flood of valid-but-abusive writes (e.g. a leaked `write` token, or anonymous `/ask` driving paid Voyage+Anthropic calls). With scoped tokens (ADR 0009) now the write surface and `/ask` publicly reachable, a per-principal/per-IP request ceiling is the missing, cheap, standard control — so it lands now as its own small slice. Keying by credential (not just IP) means a leaked token is throttled even behind a shared proxy, and reusing the existing `x-api-key` keeps it DB-free on the hot path.
**Alternatives considered:** `tower_governor` (the idiomatic layer) — rejected: it rate-limits whatever routes it wraps and can't skip `GET` on a method-multiplexed `any()` route, so excluding reads on `/documents` would force a router restructure; using `governor` directly keeps the algorithm in a vetted crate while a 40-line middleware does the method/path predicate + principal-or-IP keying this router's shape needs. Hand-rolling the GCRA algorithm — rejected (reinventing a solved, security-sensitive primitive). Per-route layering after splitting `any()` into `get().post()` — rejected as broad churn for no behavioral gain. Exempting the admin/shared key — rejected: keeping it bucketed is consistent, testable, and a bulk-import operator can raise or disable the limit via env.
**Dependency scrutiny (`governor` 0.10.4, MIT):** the de-facto Rust rate limiter (single maintainer, widely used), GCRA in-memory via a `DashMap` keyed store; no `unsafe` in our usage; transitive adds are mainstream (`dashmap`, `quanta`, `nonzero_ext`, `portable-atomic`). No RUSTSEC advisory known for 0.10.x. `cargo audit` was **not run** — the binary is not installed in this environment (noted for CI). Memory: the keyed `DashMap` grows with distinct keys; the keyspace here is tiny (a few tokens + the proxy/peer IPs), so no eviction sweep was added — revisit if the key cardinality ever grows.
**Consequences:** New `Config.write_rate_limit` (+ `DEFAULT_WRITE_RATE_LIMIT`), new `AppError::TooManyRequests { retry_after_secs }` (429 + `Retry-After`), new `src/http/rate_limit.rs` + `tests/rate_limit_contract.rs`. `.env.example` and the README env table document the var. **Deploy note:** behind Railway's edge proxy the `ConnectInfo` peer IP is the proxy, so anonymous callers are keyed by the forwarded header when present, else share one bucket (a global cap on anonymous `/ask`+`/webmention`) — acceptable for a pragmatic guard; authenticated writes are always keyed per-credential. Operators can raise the default or set `INKWELL_WRITE_RATE_LIMIT=0` to disable.

### Rate-limit keying hardened to VALIDATE before bucketing (PR #36 review)
**Date:** 2026-06-25
**Status:** Active — amends the CIL-128 decision above.
**Decision:** Keying no longer buckets by the *raw* credential hash. The middleware now resolves the principal through the **same `authenticate`** the handlers use, and keys by `p:<author-id>` only for a *validated* principal (shared key, live scoped token, or — when `INKWELL_BROWSER_LOGIN` is on — a live session); if a principal ever has no `author_id` the key falls back to `p:<label>` (the audit label, e.g. `shared-key`). Everything else keys by client IP. Forwarded-header trust for the IP path is gated behind a new `INKWELL_TRUST_FORWARDED_HEADERS` (default **off**); when off, IP keying uses the real `ConnectInfo` peer. `RateLimitState` now carries `Arc<Config>` + `PgPool`; the middleware short-circuits before any auth/key work when limiting is disabled or the request isn't throttled.
**Reasoning:** PR #36 review (Macroscope HIGH + Copilot) showed the original "hash the credential, no DB hit" keying was a DoS bypass: an anonymous caller on a public route (`/ask`, `/webmention`) could send a *different* random `x-api-key` (or `inkwell_session`) per request, mint a fresh `k:`/`s:` bucket each time, evade the per-IP limit, AND grow the limiter map unboundedly. Validating before bucketing closes all three: an invalid credential resolves to no principal → IP bucket, and the `p:`/`ip:` keyspace is bounded by real principals + real peer IPs. Trusting forwarded headers unconditionally was the symmetric IP-path bypass (spoof `X-Forwarded-For` for a fresh bucket); gating it off-by-default makes a directly-exposed instance unspoofable and only opts in behind a trusted proxy. The "no DB hit on the hot path" goal is consciously traded for correctness: writes are low-volume and already authenticate in the handler, so the extra lookup (and a benign second `touch_last_used`) is acceptable; the disabled-mode short-circuit keeps the zero-cost path for operators who turn limiting off.
**Alternatives considered:** Keep raw-hash keying + cap map size with an LRU/`retain_recent` sweep — rejected: caps memory but not the per-IP bypass (random keys still dodge the IP limit). Duplicate a read-only validator in the limiter to avoid the `touch_last_used` write — rejected: a second copy of auth logic could drift from `authenticate` and become its own security bug; reuse is safer. Revert session keying entirely (Macroscope's suggested diff) — rejected: it drops the legitimate NAT-sharing fix and leaves the x-api-key bypass open. Stash the resolved principal in request extensions for handlers to reuse (kill the double-auth) — deferred: a broad handler refactor, out of scope for this card.
**Consequences:** `RateLimitState::new(Arc<Config>, PgPool)`; keying is async (validates via DB for tokens/sessions; static key + anonymous stay DB-free). New `Config.trust_forwarded_headers` + `INKWELL_TRUST_FORWARDED_HEADERS` (documented). Per-author bucket (an author's tokens share one write budget). Regression test `invalid_api_keys_cannot_bypass_the_limiter` + a `client_ip` trust-gate unit test. **Gotcha:** the keying helper takes `&HeaderMap` + peer `SocketAddr`, not `&Request` — `axum::body::Body` is `!Sync`, so holding `&Request` across the auth `.await` would make the middleware future non-`Send`.
