# syntax=docker/dockerfile:1
FROM lukemathwalker/cargo-chef:latest-rust-1 AS chef
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
RUN useradd --system --uid 10001 inkwell
COPY --from=builder /app/target/release/inkwell /usr/local/bin/inkwell
USER inkwell
EXPOSE 3000
CMD ["inkwell", "serve"]
