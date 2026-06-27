# Inkwell Production Deployment Guide

This guide covers the information a new operator needs to deploy Inkwell in
production without reading source code. For the one-command local demo, see
[`docs/QUICKSTART.md`](QUICKSTART.md). For Railway-specific steps, see
[`docs/RAILWAY.md`](RAILWAY.md).

---

## Contents

1. [Environment Variables](#environment-variables)
2. [Database Requirements](#database-requirements)
3. [Deployment Paths](#deployment-paths)
   - [Docker Compose](#docker-compose)
   - [Single Binary](#single-binary)
   - [Railway (managed)](#railway-managed)
4. [Reverse Proxy and TLS](#reverse-proxy-and-tls)
5. [Secret Handling](#secret-handling)
6. [Database Migration Order and Startup Behavior](#database-migration-order-and-startup-behavior)
7. [Minimal End-to-End Deploy Example](#minimal-end-to-end-deploy-example)

---

## Environment Variables

### Required

| Variable | Description |
|---|---|
| `DATABASE_URL` | PostgreSQL connection string. Must point to a database with the **pgvector** extension available (see [Database Requirements](#database-requirements)). Example: `postgres://inkwell:secret@db:5432/inkwell`. The password is embedded in the DSN — treat it as a secret. |
| `INKWELL_API_KEY` | Admin shared key. Required for all write routes (`POST`, `PUT`, `PATCH`, `DELETE`). The server refuses to start if this is empty or unset. Sent by callers as the `X-Api-Key` request header. Generate with `openssl rand -hex 32`. |

### Optional — server behavior

| Variable | Default | Description |
|---|---|---|
| `HOST` | `0.0.0.0` | Bind address. Leave as `0.0.0.0` in production so the process listens on all interfaces (or the container's interface). |
| `PORT` | `3000` | Bind port. **Do not set on Railway** — Railway injects this value and uses it for routing and healthchecks. |
| `INKWELL_SITE_URL` | _(none)_ | Canonical public URL (e.g. `https://blog.example.com`). Used in RSS feed `<link>` elements, sitemap URLs, and Open Graph metadata. Set this whenever the service is publicly reachable. |
| `INKWELL_WRITE_RATE_LIMIT` | `60` | Write rate limit in requests per minute. Applied per validated principal (credential is verified before it keys a bucket, so forged keys cannot mint quota), or per client IP when anonymous. Covers all mutation routes plus `/ask`. `0` disables limiting. |
| `INKWELL_TRUST_FORWARDED_HEADERS` | `false` | When `true`, the rate limiter keys anonymous callers by `X-Forwarded-For` / `X-Real-IP`. Those headers are client-spoofable — set this to `true` **only** when Inkwell sits behind a proxy that unconditionally overwrites them (e.g. Railway, nginx `proxy_set_header X-Real-IP`). When `false`, IP keying uses the real peer address. |
| `INKWELL_WEBMENTION_SEND` | `false` | Set to `true` to send outbound Webmentions when a published note links to an external page. Receiving Webmentions (`POST /webmention`) is always on regardless of this setting. |
| `INKWELL_BROWSER_LOGIN` | `false` | Set to `true` to enable browser session login (`/auth/login`, `/auth/logout`). Off by default; when off the routes return 404 and no cookie is ever consulted. See ADR 0010. |
| `INKWELL_LLM_MODEL` | `claude-sonnet-4-6` | Claude model used for `/ask` synthesis. Change only if you need to pin a specific model version. |

### Optional — AI / semantic layer

All three are optional. Without them the site still works: full-text search
runs without keys, note embeddings fall back to a deterministic mock (so
related-notes still works), and `/ask` returns a clear "AI features not
configured" response instead of erroring.

| Variable | Description |
|---|---|
| `VOYAGE_API_KEY` | [Voyage AI](https://www.voyageai.com/) key for generating note embeddings (semantic search and related-notes). Without it, the service uses an internal deterministic mock embedder. |
| `ANTHROPIC_API_KEY` | [Anthropic](https://www.anthropic.com/) key for `/ask` answer synthesis. Without it, `/ask` responds with an explanatory message rather than a Claude answer. |
| `OPENAI_API_KEY` | Reserved for future use. Has no effect today. |

### Local / Compose only

These variables configure the Postgres container in Docker Compose and are not
read by the Inkwell binary itself (it only reads `DATABASE_URL`).

| Variable | Default |
|---|---|
| `POSTGRES_USER` | `inkwell` |
| `POSTGRES_PASSWORD` | `inkwell` |
| `POSTGRES_DB` | `inkwell` |
| `POSTGRES_PORT` | `5433` (host-side only) |

---

## Database Requirements

Inkwell requires **PostgreSQL 16 or 17** with the **pgvector** extension
installed. The extension is created automatically by migration `0009` —
the database user named in `DATABASE_URL` must have `CREATE EXTENSION`
privilege, or the extension must already exist.

Use the `pgvector/pgvector:pg17` Docker image or install the extension
manually on a self-managed Postgres instance:

```sql
-- as a superuser or a user with CREATE privilege on the database
CREATE EXTENSION IF NOT EXISTS vector;
```

Railway's managed PostgreSQL does not include pgvector by default. Provision a
**Railway Postgres** service which does include it, or use a pgvector-enabled
external database.

### Media storage

Uploaded images (`POST /media`, `inkwell author upload`) are stored as `bytea`
rows in the `media` table (migration 0019). No separate filesystem or object
storage is required for v1 — the database is the sole store.

**Capacity planning:** each file is capped at 5 MiB. Database size grows with
the number of uploaded assets. On Railway the managed Postgres volume expands
automatically within the plan's limits; on self-hosted installs monitor the
Postgres data directory. A typical photo-heavy garden (hundreds of images)
will add a few GiB; a note-only garden adds nothing.

**Backup:** media blobs are included automatically in every `pg_dump` backup.
No extra steps are needed beyond the standard runbook in
[`docs/BACKUP-RESTORE.md`](BACKUP-RESTORE.md).

---

## Deployment Paths

### Docker Compose

Requires Docker with Compose v2 (`docker compose version`).

```bash
# 1. Set required secrets
cp .env.example .env
#    Edit .env and set INKWELL_API_KEY to a long random string:
#    INKWELL_API_KEY=$(openssl rand -hex 32)

# 2. Start everything (migrate → seed → serve)
docker compose up --build

# 3. Verify
curl http://localhost:3000/health
```

The Compose file (`docker-compose.yml`) starts two services:

- **db** — `pgvector/pgvector:pg17`, data persisted in the named volume
  `inkwell-pgdata`. The app service waits until Postgres passes its healthcheck
  before starting.
- **app** — built from the repo `Dockerfile`, runs
  `inkwell db migrate && inkwell seed /app/examples/garden && inkwell serve`.
  Seed is idempotent: it only plants the bundled sample notes when the garden is
  empty, so restarts never duplicate content.

To stop and wipe the database volume:

```bash
docker compose down -v
```

To upgrade Postgres (major version bump): `docker compose down -v` first, then
change the image tag and bring it back up. The pgvector image does not yet
publish a pg18 tag.

### Single Binary

Requires:
- A Rust toolchain (`cargo build --release`) or a pre-built `inkwell` binary.
- PostgreSQL 16+ with pgvector (see [Database Requirements](#database-requirements)).

```bash
# Build (skip if you have a pre-built binary)
cargo build --release --bin inkwell
# Resulting binary: ./target/release/inkwell

# Export required env vars (or write a .env file and let dotenvy load it)
export DATABASE_URL="postgres://inkwell:secret@localhost:5432/inkwell"
export INKWELL_API_KEY="$(openssl rand -hex 32)"
export INKWELL_SITE_URL="https://blog.example.com"

# Run migrations first, then start the server
./target/release/inkwell db migrate
./target/release/inkwell serve
```

The binary reads `.env` automatically via [dotenvy](https://crates.io/crates/dotenvy) if
that file exists; variables already set in the process environment take
precedence over `.env`. Never commit a `.env` containing real secrets.

To run as a systemd service, write a unit file that sets the environment
variables (or sources a protected env file) and calls `inkwell serve` as the
`ExecStart`. Pair it with a `ExecStartPre=inkwell db migrate` to apply
migrations automatically before each start.

### Railway (managed)

Railway is the project's canonical production platform. Deployment is
fully automated on push to `main`.

1. Create a Railway project from the repo.
2. Add a **PostgreSQL** database service (Railway's managed Postgres includes pgvector).
3. On the Inkwell service, set the following variables:

   | Variable | Value |
   |---|---|
   | `DATABASE_URL` | Reference the PostgreSQL service's `${{Postgres.DATABASE_URL}}` |
   | `INKWELL_API_KEY` | A long random string (`openssl rand -hex 32`) |
   | `INKWELL_SITE_URL` | Your Railway public URL or custom HTTPS domain |
   | `HOST` | `0.0.0.0` |
   | `INKWELL_TRUST_FORWARDED_HEADERS` | `true` (Railway overwrites forwarded headers) |

   Do **not** set `PORT` — Railway injects it.

4. Deploy from the dashboard or CLI (`railway up`).

`railway.json` configures the deploy pipeline:
- **Build**: `Dockerfile`
- **Pre-deploy**: `inkwell db migrate` (runs before traffic shifts to the new deployment)
- **Start**: `inkwell serve`
- **Healthcheck**: `GET /health` (must return 200 before the deploy is promoted)
- **Restart policy**: on failure, up to 3 retries

See [`docs/RAILWAY.md`](RAILWAY.md) for the full Railway walkthrough including
smoke-check commands.

---

## Reverse Proxy and TLS

Inkwell binds plain HTTP. TLS must be terminated by a reverse proxy or a
platform layer (Railway, Fly.io, etc.) in front of it.

**Railway** provides HTTPS automatically on the `*.up.railway.app` domain
and on custom domains once you configure them in the service Networking tab.
No extra reverse proxy configuration is needed.

**Self-hosted with nginx** — minimal config to proxy to a local Inkwell process:

```nginx
server {
    listen 443 ssl http2;
    server_name blog.example.com;

    ssl_certificate     /etc/letsencrypt/live/blog.example.com/fullchain.pem;
    ssl_certificate_key /etc/letsencrypt/live/blog.example.com/privkey.pem;

    location / {
        proxy_pass         http://127.0.0.1:3000;
        proxy_set_header   Host              $host;
        proxy_set_header   X-Real-IP         $remote_addr;
        proxy_set_header   X-Forwarded-For   $proxy_add_x_forwarded_for;
        proxy_set_header   X-Forwarded-Proto $scheme;
    }
}

server {
    listen 80;
    server_name blog.example.com;
    return 301 https://$host$request_uri;
}
```

When using this config, set `INKWELL_TRUST_FORWARDED_HEADERS=true` so the rate
limiter buckets callers by their real client IP rather than the proxy's address.

**Self-hosted with Caddy** — Caddy handles TLS (Let's Encrypt) automatically:

```caddy
blog.example.com {
    reverse_proxy localhost:3000
}
```

Caddy sets `X-Forwarded-For` by default. Set `INKWELL_TRUST_FORWARDED_HEADERS=true`.

---

## Secret Handling

| Secret | How to generate | Where to store |
|---|---|---|
| `INKWELL_API_KEY` | `openssl rand -hex 32` | Env var in platform (Railway, etc.) or a protected `.env` file (mode 600, not committed) |
| `DATABASE_URL` | From your Postgres provider | Same as above; password embedded in the DSN |
| `VOYAGE_API_KEY` | From the Voyage AI dashboard | Same as above |
| `ANTHROPIC_API_KEY` | From the Anthropic console | Same as above |

**MCP agent access**: never give the MCP server the admin `INKWELL_API_KEY`.
Instead, mint a scoped token:

```bash
inkwell author token create --name ai-agent --scopes read,write
# prints ink_<prefix>_<secret> ONCE — copy it; it is unrecoverable
```

Set the MCP server's `INKWELL_API_KEY` env var to that scoped token. This keeps
AI-agent access least-privilege and independently revocable:

```bash
# List tokens
inkwell author token list

# Revoke a token by its prefix
inkwell author token revoke <prefix>
```

Never commit real secret values. The `Debug` implementation for `Config`
redacts all secrets (`<redacted>`) so they never appear in logs.

---

## Database Migration Order and Startup Behavior

Migrations live in `migrations/` and are applied in ascending numeric order by
`inkwell db migrate`:

```
0001_create_documents
0002_add_document_status
0003_add_document_tags
0004_add_documents_list_index
0005_create_links
0006_add_document_version
0007_add_document_growth
0008_add_documents_fts          ← full-text search vector (no key needed)
0009_create_note_chunks         ← installs pgvector extension
0010_create_webmentions
0011_create_authors
0012_create_author_tokens
0013_add_documents_owner
0014_create_write_audit
0015_seed_bootstrap_admin
0016_set_owner_id_default
0017_owner_id_not_null
0018_add_note_chunk_embedding_provenance
0019_create_media
0020_create_sessions
0021_create_slug_aliases
0022_create_preview_tokens
```

**Always run `inkwell db migrate` before `inkwell serve`.**

- SQLx tracks applied migrations in a `_sqlx_migrations` table.
- Migrations are idempotent on re-run — already-applied ones are skipped.
- If migration fails, the server never starts. Fix the database issue and retry.
- Migration `0009` creates the `vector` extension; the database user must have
  `CREATE EXTENSION` privilege or it must already exist.
- Migration `0015` seeds a bootstrap admin author. It is safe to re-run.

On **Railway**: `preDeployCommand = "inkwell db migrate"` in `railway.json`
ensures migrations run before the new deployment receives traffic. A failed
migration aborts the deploy and keeps the previous deployment live.

On **Docker Compose**: the startup command is
`inkwell db migrate && inkwell seed /app/examples/garden && inkwell serve`.
Seed is idempotent — it inserts sample notes only into an empty garden.

On **single-binary deploys**: run `inkwell db migrate` yourself before the
first start and before each upgrade that includes new migrations.

---

## Minimal End-to-End Deploy Example

This example deploys Inkwell as a single binary on a Debian/Ubuntu server
with nginx + Certbot for TLS.

```bash
# --- On the server ---

# 1. Install dependencies
apt-get install -y postgresql-16 postgresql-16-pgvector nginx certbot python3-certbot-nginx

# 2. Create database
# Replace 'replace-with-strong-password' with a random password: openssl rand -hex 16
DB_PASS="replace-with-strong-password"
sudo -u postgres psql -c "CREATE USER inkwell WITH PASSWORD '$DB_PASS';"
# Make inkwell the database owner — required for migration 0009 to create the vector extension.
sudo -u postgres psql -c "CREATE DATABASE inkwell OWNER inkwell;"

# 3. Drop the binary (build on CI or dev machine, scp here)
scp target/release/inkwell user@server:/usr/local/bin/inkwell
chmod +x /usr/local/bin/inkwell

# 4. Write secrets to a protected env file
# Generate a strong API key first:
API_KEY=$(openssl rand -hex 32)
cat > /etc/inkwell.env <<EOF
DATABASE_URL=postgres://inkwell:${DB_PASS}@127.0.0.1:5432/inkwell
INKWELL_API_KEY=${API_KEY}
INKWELL_SITE_URL=https://blog.example.com
# Set INKWELL_TRUST_FORWARDED_HEADERS=true only AFTER the reverse proxy is running.
# Enabling it before nginx is live allows callers to spoof X-Forwarded-For.
INKWELL_TRUST_FORWARDED_HEADERS=false
EOF
chmod 600 /etc/inkwell.env

# 5. Apply migrations
set -a; source /etc/inkwell.env; set +a
inkwell db migrate

# 6. Create a systemd unit
cat > /etc/systemd/system/inkwell.service <<'EOF'
[Unit]
Description=Inkwell
After=network.target postgresql.service

[Service]
User=inkwell
EnvironmentFile=/etc/inkwell.env
ExecStartPre=/usr/local/bin/inkwell db migrate
ExecStart=/usr/local/bin/inkwell serve
Restart=on-failure
RestartSec=5

[Install]
WantedBy=multi-user.target
EOF

useradd --system --no-create-home inkwell
systemctl daemon-reload
systemctl enable inkwell   # registers the unit; start after nginx is configured below

# 7. Configure nginx + TLS
certbot --nginx -d blog.example.com
# Add the proxy_pass block shown in the nginx section above
systemctl reload nginx

# 8. Now start Inkwell (proxy is live — TRUST_FORWARDED_HEADERS is safe to enable)
# Update the env file to trust forwarded headers now that nginx is in front:
sed -i 's/INKWELL_TRUST_FORWARDED_HEADERS=false/INKWELL_TRUST_FORWARDED_HEADERS=true/' /etc/inkwell.env
systemctl start inkwell

# 9. Smoke check
curl -fsS https://blog.example.com/health
```

After the service is running, create an admin token for authoring:

```bash
# From your local machine, targeting the live server
export INKWELL_API_KEY=<your-key>
export INKWELL_API_URL=https://blog.example.com
inkwell author token create --name laptop --scopes read,write,publish
```

Use that scoped token instead of the admin key for day-to-day authoring
and for any MCP agent connections.
