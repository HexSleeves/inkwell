# Inkwell

An open, API-first Markdown publishing platform implemented as a Rust service.

## Stack

- Rust 2024
- Tokio + Axum
- SQLx + PostgreSQL
- Comrak + Ammonia
- Cargo + Docker Compose

## Environment

- `DATABASE_URL` required
- `PORT` default `3000`
- `HOST` default `0.0.0.0`
- `INKWELL_API_KEY` optional but writes fail closed when unset
- `INKWELL_SITE_URL` optional, used for absolute feed/sitemap/page metadata URLs

## Run

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all
cargo build --release --bin inkwell
export DATABASE_URL=postgres://user:pass@localhost:5432/inkwell
cargo run --bin inkwell -- db migrate
cargo run --bin inkwell -- serve
```

## Docker Compose

```bash
docker compose up --build
```

The app runs `inkwell db migrate && inkwell serve` once Postgres is healthy.

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
