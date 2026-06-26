# Database Backup and Restore Runbook

Inkwell stores all data in a single PostgreSQL database. This runbook covers
backup cadence, restore procedures for local and Railway (production) environments,
migration compatibility, and post-restore smoke checks.

---

## Table of Contents

1. [Backup Cadence](#backup-cadence)
2. [Taking a Backup](#taking-a-backup)
   - [Local (Docker Compose)](#local-docker-compose)
   - [Railway (Production)](#railway-production)
3. [Restoring](#restoring)
   - [Prerequisites: pgvector Extension](#prerequisites-pgvector-extension)
   - [Restore — Local](#restore--local)
   - [Restore — Railway (Production)](#restore--railway-production)
4. [Migration Compatibility](#migration-compatibility)
5. [Post-Restore Smoke Checks](#post-restore-smoke-checks)

---

## Backup Cadence

| Environment | Recommended cadence | Notes |
|-------------|---------------------|-------|
| **Railway (prod)** | Daily automated + manual before any schema change | Railway Postgres snapshots are available in the dashboard; supplement with `pg_dump` exports you control |
| **Local / staging** | Before every `inkwell db migrate` run; before any bulk import | Cheap insurance when experimenting |

**Before schema changes (mandatory):** Always take a manual `pg_dump` immediately before running migrations. A botched migration is easier to recover from when the backup is minutes old.

---

## Taking a Backup

Backups are plain `pg_dump` exports in PostgreSQL custom format (`-Fc`). Custom
format is compressed, supports parallel restore, and lets you restore individual
tables — prefer it over plain SQL dumps.

### Local (Docker Compose)

The local stack exposes Postgres on host port **5433** (mapped from container
port 5432). Default credentials: user `inkwell`, password `inkwell`, database
`inkwell`.

```bash
# Substitute your actual credentials if you changed them in .env
pg_dump \
  --host=localhost --port=5433 \
  --username=inkwell \
  --dbname=inkwell \
  --format=custom \
  --no-acl --no-owner \
  --file="inkwell-backup-$(date +%Y%m%dT%H%M%S).dump"
```

Or using the `DATABASE_URL` you already have in `.env`:

```bash
pg_dump "$DATABASE_URL" \
  --format=custom \
  --no-acl --no-owner \
  --file="inkwell-backup-$(date +%Y%m%dT%H%M%S).dump"
```

### Railway (Production)

Railway injects `DATABASE_URL` into the service environment. Use the Railway CLI
to run `pg_dump` in a one-off process that shares the service's network and
credentials:

```bash
# Requires: railway login && railway link <project>
railway run --service inkwell \
  pg_dump "$DATABASE_URL" \
    --format=custom \
    --no-acl --no-owner \
    --file="inkwell-prod-$(date +%Y%m%dT%H%M%S).dump"
```

Copy the dump from the Railway ephemeral filesystem to your local machine:

```bash
# If railway run doesn't stream the file, write to stdout instead:
railway run --service inkwell \
  pg_dump "$DATABASE_URL" \
    --format=custom \
    --no-acl --no-owner \
  > "inkwell-prod-$(date +%Y%m%dT%H%M%S).dump"
```

Alternatively, retrieve `DATABASE_URL` from the Railway dashboard and run
`pg_dump` locally against it (Railway Postgres is accessible externally):

```bash
# Replace with your Railway Postgres external connection string
export PROD_DATABASE_URL="postgresql://postgres:<password>@<host>.railway.app:5432/railway"

pg_dump "$PROD_DATABASE_URL" \
  --format=custom \
  --no-acl --no-owner \
  --file="inkwell-prod-$(date +%Y%m%dT%H%M%S).dump"
```

---

## Restoring

### Prerequisites: pgvector Extension

**Critical:** Inkwell requires the `pgvector` extension (migration 0009 creates
the `vector(1024)` column in `note_chunks`). Restoring into a fresh database
without `pgvector` installed will fail when `pg_restore` tries to recreate
that table.

Before restoring, create the extension as a superuser on the **target** database:

```sql
CREATE EXTENSION IF NOT EXISTS vector;
```

Or from the shell:

```bash
psql "$TARGET_DATABASE_URL" -c "CREATE EXTENSION IF NOT EXISTS vector;"
```

On Railway Postgres, `pgvector` is pre-installed; the `CREATE EXTENSION` call
still needs to execute against the target database before restore.

### Restore — Local

1. **Stop the app** (prevents writes during restore):
   ```bash
   docker compose stop app
   ```

2. **Create a fresh target database** (if restoring to a new DB):
   ```bash
   createdb --host=localhost --port=5433 --username=inkwell inkwell_restored
   ```

3. **Install pgvector** on the target:
   ```bash
   psql --host=localhost --port=5433 --username=inkwell --dbname=inkwell_restored \
     -c "CREATE EXTENSION IF NOT EXISTS vector;"
   ```

4. **Restore:**
   ```bash
   pg_restore \
     --host=localhost --port=5433 \
     --username=inkwell \
     --dbname=inkwell_restored \
     --no-acl --no-owner \
     --exit-on-error \
     inkwell-backup-<timestamp>.dump
   ```

5. **Point the app at the restored database** (if testing before cut-over):
   ```bash
   # Update DATABASE_URL in .env, then restart
   docker compose up app
   ```

6. **Run pending migrations** (see [Migration Compatibility](#migration-compatibility)):
   ```bash
   inkwell db migrate
   ```

**Restore in-place** (overwrite the existing local database):

```bash
# Drop and recreate first to avoid constraint conflicts
docker compose stop app
psql --host=localhost --port=5433 --username=inkwell --dbname=postgres \
  -c "DROP DATABASE inkwell; CREATE DATABASE inkwell;"
psql --host=localhost --port=5433 --username=inkwell --dbname=inkwell \
  -c "CREATE EXTENSION IF NOT EXISTS vector;"
pg_restore \
  --host=localhost --port=5433 \
  --username=inkwell --dbname=inkwell \
  --no-acl --no-owner --exit-on-error \
  inkwell-backup-<timestamp>.dump
docker compose start app
```

### Restore — Railway (Production)

**Warning:** Restoring to production overwrites live data. Communicate downtime
before proceeding.

1. **Scale down the app** to prevent writes during restore (Railway dashboard →
   service → Settings → Replicas = 0, or redeploy with `inkwell serve` removed
   from the start command temporarily).

2. **Obtain the target DATABASE_URL** from Railway dashboard → PostgreSQL service
   → Variables → `DATABASE_URL`.

3. **Install pgvector** on the target:
   ```bash
   psql "$RAILWAY_DATABASE_URL" -c "CREATE EXTENSION IF NOT EXISTS vector;"
   ```

4. **Restore:**
   ```bash
   pg_restore \
     --dbname="$RAILWAY_DATABASE_URL" \
     --no-acl --no-owner \
     --exit-on-error \
     inkwell-prod-<timestamp>.dump
   ```

   If restoring to a completely empty Railway database (e.g., after a data loss
   incident requiring a fresh PostgreSQL service), drop and recreate schemas first:
   ```bash
   psql "$RAILWAY_DATABASE_URL" -c "DROP SCHEMA public CASCADE; CREATE SCHEMA public;"
   psql "$RAILWAY_DATABASE_URL" -c "CREATE EXTENSION IF NOT EXISTS vector;"
   pg_restore \
     --dbname="$RAILWAY_DATABASE_URL" \
     --no-acl --no-owner \
     --exit-on-error \
     inkwell-prod-<timestamp>.dump
   ```

5. **Run pending migrations:**
   ```bash
   railway run --service inkwell inkwell db migrate
   ```
   Or trigger a Railway redeploy — `railway.json` runs `inkwell db migrate`
   before each deploy automatically.

6. **Scale the app back up** (reset replicas to 1 or redeploy normally).

---

## Migration Compatibility

Inkwell uses SQLx migrations tracked in the `_sqlx_migrations` table. When you
restore a backup taken from an older schema version, the database will be behind
the current migration state in the codebase.

**After every restore, run:**

```bash
inkwell db migrate
```

This applies any migrations the backup did not include. Migrations are additive
and idempotent (already-applied ones are skipped).

**Caveats:**

- **Downgrade is not supported.** Restoring a backup taken *after* a migration
  into a codebase *before* that migration is not supported. Pin the codebase
  version to the backup's migration level, or restore into a database that
  matches the current codebase.

- **Check migration state before cutting over:**
  ```sql
  SELECT version, description, installed_on
  FROM _sqlx_migrations
  ORDER BY version;
  ```
  The highest `version` in the table should match the highest numbered file in
  `migrations/`. As of this writing, the latest migration is
  `0021_create_slug_aliases.sql`.

- **pgvector column:** Migration 0009 adds `note_chunks.embedding vector(1024)`.
  If `pg_restore` fails on this column, the pgvector extension was not installed
  before restore (see [Prerequisites](#prerequisites-pgvector-extension)).

- **`NOT NULL` owner constraint:** Migration 0017 makes `documents.owner_id NOT NULL`.
  A backup from before 0017 may contain rows with `NULL` owner_id. Migration 0017
  backfills these to the bootstrap admin ID — running `inkwell db migrate` after
  restore handles this automatically.

---

## Post-Restore Smoke Checks

Run these against the restored instance before declaring recovery complete.

### Health and API

```bash
BASE=http://localhost:3000       # or your Railway URL
KEY=<INKWELL_API_KEY>

# 1. Health check
curl -fsS "$BASE/health"
# Expected: 200 OK (body varies but must not error)

# 2. List documents (verifies DB connectivity and read path)
curl -fsS -H "x-api-key: $KEY" "$BASE/documents" | jq '.total'
# Expected: integer >= 0

# 3. Garden graph (verifies link graph tables)
curl -fsS "$BASE/graph" | jq '.nodes | length'
# Expected: integer >= 0

# 4. Full-text search (verifies search_vector column / migration 0008)
curl -fsS "$BASE/search?q=garden" | jq '.hits | length'
# Expected: integer >= 0 (no 500 error)
```

### Write Path

```bash
# 5. Create a smoke-test document
curl -fsS -X POST "$BASE/documents" \
  -H "x-api-key: $KEY" \
  -H "content-type: application/json" \
  -d '{"title":"Restore smoke test","bodyMarkdown":"# Smoke test\n\nCreated during restore verification.","tags":["smoke"]}' \
  | jq '.slug'
# Expected: "restore-smoke-test" (or similar derived slug)

# 6. Publish it
curl -fsS -X POST "$BASE/documents/restore-smoke-test/publish" \
  -H "x-api-key: $KEY"
# Expected: 200

# 7. Read it back via the public HTML page
curl -fsS -o /dev/null -w '%{http_code}\n' "$BASE/restore-smoke-test"
# Expected: 200

# 8. Clean up
curl -fsS -X DELETE "$BASE/documents/restore-smoke-test" \
  -H "x-api-key: $KEY"
# Expected: 204
```

### Semantic Search (if Voyage AI configured)

```bash
# 9. /ask endpoint (verifies note_chunks table and pgvector index)
curl -fsS -X POST "$BASE/ask" \
  -H "x-api-key: $KEY" \
  -H "content-type: application/json" \
  -d '{"question":"What is this garden about?"}' \
  | jq '.answer'
# Expected: non-empty string (or "AI not configured" if ANTHROPIC_API_KEY absent)
# A 500 here indicates the note_chunks table or pgvector index is missing
```

### Auth Boundary

```bash
# 10. Unauthenticated write must be rejected
curl -s -o /dev/null -w '%{http_code}\n' \
  -X POST "$BASE/documents" \
  -H "content-type: application/json" \
  -d '{"title":"should fail"}'
# Expected: 401
```

If all ten checks pass, the restore is complete and the instance is operational.
