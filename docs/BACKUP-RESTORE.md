# Backup and Restore Runbook

Inkwell keeps authored documents in PostgreSQL and uploaded media bytes behind a
[pluggable storage backend](adr/0013-media-storage.md) — the local filesystem by
default, or the `media_blobs` table. A complete backup needs **both**.

`inkwell backup` and `inkwell restore` do that in one step, producing a single
bundle file. The `pg_dump`/`pg_restore` runbook further down remains the
physical-fidelity alternative, and the one to reach for when you want
Railway's own snapshot tooling or partial table recovery.

---

## Table of Contents

1. [`inkwell backup` / `inkwell restore`](#inkwell-backup--inkwell-restore)
   - [What is and is not backed up](#what-is-and-is-not-backed-up)
   - [Taking a bundle](#taking-a-bundle)
   - [Restoring a bundle](#restoring-a-bundle)
   - [Version compatibility](#version-compatibility)
   - [Docker Compose](#docker-compose)
   - [Railway](#railway)
   - [Scheduling with cron](#scheduling-with-cron)
2. [Backup Cadence](#backup-cadence)
3. [`pg_dump` / `pg_restore` (physical alternative)](#taking-a-backup)
   - [Local (Docker Compose)](#local-docker-compose)
   - [Railway (Production)](#railway-production)
4. [Restoring](#restoring)
   - [Prerequisites: pgvector Extension](#prerequisites-pgvector-extension)
   - [Restore — Local](#restore-local)
   - [Restore — Railway (Production)](#restore-railway-production)
5. [Migration Compatibility](#migration-compatibility)
6. [Post-Restore Smoke Checks](#post-restore-smoke-checks)

---

## `inkwell backup` / `inkwell restore`

A bundle is a single gzipped JSON Lines file: a manifest line recording the
Inkwell version, the schema (migration) version, per-table row counts, and the
source media backend; then every table row; then every media blob, base64-encoded
and verified against its content-addressed key on the way back in.

The dump runs inside one `REPEATABLE READ` transaction, so a bundle is a
consistent snapshot even if the server keeps serving writes while it runs. You do
not need to stop the app to take a backup. You *do* need to stop writes before a
restore.

That consistency is tested, not asserted: a contract test parks a dump partway
through, publishes documents while it is parked, and proves the bundle contains
the snapshot as of the dump's start — the same count in the manifest as rows in
the file, no half-written note, and a clean restore with every foreign key
enforced. A note committed during the dump is simply absent, which is the correct
outcome for a backup taken before it existed.

### What is and is not backed up

**Included:**

| Data | Where it lives |
|------|----------------|
| Documents (published and draft), titles, bodies, tags, status, version | `documents` |
| Slug history so old URLs keep redirecting | `slug_aliases` |
| Wikilink graph | `links` |
| Search corpus | recomputed from `documents` on restore (`search_vector` is a generated column) |
| Embedding chunks and vectors for `/ask` and `/related` | `note_chunks` |
| Authors, scoped API tokens, browser sessions | `authors`, `author_tokens`, `sessions` |
| Write audit trail | `write_audit` |
| Received webmentions | `webmentions` |
| Preview tokens | `preview_tokens` |
| Media metadata **and image bytes** | `media` + the configured media store |

**Not included — and why:**

- **Configuration and secrets.** `INKWELL_API_KEY`, `DATABASE_URL`, AI provider
  keys, `INKWELL_SITE_URL`, and everything else from the environment. A bundle is
  data, not a deployment; keep your env in your own secret store. This is
  deliberate: bundles get copied to laptops and object stores, and a bundle that
  carried credentials would be a credential leak waiting to happen.
- **Migration state.** The target deployment owns its own; the bundle records the
  source's version in the manifest so a mismatch can be detected.
- **The `media_blobs` table as such.** Blobs are dumped through the storage trait
  instead, which is what makes a bundle portable across backends: back up a
  filesystem-backed deployment and restore it onto a Postgres-backed one (or the
  reverse) and every `/media/{id}` URL still resolves.
- **Postgres roles, extensions, and indexes.** `inkwell restore` runs migrations
  on the target first, which recreates all of them. The target's Postgres must
  have `pgvector` available (see [Prerequisites](#prerequisites-pgvector-extension)).

The "included" list above cannot silently fall behind the schema: a contract test
diffs the backed-up table set against the live database, so a future migration
that adds a table fails CI until that table is either added to the bundle or
excluded with a stated reason. An incomplete backup is a bug, not a surprise you
discover during a restore.

### Taking a bundle

```bash
# Writes ./inkwell-backup-<UTC timestamp>.inkwell.gz
inkwell backup

# Explicit destination
inkwell backup --out /backups/inkwell-nightly.inkwell.gz

# Stream to stdout: pipe into gpg, object storage, or ssh
inkwell backup --out - | gpg --encrypt -r ops@example.com > inkwell.inkwell.gz.gpg
inkwell backup --out - | aws s3 cp - s3://my-backups/inkwell/$(date -u +%Y%m%dT%H%M%SZ).inkwell.gz
```

`inkwell backup` reads `DATABASE_URL`, `INKWELL_MEDIA_BACKEND`, and
`INKWELL_MEDIA_DIR` from the environment — the same variables the server uses, so
run it with the same env (or the same container) as your deployment. Progress and
the summary go to **stderr**, which keeps `--out -` a clean pipe.

### Restoring a bundle

```bash
# Into an empty deployment. Migrations run automatically first.
inkwell restore /backups/inkwell-nightly.inkwell.gz

# Into a deployment that already holds data: --overwrite is required
inkwell restore /backups/inkwell-nightly.inkwell.gz --overwrite

# From stdin
gpg --decrypt inkwell.inkwell.gz.gpg | inkwell restore -
```

Stop the app first so nothing writes mid-restore:

```bash
docker compose stop app        # compose
# Railway: set replicas to 0, or redeploy without `inkwell serve`
```

Three safety properties, all covered by
[`tests/backup_restore_contract.rs`](https://github.com/HexSleeves/inkwell/blob/main/tests/backup_restore_contract.rs):

- **Never silently clobber.** Without `--overwrite`, restoring into a deployment
  that holds any documents, media, or authors beyond the seeded bootstrap admin
  fails before the first write and changes nothing — not a row, not a blob.
- **All or nothing.** Every database write happens in one transaction. A bad row,
  a checksum failure, or a missing column aborts the whole restore and leaves the
  target exactly as it was.
- **Consistent under load.** A dump taken while the deployment is serving writes
  is a single-instant snapshot, and restores as one.

With `--overwrite`, the backed-up tables are truncated and replaced wholesale
(not merged), and blobs the previous deployment referenced that the bundle does
not contain are deleted **after** the transaction commits — so a failed restore
never leaves you with neither the old data nor the new.

### Version compatibility

`inkwell restore` **refuses** a bundle whose schema version is newer than the
binary knows, and says so explicitly rather than failing on a missing column
halfway through:

```
Error: bundle schema version 27 is newer than this binary knows (25). The bundle
was written by Inkwell 0.3.0; upgrade to that version or later before restoring.
Nothing was changed.
```

Restoring an **older** bundle works and is the normal upgrade path: migrations run
on the target first, then rows load into the current schema. Columns added since
the bundle was written take their schema defaults, and `inkwell restore` prints a
warning naming each one. Review those warnings — a column with no sensible
default is a real data gap, not noise.

### Docker Compose

The composed stack keeps two volumes:

| Volume | Container path | Holds |
|--------|----------------|-------|
| `inkwell-pgdata` | `/var/lib/postgresql/data` | The database |
| `inkwell-media` | `/app/data/media` | Media blobs (`INKWELL_MEDIA_DIR`) |

Run backup and restore **inside the app container**, which already has
`DATABASE_URL`, `INKWELL_MEDIA_DIR`, and the media volume mounted:

```bash
# Back up to your host's current directory
docker compose exec -T app inkwell backup --out - > "inkwell-$(date -u +%Y%m%dT%H%M%SZ).inkwell.gz"

# Restore from the host
docker compose stop app
docker compose run --rm -T app inkwell restore - --overwrite < inkwell-20260725T140309Z.inkwell.gz
docker compose start app
```

`docker compose run --rm` is used for the restore because `exec` needs the
service running, and the point of the restore is that it is not.

To rehearse the "disk loss" case end to end, wipe both volumes and restore into
the fresh stack:

```bash
docker compose down -v          # destroys inkwell-pgdata AND inkwell-media
docker compose up -d db
docker compose run --rm -T app inkwell restore - < inkwell-20260725T140309Z.inkwell.gz
docker compose up -d app
```

No `--overwrite` is needed there: the deployment is genuinely empty.

### Railway

Railway's filesystem is ephemeral, so set `INKWELL_MEDIA_BACKEND=postgres` there
(the default `local` backend would lose uploads on every redeploy) and the bundle
covers everything with no volume to think about.

```bash
# Requires: railway login && railway link <project>
railway run --service inkwell inkwell backup --out - > "inkwell-prod-$(date -u +%Y%m%dT%H%M%SZ).inkwell.gz"
```

Restore is the same command against the production environment. Scale the service
to 0 replicas first, restore, then scale back up — see
[Restore — Railway](#restore-railway-production) for the scaling steps.

### Scheduling with cron

Copy-pasteable: nightly at 03:15 UTC, 14 days of retention, logged.

```cron
# /etc/cron.d/inkwell-backup
# Nightly Inkwell backup at 03:15 UTC, keeping 14 days.
SHELL=/bin/bash
PATH=/usr/local/bin:/usr/bin:/bin
15 3 * * * inkwell /usr/local/bin/inkwell-backup.sh >> /var/log/inkwell-backup.log 2>&1
```

```bash
#!/usr/bin/env bash
# /usr/local/bin/inkwell-backup.sh
set -euo pipefail

BACKUP_DIR=/var/backups/inkwell
RETAIN_DAYS=14
STAMP=$(date -u +%Y%m%dT%H%M%SZ)

# Same environment the server runs with: DATABASE_URL, INKWELL_MEDIA_BACKEND,
# INKWELL_MEDIA_DIR. Keep this file mode 0600 — it references secrets.
source /etc/inkwell/backup.env

mkdir -p "$BACKUP_DIR"

# Write to a .partial name and rename on success, so a crashed or truncated run
# can never leave a half-written file that looks like a valid backup.
inkwell backup --out "$BACKUP_DIR/inkwell-$STAMP.inkwell.gz.partial"
mv "$BACKUP_DIR/inkwell-$STAMP.inkwell.gz.partial" "$BACKUP_DIR/inkwell-$STAMP.inkwell.gz"

find "$BACKUP_DIR" -name 'inkwell-*.inkwell.gz' -mtime "+$RETAIN_DAYS" -delete
find "$BACKUP_DIR" -name '*.partial' -mtime +1 -delete

echo "$(date -u +%FT%TZ) backup ok: inkwell-$STAMP.inkwell.gz"
```

For Docker Compose, replace the `inkwell backup` line with the
`docker compose exec -T app …` form above.

**A backup you have never restored is not a backup.** Restore into a scratch
deployment (`docker compose down -v` then restore, or a throwaway database) on
some regular cadence, and run the
[post-restore smoke checks](#post-restore-smoke-checks).

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
  `migrations/`. `inkwell db status` prints the same list without a psql session,
  and `inkwell backup` records it in the bundle manifest.

- **pgvector column:** Migration 0009 adds `note_chunks.embedding vector(1024)`.
  If `pg_restore` fails on this column, the pgvector extension was not installed
  before restore (see [Prerequisites](#prerequisites-pgvector-extension)).

- **`NOT NULL` owner constraint:** Migration 0017 makes `documents.owner_id NOT NULL`.
  A backup from before 0017 may contain rows with `NULL` owner_id. Migration 0017
  backfills these to the bootstrap admin ID — running `inkwell db migrate` after
  restore handles this automatically.

- **Media blobs:** where uploaded image bytes live depends on
  `INKWELL_MEDIA_BACKEND` (see [ADR 0013](adr/0013-media-storage.md)):
  - `postgres` — bytes are in the `media_blobs` table and are included
    automatically in `pg_dump` output. Large collections increase dump size
    proportionally (each file up to `INKWELL_MEDIA_MAX_BYTES`, 5 MiB by default).
  - `local` (**the default**) — bytes are files under `INKWELL_MEDIA_DIR` and are
    **not** in `pg_dump`. Back that directory up alongside the dump, e.g.
    `tar -czf media-$(date +%F).tar.gz -C "$INKWELL_MEDIA_DIR" .` (or
    `docker run --rm -v inkwell-media:/media -v "$PWD":/backup busybox tar -czf /backup/media.tar.gz -C /media .`
    for the compose volume). Restore by extracting into the same directory before
    starting the app; blob paths are content-addressed, so the archive is
    position-independent and safe to restore onto a different host.

  A restored database whose media directory is missing serves `404` for those
  images and logs an `error` naming the media id, storage key, and backend.

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
curl -fsS "$BASE/search?q=garden&format=json" | jq '.results | length'
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
  -d '{"q":"What is this garden about?"}' \
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
