# ADR 0012 — Observability: structured logs, request ids, and `/metrics`

- Status: Accepted
- Date: 2026-07-25
- Relates: ADR 0004 (HTTP API — route shapes are the metric labels), CIL-125
  (request correlation ids), CYP-35 / plan 022 (secret redaction), CYP-46.

## Context

Inkwell shipped v0.1.0 with JSON logs and an `X-Request-Id` middleware, but it was
not actually **operable**:

- Request logging came from `tower_http`'s `TraceLayer`, whose default
  on-request/on-response events are emitted at `DEBUG`. At the default filter
  (`inkwell=info,tower_http=info`) a request produced **zero** log lines, so there
  was nothing to correlate a user-reported error against. Raising the filter to
  `debug` produced **two** lines per request instead of one.
- There were **no metrics at all** — no request rate, no latency distribution, no
  DB pool visibility. Diagnosing staging meant guessing.
- `GET /health` conflated liveness and readiness: it pinged Postgres, so a
  transient DB outage would have told an orchestrator to restart a perfectly
  healthy process.

## Decision

### 1. One structured event per request, from our own middleware

`http::observability::observe` replaces `TraceLayer` (the `trace` feature is
dropped from `tower-http`). Per request it:

- builds an `http_request` span carrying `method`, `route`, and `request_id`, so
  anything a handler logs inherits the correlation fields;
- emits exactly **one** `INFO` event, `"request completed"`, with `method`,
  `route`, `status`, `latency_ms`, and `request_id`.

It sits **inside** the request-id middleware (so the id is in scope) and
**outside** the rate limiter and security headers (so a `429` is still counted and
logged with its real status).

### 2. Logs are JSON by default, `pretty` for local dev

`init_tracing` resolves directives from `INKWELL_LOG`, else `RUST_LOG`, else
`inkwell=info,tower_http=warn`. An invalid directive string falls back to the
default rather than failing startup. `INKWELL_LOG_FORMAT=pretty` swaps the JSON
formatter for the human-readable one. `inkwell mcp` keeps logging to stderr,
because it owns stdout for its JSON-RPC stream.

### 3. Route templates, never raw paths, as metric labels

The `route` label comes from axum's `MatchedPath`, so a million documents collapse
to one `/documents/{slug}` series. This is both a cardinality decision and a
**redaction** decision: the raw URI of `/documents/{slug}/preview?token=…` carries
a preview token, so neither the log event nor the metric label ever contains it.

Label cardinality is bounded on every axis:

| Axis     | Bound                                                              |
|----------|--------------------------------------------------------------------|
| `method` | fixed verb list, anything else becomes `OTHER`                      |
| `route`  | route templates, plus `<unmatched>` for 404s and `<overflow>`        |
| total    | `MAX_SERIES = 2000`; past that, new label sets fold into `<overflow>` and `inkwell_http_metrics_series_dropped_total` moves |

A cardinality leak therefore shows up as a visible counter instead of unbounded
memory growth.

### 4. A hand-rolled registry instead of the `metrics` ecosystem

`http::metrics` is ~250 lines: one counter, one histogram (the default Prometheus
bucket ladder), a few gauges, and a text renderer. We chose this over
`metrics` + `metrics-exporter-prometheus` because:

- the required surface is genuinely that small, and the founding-stage bias is a
  small, well-tested core;
- it avoids a **process-global recorder**, which would make the registry shared
  between concurrently-running integration tests;
- the exposition text becomes directly unit-testable, which is how the
  bucket/cumulation and escaping rules are pinned.

The cost is that we own the exposition format. It is pinned by tests, and the
format (version 0.0.4) is stable and simple. If Inkwell later needs exemplars,
summaries, or OpenTelemetry export, swapping in the ecosystem crates is a
contained change behind `Metrics`.

### 5. `/metrics` is off by default

The route is **registered only** when `INKWELL_METRICS_ENABLED=true`. On a default
install the path does not exist, so it cannot be scraped, probed, or
fingerprinted. When enabled, `INKWELL_METRICS_TOKEN` (optional) requires a
matching `Authorization: Bearer …`, compared constant-time over SHA-256 digests
like `http::auth::match_static_key`. Enabling without a token logs a startup
warning. The public `/settings` page deliberately does **not** advertise whether
metrics are on.

### 6. `/healthz` (liveness) and `/readyz` (readiness) are distinct

`/healthz` touches nothing external and answers `200` whenever the HTTP stack can
serve. `/readyz` runs `SELECT 1` under a 1 s timeout and answers `503` when
Postgres is unreachable. `/health` is retained as an alias of `/readyz`: deploy
configs (`railway.json`, compose healthchecks) and the runbooks point at it, and
its response body is a documented wire contract.

## Consequences

- An operator can answer "is it up", "is it ready", "how fast", "how often", and
  "which request was that" without attaching a debugger.
- One log line per request keeps volume predictable and makes `request_id` the
  single join key across the log event, the error envelope, and the response
  header.
- We own the Prometheus text format. Every metric family's `HELP`/`TYPE` and the
  histogram cumulation are asserted by unit tests so a regression is caught in
  CI, not by a broken dashboard.
- Metrics are opt-in, so upgrading an existing install exposes nothing new until
  the operator asks for it.
- Adding a route adds label values. That is bounded by the route table, but a
  future dynamic-route feature must keep using `MatchedPath`, not raw paths.

See `docs/OBSERVABILITY.md` for the scrape setup, the metric reference, and the
logging env vars.
