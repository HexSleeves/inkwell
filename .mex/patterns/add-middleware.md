---
name: add-middleware
description: Add a tower/axum middleware layer to the HTTP stack — request-scoped state, response-header rewriting, span enrichment. Covers the layer-ordering trap.
triggers:
  - "add middleware"
  - "middleware"
  - "tower layer"
  - "from_fn"
  - "request id"
  - "correlation id"
  - "tracing span field"
  - "response header"
edges:
  - target: context/conventions.md
    condition: always — naming, error handling, camelCase response types
  - target: context/architecture.md
    condition: when the middleware touches the request → handler → response flow
last_updated: 2026-06-25
---

# Add Middleware

## Context

App-wide middleware is wired in `src/http/router.rs` → `build_router_with_providers`,
after all `.route(...)` calls. Two existing examples:

- `security_headers::apply_security_headers` — `middleware::from_fn`, mutates the
  response (adds headers, reads a per-request `CspNonce` it injected into request
  extensions).
- `request_id::propagate_request_id` (CIL-125) — `middleware::from_fn`, assigns a
  correlation id, exposes it via a `tokio::task_local!`, enriches the `TraceLayer`
  span, and echoes it on the response.

`TraceLayer` (request/response logging + the per-request span) is also a layer in
the same chain.

## Steps

1. **Create the module** `src/http/<name>.rs` with an `async fn(request: Request, next: Next) -> Response`.
2. **Register it** in `src/http/mod.rs` (`pub mod <name>;`).
3. **Add to the `use super::{...}` import** in `router.rs` and add the layer:
   ```rust
   .layer(middleware::from_fn(<name>::<fn>))
   ```
4. **For request-scoped data the error envelope or a span must read** (no handler
   threading): use a `tokio::task_local!` set with `SCOPE.scope(value, async move { next.run(request).await }).await`.
   Read it elsewhere with `SCOPE.try_with(...).ok()`. See `src/http/request_id.rs`
   + `src/error.rs::json_error`.
5. **To add a field to every request log line**: override `TraceLayer`'s span via
   `.make_span_with(|req| info_span!("http_request", ..., my_field = ...))`. Do NOT
   add a second logging middleware — `TraceLayer` already owns request logging.

## Gotchas

- **LAYER ORDER IS REVERSED.** The FIRST `.layer(...)` call is the INNERMOST
  (closest to the handler); the LAST `.layer(...)` is the OUTERMOST (runs first on
  the request, last on the response). To run *before* `TraceLayer` builds its span,
  a middleware must be added *after* `.layer(TraceLayer...)` in the source.
- **`CompressionLayer` is innermost** → on the response path it runs FIRST, so any
  outer middleware sees an already-compressed body. Never try to read/rewrite a
  response body from an outer layer. Inject request-scoped values at
  *construction* time (task-local read inside `IntoResponse`) instead of rewriting
  bytes afterward.
- **Task-local scope must wrap `next.run`**, not just surround the middleware body,
  or handlers/`IntoResponse` won't see it. `make_span_with` for `TraceLayer` runs
  *inside* an outer middleware's scope, so it can read the task-local too.
- **Never trust an inbound header value verbatim** in a response header or a log
  field — validate it (length + charset) to prevent header/log injection. See
  `request_id::is_well_formed`.
- New fields on the error envelope go through `ErrorPayload` in `src/error.rs`;
  keep `#[serde(rename_all = "camelCase")]` and `skip_serializing_if` for options.

## Verify

- [ ] Layer added in the correct position for its ordering requirement (inner vs outer)
- [ ] No second request-logging path introduced (TraceLayer stays the owner)
- [ ] Inbound header values validated before echo/log
- [ ] Contract test covers the observable behaviour (response header, error body, etc.)
- [ ] `cargo fmt --check` + `cargo clippy --all-targets --all-features -- -D warnings` pass
- [ ] `cargo test --all` passes

## Debug

- Field missing from logs → the value-setting middleware is INSIDE `TraceLayer`
  (added before it in source). Move it after `.layer(TraceLayer...)`.
- `task_local` read returns `None` → the read happens outside the `.scope(...)`
  future, or on a different task (a `tokio::spawn` inside the handler does not
  inherit task-locals).
- Response header missing on some responses → the middleware is not the outermost
  layer; something outside it produced the response.

## Update Scaffold
- [ ] Update `.mex/ROUTER.md` "Current Project State" if a new capability shipped
- [ ] Update `context/architecture.md` if the middleware changes the request flow
