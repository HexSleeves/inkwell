# Releases

How to version, build, and publish an Inkwell release from a clean checkout.

## Version scheme

Inkwell follows [Semantic Versioning](https://semver.org/):

| Change | Bump |
|--------|------|
| Breaking API or CLI change, DB migration requires manual intervention | MAJOR |
| New backward-compatible feature (endpoint, flag, migration) | MINOR |
| Bug fix, dependency patch, docs-only | PATCH |

Pre-release tags use a hyphen suffix: `v0.3.0-beta.1`. The automated workflow
skips applying the `latest` Docker tag for any tag that contains a hyphen.

## Version bump procedure

1. Update `version` in `Cargo.toml`.
2. Run `cargo check` to regenerate `Cargo.lock`.
3. Commit: `git commit -m "chore: bump version to vX.Y.Z"`.
4. Push to main and wait for CI to pass.
5. Tag and push (see [Creating a release](#creating-a-release)).

Do **not** bump the version inside the release workflow itself — the tag is the
source of truth; the workflow reads it via `${{ github.ref_name }}`.

## Verification gates

Before tagging, confirm the commit passes every item in
[`docs/RELEASE-CHECKLIST.md`](RELEASE-CHECKLIST.md). The automated workflow
re-runs these same gates and will fail the build if any check does not pass.

Gate summary (authoritative list is in RELEASE-CHECKLIST.md):

- `cargo fmt --check`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo test --all` against a real pgvector Postgres (`INKWELL_REQUIRE_DB_TESTS=1`)
- `cargo build --release --bin inkwell`
- `docker build` smoke (no push)

## Creating a release

### Automated (recommended)

Push a semver tag from a commit that has already passed CI on `main`:

```bash
git checkout main
git pull --ff-only

# Bump Cargo.toml, commit, and push first (see above), then:
git tag v0.2.0
git push origin v0.2.0
```

The `.github/workflows/release.yml` workflow fires on `v*.*.*` tags and:

1. **Verifies** — runs fmt, clippy, full test suite (with postgres), release
   build, and a Docker build smoke check. Fails fast if anything regresses.
2. **Builds** — compiles the release binary (`inkwell`), strips it, and
   packages it as `inkwell-x86_64-unknown-linux-gnu.tar.gz` with a SHA-256
   checksum.
3. **Pushes** the Docker image to GHCR (`ghcr.io/<owner>/inkwell`) tagged with
   the full semver, `MAJOR.MINOR`, `MAJOR`, and `latest` (stable tags only —
   pre-release tags skip `latest`).
4. **Publishes** a GitHub Release with the binary archive + checksum attached
   and auto-generated release notes from merged PR titles.

Monitor the run at `Actions → Release` in the repository.

### Manual (fallback)

Run from a clean checkout of the tag you want to release:

```bash
git checkout v0.2.0

# Verify (mirrors the automated gates)
cargo fmt --all -- --check
cargo clippy --all-targets --all-features --locked -- -D warnings
INKWELL_REQUIRE_DB_TESTS=1 cargo test --all --locked  # requires DATABASE_URL pointing at pgvector pg17
cargo build --release --bin inkwell --locked

# Package binary
strip target/release/inkwell
tar -czf "inkwell-x86_64-unknown-linux-gnu.tar.gz" -C target/release inkwell
sha256sum "inkwell-x86_64-unknown-linux-gnu.tar.gz" > "inkwell-x86_64-unknown-linux-gnu.tar.gz.sha256"

# Build and push Docker image
docker build -t ghcr.io/<owner>/inkwell:v0.2.0 .
docker push ghcr.io/<owner>/inkwell:v0.2.0

# Create GitHub Release (requires gh CLI)
gh release create v0.2.0 \
  inkwell-x86_64-unknown-linux-gnu.tar.gz \
  inkwell-x86_64-unknown-linux-gnu.tar.gz.sha256 \
  --title "v0.2.0" \
  --generate-notes
```

## Artifact locations

| Artifact | Location |
|----------|----------|
| Release binary + checksum | GitHub Release assets (this repo) |
| Docker image | `ghcr.io/<owner>/inkwell:<tag>` |
| Release notes | GitHub Release body (auto-generated from PR titles) |

## Verifying a published release

```bash
# Verify binary checksum
sha256sum -c inkwell-x86_64-unknown-linux-gnu.tar.gz.sha256

# Smoke-test the image
docker run --rm -e INKWELL_API_KEY=test ghcr.io/<owner>/inkwell:v0.2.0 inkwell --version

# Verify the image tag on GHCR
docker pull ghcr.io/<owner>/inkwell:v0.2.0
docker inspect ghcr.io/<owner>/inkwell:v0.2.0 | grep -i labels
```
