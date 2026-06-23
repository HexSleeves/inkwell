# syntax=docker/dockerfile:1
# Pin the builder to the bookworm-based chef image so its glibc matches the
# `debian:bookworm-slim` runtime below. The default `latest-rust-1` tracks the
# newest Debian and links a newer glibc (e.g. GLIBC_2.38), which the bookworm
# runtime lacks — the binary then fails at startup with "GLIBC_x not found".
FROM lukemathwalker/cargo-chef:latest-rust-1-bookworm AS chef
WORKDIR /app

FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
COPY --from=planner /app/recipe.json recipe.json
RUN cargo chef cook --release --recipe-path recipe.json
COPY . .
RUN cargo build --release --bin inkwell

FROM debian:bookworm-slim AS runtime
# `debian:bookworm-slim` ships without ca-certificates. reqwest's default
# native-tls backend loads the system root cert store when a client is built;
# with no cert bundle, `reqwest::Client::builder().build()` fails — breaking
# every server-side HTTP client (ai/voyage, ai/claude, federation/fetch).
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates \
 && rm -rf /var/lib/apt/lists/*
RUN useradd --system --uid 10001 inkwell
COPY --from=builder /app/target/release/inkwell /usr/local/bin/inkwell
USER inkwell
WORKDIR /app
# Bundle the sample vault so `inkwell seed` can plant a populated demo garden at
# runtime (the compose app points the seed step at this path).
COPY --chown=inkwell:inkwell examples/garden /app/examples/garden
EXPOSE 3000
CMD ["inkwell", "serve"]
