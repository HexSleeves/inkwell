# Release Checklist

Authoritative pre-release gate for all Inkwell production deployments. Work through
each section in order; do not proceed to the next section until all items in the
current one pass.

---

## Phase 1 — Pre-flight (local, before tagging)

### Format and lint

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features --locked -- -D warnings
```

Both must exit `0`. Fix any warnings — Clippy is configured with `-D warnings` in
CI and locally.

### Tests

```bash
# Fast suite (lib + contract tests, no DB required)
cargo nextest run --locked --profile ci-fast --lib
cargo nextest run --locked --profile ci-fast \
  --test domain_contract \
  --test rendering_contract \
  --test clap_cli_contract \
  --test view_layout_contract \
  --test security_headers_contract

# Integration suite (requires a running Postgres with pgvector)
INKWELL_REQUIRE_DB_TESTS=1 DATABASE_URL=postgres://inkwell:inkwell@localhost/inkwell_test \
  cargo test --all --locked
```

All tests must pass. `INKWELL_REQUIRE_DB_TESTS=1` turns DB-skipped tests into
failures — do not skip it.

### Security audit

```bash
cargo audit --ignore RUSTSEC-2023-0071 --ignore RUSTSEC-2026-0185
cargo deny check
```

`cargo audit` must exit `0` for any non-ignored advisory. The two ignored entries
are lockfile-only crates that are never compiled into the binary (see `deny.toml`
for rationale). Any new advisory not in that list must be resolved or explicitly
ignored with justification in `deny.toml` before the release proceeds.

`cargo deny check` must pass for licenses, bans, and sources.

### Release build

```bash
cargo build --release --bin inkwell --locked
docker build -t inkwell:ci .
```

Both must succeed. The `--locked` flag ensures `Cargo.lock` is respected; do not
strip it.

---

## Phase 2 — Migration and backup checks

> The database backup and restore procedure is documented in
> `docs/BACKUP-RESTORE.md` (CIL-126). Read it before proceeding.

### Verify pending migrations

```bash
# List unapplied migrations against the target database
inkwell db migrate --dry-run   # or inspect migrations/ against applied list
```

Confirm the set of new migration files in `migrations/` matches what you expect for
this release. Each migration must be:

- Additive where possible (new tables/columns before dropping old ones).
- Backwards-compatible with the current binary if a mid-deploy window could run
  both old and new binary simultaneously against the same DB.
- Not modifying an already-applied migration file (SQLx checksums these; a changed
  file will abort the deploy).

### Pre-deploy database backup

Take a backup of production **before** triggering the deploy, using the procedure
in `docs/BACKUP-RESTORE.md`. Record the backup location and timestamp here:

```
Backup taken: <timestamp>
Backup location: <path or Railway backup name>
Migration count before deploy: <n>
```

On Railway, enable automatic daily backups via the Postgres service settings and
confirm the most recent backup is no older than 24 hours before deploying.

---

## Phase 3 — Docs review and API compatibility checks

> The API reference is documented in `docs/API.md` (CIL-133).
> Breaking-change contracts are tracked in `docs/COMPATIBILITY.md` (CIL-136).

### Docs review

- [ ] `README.md` reflects the current feature set and no longer references removed
      endpoints or TypeScript/npm tooling.
- [ ] `docs/QUICKSTART.md` commands work against the current binary.
- [ ] `docs/RAILWAY.md` env-var table matches `.env.example`.
- [ ] `docs/STAGING.md` smoke-check commands match current API surface.
- [ ] Release notes drafted in `docs/RELEASE-NOTES-vX.Y.Z.md`.

### API compatibility checks

Compare the previous release tag against `HEAD`:

```bash
git diff <prev-tag>..HEAD -- src/http/
```

For each changed endpoint, verify:

- No change to a documented request/response shape without a version bump or
  migration path (check `docs/COMPATIBILITY.md` for stable contracts).
- New optional fields are additive (existing clients ignore unknown JSON keys).
- Removed fields or changed status codes are called out explicitly in release notes.
- Auth requirements are unchanged or made strictly more permissive (never
  silently tighten without a major release).

Concrete surface to check for breaking changes:

| Area | Files |
|------|-------|
| Route signatures | `src/http/api.rs` |
| Auth enforcement | `src/http/auth.rs` |
| Error envelopes | `src/http/errors.rs` |
| Rate-limit responses | `src/http/rate_limit.rs` |
| Request-ID header | `src/http/request_id.rs` |
| MCP tool schemas | `src/mcp/` |

---

## Phase 4 — Staging smoke

Deploy to the staging environment using the procedure in `docs/STAGING.md`,
then run the full smoke check:

```bash
BASE=http://localhost:3000   # or staging URL
KEY=<INKWELL_API_KEY>

# 1. DB-aware health
curl -fsS "$BASE/health"
# expect: {"status":"ok","db":"up"}

# 2. Write + publish
curl -fsS -X POST "$BASE/documents" \
  -H "x-api-key: $KEY" -H 'content-type: application/json' \
  -d '{"title":"Release smoke","bodyMarkdown":"# Release smoke","tags":["smoke"]}'
curl -fsS -X POST "$BASE/documents/release-smoke/publish" -H "x-api-key: $KEY"

# 3. Public read
curl -fsS "$BASE/"
curl -fsS "$BASE/release-smoke"

# 4. Feed and sitemap carry absolute INKWELL_SITE_URL
curl -fsS "$BASE/sitemap.xml"
curl -fsS "$BASE/feed.xml"

# 5. Fail-closed: unauthenticated write → 401
curl -s -o /dev/null -w '%{http_code}\n' -X POST "$BASE/documents" \
  -H 'content-type: application/json' -d '{"title":"nope"}'

# 6. Rate-limit header present on mutation responses
curl -sI -X POST "$BASE/documents" \
  -H "x-api-key: $KEY" -H 'content-type: application/json' \
  -d '{"title":"header-check","bodyMarkdown":"test"}' | grep -i "x-request-id"

# 7. Teardown smoke document
curl -fsS -X DELETE "$BASE/documents/release-smoke" -H "x-api-key: $KEY"
curl -fsS -X DELETE "$BASE/documents/header-check"  -H "x-api-key: $KEY" || true
```

All checks must pass before tagging.

---

## Phase 5 — Tag and release

```bash
# Confirm working tree is clean and on main
git status
git log --oneline -5

# Tag (vX.Y.Z follows semver)
git tag -a vX.Y.Z -m "Release vX.Y.Z"
git push origin vX.Y.Z
```

The `Release` CI workflow (`.github/workflows/release.yml`) triggers on `v*.*.*`
tags and:

1. Builds the release binary with `cargo build --release --bin inkwell --locked`.
2. Strips and packages `inkwell-x86_64-unknown-linux-gnu.tar.gz` with a SHA-256
   checksum.
3. Pushes a Docker image to GHCR with `latest` + semver tags.
4. Creates a GitHub Release with generated notes and the binary artifact.

Monitor the workflow run in GitHub Actions. All jobs must succeed.

---

## Phase 6 — Production deploy (Railway)

Railway auto-deploys on push to `main`; a tagged release also triggers a deploy
via the standard Docker image path.

```bash
# Confirm the deploy is healthy
railway logs --tail 50   # watch for "inkwell db migrate" then "inkwell serve"
curl -fsS https://your-app.up.railway.app/health
```

Run the Railway smoke check from `docs/RAILWAY.md` against the production URL.

---

## Phase 7 — Post-release

- [ ] GitHub Release published and binary artifact attached.
- [ ] `docs/RELEASE-NOTES-vX.Y.Z.md` committed and linked from the GitHub Release.
- [ ] ROUTER.md "Recently shipped" section updated.
- [ ] Smoke document deleted from production (if created during smoke check).
- [ ] Linear milestone updated: mark shipped issues Done.

---

## Rollback procedure

### Railway

```bash
# List recent deployments
railway deployments

# Redeploy the previous successful deployment
railway redeploy <previous-deployment-id>
```

Railway runs `inkwell db migrate` on each deploy. If the new migrations are not
backwards-compatible, a rollback requires a database restore — see
`docs/BACKUP-RESTORE.md`.

### Docker Compose (staging / self-hosted)

```bash
# Roll back to the previous image tag
docker compose --env-file /etc/inkwell/staging.env down
git checkout <prev-tag>
docker compose --env-file /etc/inkwell/staging.env up --build -d
```

If the release introduced new migrations and the rollback target binary is
incompatible with those migrations, restore the database from the pre-deploy
backup taken in Phase 2 before bringing up the old binary.

### Database restore

Follow `docs/BACKUP-RESTORE.md` exactly. Do not attempt a live migration rollback
by hand-editing applied records in the `_sqlx_migrations` table — restore from backup
instead.

### Decision matrix

| Severity | Action |
|----------|--------|
| Binary crashed / health 503 | Railway redeploy immediately; investigate after |
| API regression (no schema change) | Hotfix branch → CI → merge → auto-deploy |
| DB migration applied, rollback needed | Restore from Phase 2 backup, then redeploy previous tag |
| Security advisory in a compiled crate | Hotfix + `cargo audit` pass + re-release |
