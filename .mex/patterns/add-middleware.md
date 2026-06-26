---
name: add-middleware
description: Add a cross-cutting tower/axum middleware (rate limit, security headers, auth gate, request id) to the HTTP stack — request-scoped state, span fields, response headers. Covers the layer-ordering trap. Use when a concern must apply across routes rather than inside one handler.
triggers:
  - "add middleware"
  - "middleware"
  - "rate limit"
  - "throttle"
  - "cross-cutting"
  - "tower layer"
  - "from_fn"
  - "request id"
  - "correlation id"
  - "tracing span field"
  - "response header"
  - "security headers"
edges:
  - target: context/conventions.md
    condition: always — naming, error handling (AppError), camelCase response types, config flags
  - target: context/architecture.md
    condition: when the middleware touches the request → handler → response flow / layer order
last_updated: 2026-06-26
---

# Add a cross-cutting middleware

## Context

App-wide middleware is wired in `src/http/router.rs` → `build_router_with_providers`,
after all `.route(...)` calls. The router registers routes with `any(handler)` — methods
are multiplexed **inside** one handler per path (e.g. `GET /documents` lists,
`POST /documents` creates, both on the same route), so you **cannot** cleanly attach a
per-route tower `Layer` that should apply to only some methods. The idiomatic move is a
global `axum::middleware::from_fn` (or `from_fn_with_state`) that inspects
`request.method()` / `request.uri().path()` itself. Worked examples in the same chain:

- `security_headers::apply_security_headers` — `from_fn`, mutates the response (adds
  headers, reads a per-request `CspNonce` it injected into request extensions).
- `request_id::propagate_request_id` (CIL-125) — `from_fn`, assigns a correlation id,
  exposes it via a `tokio::task_local!`, enriches the `TraceLayer` span, and echoes it on
  the response.
- `rate_limit::rate_limit` (CIL-128) — `from_fn_with_state`, holds a shared
  `Arc<RateLimitState>`, filters on method/path, and short-circuits over-limit writes with
  an `AppError`.

`TraceLayer` (request/response logging + the per-request span) is also a layer in this chain.

## Steps

1. **Create the module** `src/http/<name>.rs` with an
   `async fn(/* State(...) optional, */ request: Request, next: Next) -> Response`; add
   `pub mod <name>;` to `src/http/mod.rs`.
2. **Write the middleware fn** with a pass-through fast path:

   ```rust
   pub async fn my_mw(/* State(...) optional, */ request: Request, next: Next) -> Response {
       if !applies(request.method(), request.uri().path()) {
           return next.run(request).await;   // pass-through fast path
       }
       // ... do the work; short-circuit by returning a Response, or:
       next.run(request).await
   }
   ```

3. **Add to the `use super::{...}` import** in `router.rs` and add the layer:

   ```rust
   .layer(middleware::from_fn(<name>::<fn>))
   ```

4. **If it needs shared state** (a limiter, a cache), build it **once** in
   `build_router_with_providers` before `state` moves `config`, wrap in `Arc`, and use
   `middleware::from_fn_with_state(the_arc, my_mw)` with a `State<Arc<T>>` first arg.
5. **For request-scoped data the error envelope or a span must read** (no handler
   threading): use a `tokio::task_local!` set with
   `SCOPE.scope(value, async move { next.run(request).await }).await`. Read it elsewhere
   with `SCOPE.try_with(...).ok()`. See `src/http/request_id.rs` + `src/error.rs::json_error`.
6. **To add a field to every request log line**: override `TraceLayer`'s span via
   `.make_span_with(|req| info_span!("http_request", ..., my_field = ...))`. Do NOT add a
   second logging middleware — `TraceLayer` already owns request logging.
7. **Short-circuit via `AppError`** so the JSON error shape stays centralized — add a
   variant to `src/error.rs` and handle it in `IntoResponse` (set any extra response
   headers there, e.g. `Retry-After`).
8. **Config flags** go through `Config::from_env` + a `DEFAULT_*` const; document the env
   var in `.env.example` AND the README "Environment" list. Add the field to every
   `Config { .. }` literal (grep `Config {` across `src` and `tests`) and to the `Debug` impl.
9. **If you need the client IP**, serve with
   `into_make_service_with_connect_info::<SocketAddr>()` in `main.rs` and read
   `request.extensions().get::<ConnectInfo<SocketAddr>>()` — but it is absent under tower
   `oneshot` in tests, so always have a fallback.

## Gotchas

- **LAYER ORDER IS REVERSED.** The FIRST `.layer(...)` call is the INNERMOST (closest to
  the handler); the LAST `.layer(...)` is the OUTERMOST (runs first on the request, last on
  the response). To run *before* `TraceLayer` builds its span, a middleware must be added
  *after* `.layer(TraceLayer...)` in source. Place a short-circuiting guard **inside**
  `security_headers` (added after it in code) so error responses still get security headers,
  but **outside** the handlers so work is skipped before any DB/AI work runs.
- **`any()` routing ⇒ no per-method layers.** Filter on method/path inside the middleware.
- **`CompressionLayer` is innermost** → on the response path it runs FIRST, so any outer
  middleware sees an already-compressed body. Never read/rewrite a response body from an
  outer layer; inject request-scoped values at *construction* time (task-local read inside
  `IntoResponse`) instead of rewriting bytes afterward.
- **Task-local scope must wrap `next.run`**, not just surround the middleware body, or
  handlers/`IntoResponse` won't see it. `make_span_with` for `TraceLayer` runs *inside* an
  outer middleware's scope, so it can read the task-local too.
- **Never trust an inbound header value verbatim** in a response header or a log field —
  validate it (length + charset) to prevent header/log injection. See
  `request_id::is_well_formed`. For keying, *validate the credential* before it can mint
  per-key state (see `rate_limit`), so forged headers can't grow an unbounded map.
- **Tests use `oneshot`, not a TCP server** — `ConnectInfo` is missing; forwarded headers
  and a constant fallback are what tests exercise. Key auth'd requests by a validated
  principal so tests are deterministic without real peer IPs.
- **Every `Config { .. }` literal must be updated** or the workspace won't compile:
  `src/config.rs` (struct + 2 tests), `src/http/auth.rs`, `tests/common/mod.rs`,
  `tests/browser_login.rs`, `tests/http_caching.rs`.
- **Disable-by-default for tests.** Give the shared `test_config` a no-op value (e.g.
  `write_rate_limit: 0`) so existing contract tests aren't perturbed; add a focused
  `router_for_with_*` helper for the new behavior's own test.
- New fields on the error envelope go through `ErrorPayload` in `src/error.rs`; keep
  `#[serde(rename_all = "camelCase")]` and `skip_serializing_if` for options.

## Verify

- [ ] Layer added in the correct position for its ordering requirement (inner vs outer)
- [ ] No second request-logging path introduced (TraceLayer stays the owner)
- [ ] Inbound header values validated before echo/log; credentials validated before keying
- [ ] Unit-test the pure predicate/state without a DB; contract-test the HTTP behaviour
      (status + headers, error body) end to end
- [ ] `cargo fmt --check` + `cargo clippy --all-targets --all-features -- -D warnings` pass
- [ ] `cargo test --all` passes (set `DATABASE_URL` for the DB-gated contract tests)

## Debug

- Field missing from logs → the value-setting middleware is INSIDE `TraceLayer` (added
  before it in source). Move it after `.layer(TraceLayer...)`.
- `task_local` read returns `None` → the read happens outside the `.scope(...)` future, or
  on a different task (a `tokio::spawn` inside the handler does not inherit task-locals).
- Response header missing on some responses → the middleware is not the outermost layer, or
  the header is set in the middleware instead of `AppError::into_response`; set it there so
  the serialized error response carries it.
- 404 instead of your effect firing → the route isn't registered, or the layer is on the
  wrong side of `.with_state(...)`.

## Update Scaffold

- [ ] Record the decision in `context/decisions.md` (why this concern, why this shape)
- [ ] Update `.mex/ROUTER.md` "Current Project State" if a new capability shipped
- [ ] Update `context/architecture.md` if the middleware changes the request flow
- [ ] Append a line to `events/decisions.jsonl`
