# Release Checklist

- `cargo fmt --check`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo test --all`
- `cargo build --release --bin inkwell`
- `docker compose up --build`
- Smoke: `curl http://localhost:3000/health`
