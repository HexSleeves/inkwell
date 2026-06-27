# Inkwell

**An open, API-first Markdown publishing platform built in Rust.**

Write notes in Markdown, link them with `[[wikilinks]]`, publish through a REST API
or CLI, and let your readers browse a fast, public digital garden.

---

## Quick links

<div class="grid cards" markdown>

- :material-rocket-launch: **Get started**

    ---

    One-command local demo with Docker Compose

    [:octicons-arrow-right-24: Quickstart](QUICKSTART.md)

- :material-pencil: **Authoring**

    ---

    CLI commands, scoped tokens, media upload, draft preview

    [:octicons-arrow-right-24: Authoring guide](AUTHORING.md)

- :material-api: **API reference**

    ---

    Full HTTP API with examples, auth, and error shapes

    [:octicons-arrow-right-24: HTTP API](API.md)

- :material-server: **Deploy**

    ---

    Railway, Docker Compose, single binary, reverse proxy

    [:octicons-arrow-right-24: Deployment guide](DEPLOYMENT.md)

</div>

---

## What's in the box

| Feature | Details |
|---------|---------|
| **REST API** | Full document CRUD, publish/unpublish, slug rename + 301 redirect, optimistic concurrency (`If-Match`/`ETag`) |
| **Digital garden** | `[[wikilinks]]` + `![[embeds]]`, backlinks panel, per-note and whole-garden graph, growth stage labels |
| **Full-text search** | Postgres `search_vector` — JSON or HTML response |
| **AI / RAG** | `GET or POST /ask` — pgvector semantic retrieval + Claude synthesis; `/documents/{slug}/related` |
| **MCP server** | 5 tools over stdio (`search_notes`, `read_note`, `list_notes`, `create_note`, `update_note`); auth via scoped token |
| **Author CLI** | `inkwell author new/push/publish/unpublish/upload`; `inkwell import` for bulk Markdown |
| **Scoped tokens** | Per-author `ink_<prefix>_<secret>` tokens; `read/write/publish/admin` scopes; full write audit |
| **Media** | `POST /media` + `GET /media/{id}`; stored in Postgres `bytea`; `inkwell author upload` CLI |
| **Draft preview** | Shareable `pvw_…` token for anonymous draft rendering before publish |
| **Archive nav** | `/archive` year/month buckets; prev/next navigation bar on document pages |
| **Site branding** | `INKWELL_SITE_TITLE`, `INKWELL_SITE_DESCRIPTION`, `INKWELL_SITE_AUTHOR`, `INKWELL_CUSTOM_CSS_URL` |
| **Webmentions** | Receiving always-on; sending opt-in (`INKWELL_WEBMENTION_SEND=true`) |
| **Rate limiting** | GCRA per-principal (or IP), configurable, `429` + `Retry-After` |
| **Request IDs** | `X-Request-Id` on every response and in every error envelope |
| **Ops** | Docker Compose, Railway auto-deploy, single-binary, pgvector pg17 |

---

## 3-step quickstart

```bash
# 1. Copy env template and set your admin key
cp .env.example .env   # edit INKWELL_API_KEY

# 2. Start everything (migrate → seed → serve)
docker compose up

# 3. Visit the garden in your browser
# macOS: open http://localhost:3000
# Linux: xdg-open http://localhost:3000
# Windows: start http://localhost:3000
```

See [QUICKSTART.md](QUICKSTART.md) for the full walkthrough including MCP and AI setup.

---

## Stack

- **Rust 2024** · Tokio · Axum · SQLx
- **PostgreSQL 16/17** with pgvector
- **Comrak + Ammonia** (Markdown → safe HTML)
- Docker Compose / Railway
