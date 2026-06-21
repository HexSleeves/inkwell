# Staging Environment Runbook

Reproducible staging deploy for Inkwell. The same Docker Compose recipe runs on
any host (a dev box, a VM, or a small cloud instance), which is what makes
staging reproducible and keeps the platform self-hostable. This runbook is the
authoritative deploy/teardown/redeploy + secret-handling procedure for CYP-37.

## What staging is

A throwaway, production-shaped instance used to validate a release before it goes
anywhere user-facing:

- `app` container — the `inkwell` binary, runs `inkwell db migrate && inkwell serve`.
- `db` container — Postgres 17, data on the named volume `inkwell-pgdata`.

Both are defined in `docker-compose.yml` / `Dockerfile`. Nothing staging-specific
is baked into the image; everything that differs between environments is an env var.

## Prerequisites

- Docker Engine 24+ with the Compose v2 plugin (`docker compose version`).
- Outbound network access to pull `postgres:17-alpine` and the Rust build images.
- A host that can hold the port you publish (default `3000`) and run a Postgres
  container.

## Secrets (never commit these)

Staging needs two app secrets plus the Postgres password. `.env` and `.env.*`
are gitignored (except `.env.example`), so a staging env file is safe to keep on
the host but must never be committed.

| Variable            | Purpose                                              | Notes                                        |
| ------------------- | ---------------------------------------------------- | -------------------------------------------- |
| `INKWELL_API_KEY`   | Shared write credential for the authoring API.       | Required. App fails closed on writes without it. |
| `INKWELL_SITE_URL`  | Absolute base URL for feed/sitemap/page metadata.    | Set to the public staging URL.               |
| `POSTGRES_PASSWORD` | Postgres superuser password for the `db` container.  | Generate a fresh random value.               |

Generate strong values and write them to an env file the repo never sees. Keep it
outside the repo tree (e.g. `/etc/inkwell/staging.env`) or use a name matching the
gitignored `.env.*` glob:

```bash
umask 077
cat > /etc/inkwell/staging.env <<EOF
POSTGRES_USER=inkwell
POSTGRES_PASSWORD=$(openssl rand -hex 16)
POSTGRES_DB=inkwell
POSTGRES_PORT=5432
PORT=3000
HOST=0.0.0.0
INKWELL_API_KEY=$(openssl rand -hex 32)
INKWELL_SITE_URL=https://staging.example.com
EOF
```

For a managed secret store (Vault, SSM, Doppler, etc.), inject the same variables
into the environment before invoking Compose instead of writing a file.

### Fail-closed guarantee

Two layers enforce "no key, no writes":

1. **Compose** uses `INKWELL_API_KEY: '${INKWELL_API_KEY:?...}'`, so
   `docker compose up` aborts before starting if the variable is unset or empty.
2. **App** (`src/http/auth.rs`): when the configured key is `None`/empty, every
   write route (`POST/PATCH/PUT/DELETE /documents`, publish/unpublish) returns
   `401 Unauthorized`. Reads of published content stay public.

## Deploy

```bash
# from the repo root, on the staging host
docker compose --env-file /etc/inkwell/staging.env up --build -d
```

This builds the release image, waits for Postgres to pass its healthcheck, runs
migrations, then serves. First build compiles Rust in release mode and takes
several minutes; subsequent builds are cached via cargo-chef.

Check it came up:

```bash
docker compose --env-file /etc/inkwell/staging.env ps
curl -fsS http://localhost:3000/health   # {"status":"ok","db":"up"}
```

## Verify (release smoke)

This is the CYP-37 acceptance check; it also covers the smoke step in
`docs/RELEASE-CHECKLIST.md`. Replace `BASE` with the public staging URL and
`KEY` with `INKWELL_API_KEY`.

```bash
BASE=http://localhost:3000
KEY=<INKWELL_API_KEY>

# 1. DB-aware health
curl -fsS "$BASE/health"

# 2. Create a document via the write API, then publish it
curl -fsS -X POST "$BASE/documents" \
  -H "x-api-key: $KEY" -H 'content-type: application/json' \
  -d '{"title":"Staging smoke","bodyMarkdown":"# Hello from staging","tags":["smoke"]}'
curl -fsS -X POST "$BASE/documents/staging-smoke/publish" -H "x-api-key: $KEY"

# 3. Read it on the public surface (no key)
curl -fsS "$BASE/"                 # index lists the published doc
curl -fsS "$BASE/staging-smoke"    # rendered page

# 4. Absolute URLs honor INKWELL_SITE_URL
curl -fsS "$BASE/sitemap.xml"      # <loc> entries use INKWELL_SITE_URL
curl -fsS "$BASE/feed.xml"         # <link href> entries use INKWELL_SITE_URL

# 5. Fail-closed: a write without the key is rejected
curl -s -o /dev/null -w '%{http_code}\n' -X POST "$BASE/documents" \
  -H 'content-type: application/json' -d '{"title":"nope"}'   # 401
```

Pass criteria: health is `ok`/`up`, the created doc appears on `/` and renders at
its slug, `sitemap.xml`/`feed.xml` carry absolute `INKWELL_SITE_URL` URLs, and the
unauthenticated write returns `401`.

## Redeploy (ship a new build)

```bash
git pull
docker compose --env-file /etc/inkwell/staging.env up --build -d
```

Compose rebuilds the `app` image and recreates only changed containers. The
`db` volume persists, so data and applied migrations survive. New migrations run
automatically on app start.

## Teardown

```bash
# stop containers, keep data
docker compose --env-file /etc/inkwell/staging.env down

# stop AND wipe the Postgres volume (full reset)
docker compose --env-file /etc/inkwell/staging.env down -v
```

Use `down -v` to recycle staging from a clean database. The env file on the host
is unaffected; delete it separately when decommissioning.

## Host choice

The Compose recipe is host-agnostic. Options:

- **Any Docker host / small VM** — recommended for v0.1. Clone the repo, drop the
  staging env file, run the deploy command. A `$5–10/mo` 1 GB instance is enough.
  Front with a reverse proxy (Caddy/Nginx) for TLS and set `INKWELL_SITE_URL` to
  the HTTPS hostname.
- **Local / ephemeral** — the exact recipe above on a dev box, used to validate
  the release path (this is how CYP-37 was verified).

Paid hosting requires board sign-off: escalate to the CEO with a provider +
monthly estimate before incurring spend (see CYP-37 thread).

## Troubleshooting

- `up` aborts with an `INKWELL_API_KEY` error → the var is unset/empty in your
  env file or shell. This is the fail-closed guard working as intended.
- `/health` returns `503` / `"db":"down"` → Postgres not healthy yet; check
  `docker compose ... logs db`. The app's healthcheck retries for ~50s.
- Writes return `401` with a key set → confirm the `x-api-key` header value
  matches `INKWELL_API_KEY` exactly (it is compared in constant time).
- Port already in use → change `PORT` (app) or `POSTGRES_PORT` (db) in the env
  file; both are parameterized in `docker-compose.yml`.
