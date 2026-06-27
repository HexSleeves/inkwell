# Inkwell — Professionalization Project Closeout

_Scope: v0.1 baseline → v0.2 release-ready. Closed at merge 1554c72 (CIL-136, PR #44)._

---

## What this project was

The professionalization project transformed Inkwell from a working prototype
into a project that can be handed off, deployed by a new operator, and extended
safely by a future maintainer. No new product capabilities were added; every
item tightened the operational, security, or documentation surface of what
already existed.

The project ran in two waves:

- **Pre-wave (release-readiness features):** infrastructure added alongside docs
  to give the docs something real to describe.
- **Wave A (documentation and operations):** six documents that cover the full
  operator and contributor lifecycle.
- **Wave B (compatibility contracts):** a stability declaration plus a
  contract-test suite that prevents accidental regressions.

---

## Shipped baseline

### Core product (v0.1.0 — shipped before this project)

The items below were already live when the professionalization run began. They
are included here for archival completeness.

| Capability | Notes | ADR |
|-----------|-------|-----|
| Rust/Tokio/Axum/SQLx binary | Single-binary service, replaces Node prototype | [ADR 0007](adr/0007-rust-migration.md) |
| REST CRUD API for documents | `POST/GET/PATCH/PUT/DELETE /documents`, publish/unpublish | [ADR 0004](adr/0004-http-api.md) |
| Public HTML site | Index, paginated, slug, tags, RSS (Atom), sitemap | [ADR 0005](adr/0005-public-web-frontend.md) |
| Postgres persistence | pgvector-capable pg17; migrations via SQLx | [ADR 0003](adr/0003-postgres-persistence.md) |
| Markdown rendering | Comrak + Ammonia sanitize; wikilinks; embeds | [ADR 0002](adr/0002-markdown-rendering.md) |
| Semantic search + RAG | pgvector `note_chunks`, `/ask` + `/related` | — |
| Full-text search | `search_vector` generated column (migration 0008) | — |
| MCP server | 5 tools: `search_notes`, `read_note`, `list_notes`, `create_note`, `update_note` | — |
| Scoped author tokens | Per-author bearer tokens, ownership, write audit; `inkwell author` CLI | [ADR 0009](adr/0009-scoped-author-tokens.md) |
| Media API | `POST /media` + `GET /media/:id` | — |
| Slug rename + 301 redirect | Mutable slug, owner-enforced, alias redirect | [ADR 0011](adr/0011-slug-rename-redirect.md) |
| Flag-gated browser login | Session backend shipped off by default (`INKWELL_BROWSER_LOGIN`) | [ADR 0010](adr/0010-browser-login.md) |
| Webmention | Receiving always-on; sending opt-in via `INKWELL_WEBMENTION_SEND` | — |
| Docker Compose local stack | migrate → seed → serve, API key fail-closed | — |
| Railway auto-deploy | Push to main → deploy; pgvector available | — |
| Conditional GET / ETags | `304` on unchanged pages, feed, sitemap | — |
| CSP + hardening headers | Content-Security-Policy on HTML pages | — |
| Optimistic concurrency | `version` + `If-Match` → `409 Conflict` on stale write | — |

### Pre-wave release-readiness (shipped during this project run)

| Issue | PR | What shipped |
|-------|----|-------------|
| [CIL-125](https://linear.app/hexsleeves/issue/CIL-125) | [#35](https://github.com/HexSleeves/inkwell/pull/35) | Request correlation IDs — `X-Request-Id` middleware; every log line + error envelope carries `requestId` |
| [CIL-128](https://linear.app/hexsleeves/issue/CIL-128) | [#36](https://github.com/HexSleeves/inkwell/pull/36) | GCRA write rate limiting (`governor`) — mutations + `/ask`, keyed by principal/IP, `429 + Retry-After`, `INKWELL_WRITE_RATE_LIMIT` env |

### Wave A — Documentation and operations

| Issue | PR | Document | Purpose |
|-------|----|----------|---------|
| [CIL-124](https://linear.app/hexsleeves/issue/CIL-124) | [#41](https://github.com/HexSleeves/inkwell/pull/41) | [`docs/DEPLOYMENT.md`](DEPLOYMENT.md) | End-to-end production deployment guide for new operators |
| [CIL-126](https://linear.app/hexsleeves/issue/CIL-126) | [#37](https://github.com/HexSleeves/inkwell/pull/37) | [`docs/BACKUP-RESTORE.md`](BACKUP-RESTORE.md) | Backup cadence, restore procedures, migration compatibility |
| [CIL-127](https://linear.app/hexsleeves/issue/CIL-127) | [#42](https://github.com/HexSleeves/inkwell/pull/42) | [`docs/RELEASES.md`](RELEASES.md) | Version scheme, bump procedure, automated release workflow |
| [CIL-133](https://linear.app/hexsleeves/issue/CIL-133) | [#40](https://github.com/HexSleeves/inkwell/pull/40) | [`docs/API.md`](API.md) + [`docs/openapi.yaml`](openapi.yaml) | Authoritative HTTP API reference; machine-readable contract |
| [CIL-134](https://linear.app/hexsleeves/issue/CIL-134) | [#39](https://github.com/HexSleeves/inkwell/pull/39) | [`docs/QA-MATRIX.md`](QA-MATRIX.md) | v0.2 QA matrix and smoke-test checklist |
| [CIL-135](https://linear.app/hexsleeves/issue/CIL-135) | [#38](https://github.com/HexSleeves/inkwell/pull/38) | [`docs/RELEASE-CHECKLIST.md`](RELEASE-CHECKLIST.md) | Pre-release gate checklist for all production deployments |

### Wave B — Compatibility contracts

| Issue | PR | Merge SHA | What shipped |
|-------|----|-----------|-------------|
| [CIL-136](https://linear.app/hexsleeves/issue/CIL-136) | [#44](https://github.com/HexSleeves/inkwell/pull/44) | `1554c72` | [`docs/COMPATIBILITY.md`](COMPATIBILITY.md) — stable vs. flexible surface declaration; contract-test suite in `tests/` covering HTTP routes, document envelope, error envelope, and rendered HTML |

---

## Deferred work

The following items were captured during this project run but explicitly excluded
from v0.2 scope. They belong to the next roadmap cycle.

| Issue | Title | Why deferred |
|-------|-------|-------------|
| [CIL-129](https://linear.app/hexsleeves/issue/CIL-129) | Draft preview | UI-layer work; blocked on browser login UI |
| [CIL-130](https://linear.app/hexsleeves/issue/CIL-130) | Media upload workflow (UI) | File-picker UI deferred; API already shipped (#28) |
| [CIL-131](https://linear.app/hexsleeves/issue/CIL-131) | Site metadata (Open Graph, canonical, meta description) | SEO surface; no blocker except prioritization |
| [CIL-132](https://linear.app/hexsleeves/issue/CIL-132) | Archive / navigation UI | Pagination and archive index improvements; non-critical |
| [CIL-199](https://linear.app/hexsleeves/issue/CIL-199) | Browser login UI | Session backend shipped flag-gated; actual login HTML page still needed |

---

## Known limitations at closeout

These are real edges in the shipped system, documented so future work can
address them deliberately rather than accidentally.

- **Browser login is flag-gated off.** `INKWELL_BROWSER_LOGIN=false` by default
  (ADR 0010). The session backend is production-safe but the login HTML page
  doesn't exist yet (CIL-199). Enabling it without that page serves a blank
  login route.

- **Media upload has no browser UI.** `POST /media` + `GET /media/:id` are
  stable API surfaces (covered in COMPATIBILITY.md), but inserting a media URL
  requires the CLI or direct HTTP. A file-picker page is deferred (CIL-130).

- **Single-node write path.** Rate limiting (`INKWELL_WRITE_RATE_LIMIT`) is
  process-scoped (in-memory GCRA). Horizontal scaling requires a distributed
  counter (Redis or Postgres-backed). Not a v0.2 concern — Railway runs one
  instance.

- **No admin UI.** Token issuance and revocation are API-only (`/admin/tokens`).
  Operators use `curl` or the `inkwell` CLI.

- **Webmention send is manual opt-in.** `INKWELL_WEBMENTION_SEND=true` enables
  sending; failure is logged but does not fail the document write.

---

## Next roadmap themes

These are candidate areas for the next project cycle. None are committed or
scoped here — they are pointers for whoever picks up the roadmap next.

| Theme | Context |
|-------|---------|
| **Admin UI** | Token management, draft review, media library. Requires CIL-199 (login UI) as a prerequisite. |
| **Stronger search** | Full-text search currently uses `ILIKE`; semantic search via pgvector works but has no relevance tuning or hybrid ranking. Candidates: BM25 via `pg_bm25`, cross-encoder rerank, embedding model upgrade. |
| **Analytics** | No traffic metrics exist. Options range from a simple `inkwell_view_events` table to plumbing Plausible or Umami. |
| **Webhooks** | Outbound notification on publish/unpublish events. Enables downstream integrations without polling. |
| **Multi-site support** | Currently one site per instance. A `site_id` tenant column plus routing by `Host` header would allow multiple gardens from one deployment. |
| **Draft preview** | Real-time rendered preview before publish (CIL-129). Requires browser login UI (CIL-199). |
| **Media upload UI** | File-picker page to complete the media authoring loop (CIL-130). |

---

## Reference map

### Documents

| Document | Purpose |
|----------|---------|
| [`docs/DEPLOYMENT.md`](DEPLOYMENT.md) | Production deployment guide |
| [`docs/BACKUP-RESTORE.md`](BACKUP-RESTORE.md) | Backup and restore runbook |
| [`docs/RELEASES.md`](RELEASES.md) | Release process and versioning |
| [`docs/RELEASE-CHECKLIST.md`](RELEASE-CHECKLIST.md) | Pre-release gate checklist |
| [`docs/QA-MATRIX.md`](QA-MATRIX.md) | v0.2 QA matrix |
| [`docs/API.md`](API.md) | HTTP API reference |
| [`docs/openapi.yaml`](openapi.yaml) | OpenAPI 3.1 contract |
| [`docs/COMPATIBILITY.md`](COMPATIBILITY.md) | Stability contracts and flexible surfaces |
| [`docs/QUICKSTART.md`](QUICKSTART.md) | One-command local demo |
| [`docs/RAILWAY.md`](RAILWAY.md) | Railway-specific deployment steps |
| [`docs/RELEASE-NOTES-v0.1.0.md`](RELEASE-NOTES-v0.1.0.md) | v0.1.0 feature summary |

### Architectural Decision Records

| ADR | Decision |
|-----|---------|
| [ADR 0001](adr/0001-toolchain.md) | Toolchain choice |
| [ADR 0002](adr/0002-markdown-rendering.md) | Markdown rendering (Comrak + Ammonia) |
| [ADR 0003](adr/0003-postgres-persistence.md) | Postgres as primary store |
| [ADR 0004](adr/0004-http-api.md) | HTTP API design |
| [ADR 0005](adr/0005-public-web-frontend.md) | Public HTML site |
| [ADR 0006](adr/0006-content-discovery-and-seo.md) | Content discovery and SEO |
| [ADR 0007](adr/0007-rust-migration.md) | Rust migration from Node prototype |
| [ADR 0008](adr/0008-authoring-cli.md) | Authoring CLI |
| [ADR 0009](adr/0009-scoped-author-tokens.md) | Scoped author tokens and write audit |
| [ADR 0010](adr/0010-browser-login.md) | Flag-gated browser session login |
| [ADR 0011](adr/0011-slug-rename-redirect.md) | Slug rename with 301 alias redirect |

### Linear issues

- Professionalization project issues: CIL-124 through CIL-137 (M5: Release Readiness milestone)
- Deferred: CIL-129, CIL-130, CIL-131, CIL-132, CIL-199

---

_Last updated: 2026-06-26 · Professionalization project closed at 1554c72._
