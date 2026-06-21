# Inkwell

An open, API-first Markdown publishing platform implemented as a Rust service.

## Stack

- Rust 2024
- Tokio + Axum
- SQLx + PostgreSQL
- Comrak + Ammonia
- Cargo + Docker Compose

## Environment

- Copy `.env.example` to `.env` before local development. `.env` is gitignored.
- `DATABASE_URL` required
- `PORT` default `3000`
- `HOST` default `0.0.0.0`
- `INKWELL_API_KEY` optional but writes fail closed when unset
- `INKWELL_SITE_URL` optional, used for absolute feed/sitemap/page metadata URLs

## Run

```bash
cp .env.example .env
# Full integration tests require DATABASE_URL in your shell or .env.
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all
cargo build --release --bin inkwell
export DATABASE_URL=postgres://user:pass@localhost:5432/inkwell
cargo run --bin inkwell -- db migrate
cargo run --bin inkwell -- serve
```

`cargo test --all` stays runnable without Postgres, but the database-backed
contract tests are skipped unless `DATABASE_URL` is set. Export
`DATABASE_URL` before local verification when you want the API/SQL/migration
path covered, or set `INKWELL_REQUIRE_DB_TESTS=1` to make missing database
configuration fail fast.

## Docker Compose

```bash
cp .env.example .env
docker compose up --build
```

Set `INKWELL_API_KEY` in your shell or `.env` before starting Compose; the app refuses to start until it is set. The app runs `inkwell db migrate && inkwell serve` once Postgres is healthy.

## Staging

See [`docs/STAGING.md`](docs/STAGING.md) for the reproducible staging deploy: secret handling, deploy/redeploy/teardown commands, and the release smoke check.

## HTTP surface

Preserved routes:

- `GET /health`
- `POST /documents`
- `GET /documents`
- `GET /documents/:slug`
- `PATCH /documents/:slug`
- `PUT /documents/:slug`
- `DELETE /documents/:slug`
- `POST /documents/:slug/publish`
- `POST /documents/:slug/unpublish`
- `GET /`
- `GET /page/:page`
- `GET /:slug`
- `GET /tags`
- `GET /tags/:tag`
- `GET /tags/:tag/page/:page`
- `GET /search`
- `GET /feed.xml`
- `GET /sitemap.xml`
