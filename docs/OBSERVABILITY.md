# Observability

How to see what a running Inkwell is doing: structured logs, request correlation
ids, health probes, and Prometheus metrics. Design rationale lives in
[ADR 0012](adr/0012-observability.md).

## Quick reference

| Surface | Path | Default |
|---------|------|---------|
| Liveness | `GET /healthz` | always on, no DB |
| Readiness | `GET /readyz` | always on, pings Postgres |
| Readiness (legacy alias) | `GET /health` | always on, identical to `/readyz` |
| Metrics | `GET /metrics` | **off** — `INKWELL_METRICS_ENABLED=true` to register |

| Env var | Default | Meaning |
|---------|---------|---------|
| `INKWELL_LOG` | — | `EnvFilter` directives; wins over `RUST_LOG` |
| `RUST_LOG` | — | fallback `EnvFilter` directives |
| (neither set) | `inkwell=info,tower_http=warn` | built-in default |
| `INKWELL_LOG_FORMAT` | `json` | `pretty` for the human-readable local-dev formatter |
| `INKWELL_METRICS_ENABLED` | `false` | exactly `true` (case-insensitive) registers `/metrics` |
| `INKWELL_METRICS_TOKEN` | — | when set, `/metrics` requires `Authorization: Bearer <token>` |

## Logging

Logs are newline-delimited JSON on stdout, one object per event. Under
`inkwell mcp` they go to **stderr** instead, because that command owns stdout for
its JSON-RPC stream.

### Levels and targets

`INKWELL_LOG` is checked first, then `RUST_LOG`, then the built-in default
`inkwell=info,tower_http=warn`. Both use
[`tracing-subscriber`'s `EnvFilter` syntax](https://docs.rs/tracing-subscriber/latest/tracing_subscriber/filter/struct.EnvFilter.html):

```bash
# everything Inkwell logs, at debug
INKWELL_LOG=inkwell=debug

# debug, but keep per-request lines at info
INKWELL_LOG=inkwell=debug,inkwell::http::observability=info

# quiet: warnings and errors only
INKWELL_LOG=inkwell=warn
```

An unparseable directive string falls back to the default rather than failing
startup.

### Local development

```bash
INKWELL_LOG_FORMAT=pretty INKWELL_LOG=inkwell=debug cargo run -- serve
```

### The per-request event

Every request produces **exactly one** `INFO` event:

```json
{
  "timestamp": "2026-07-25T12:00:00.123456Z",
  "level": "INFO",
  "fields": {
    "message": "request completed",
    "method": "GET",
    "route": "/documents/{slug}",
    "status": 200,
    "latency_ms": 4.812,
    "request_id": "550e8400-e29b-41d4-a716-446655440000"
  },
  "target": "inkwell::http::observability",
  "span": {
    "name": "http_request",
    "method": "GET",
    "route": "/documents/{slug}",
    "request_id": "550e8400-e29b-41d4-a716-446655440000"
  }
}
```

`route` is the **route template**, never the raw path — so `/documents/a` and
`/documents/b` both log `/documents/{slug}`. Requests that matched no route log
`<unmatched>`.

Anything a handler logs during the request is inside the `http_request` span, so
it carries the same `request_id`, `route`, and `method` and joins cleanly against
the request-completed line.

### Correlating a user report

`X-Request-Id` is honoured on the way in (when it is a short token of ASCII
alphanumerics, `-`, or `_`) and always echoed on the response. The same id
appears in the JSON error envelope as `error.requestId`. So a bug report that
quotes a request id maps to exactly one log event:

```bash
curl -si -H 'X-Request-Id: my-trace-42' "$BASE/documents/some-note"
# → x-request-id: my-trace-42

# then, in your log store:
#   fields.request_id = "my-trace-42"
```

### What is never logged

The per-request event carries only the route template, method, status, latency,
and request id. It does **not** log:

- the raw URI or query string (which is how the preview token in
  `/documents/{slug}/preview?token=…` stays out of the logs),
- any request or response header — no `Authorization`, no `X-Api-Key`, no `Cookie`,
- request or response bodies.

`Config`'s `Debug` impl redacts `DATABASE_URL`, `INKWELL_API_KEY`,
`VOYAGE_API_KEY`, `ANTHROPIC_API_KEY`, and `INKWELL_METRICS_TOKEN`, so a config
dump cannot leak them either. Both properties are covered by tests
(`tests/observability_contract.rs::never_logs_tokens_api_keys_cookies_or_authorization`
and `config::tests::debug_does_not_leak_api_key_or_dsn_password`).

## Health probes

Two distinct signals, so an orchestrator can tell "restart this process" from
"don't route traffic here yet":

```bash
curl -si "$BASE/healthz"   # {"status":"ok"}                 — liveness, no DB
curl -si "$BASE/readyz"    # {"status":"ok","db":"up"}       — readiness, pings DB
curl -si "$BASE/health"    # identical to /readyz (legacy alias)
```

- **`/healthz`** consults nothing. Use it for the liveness probe / restart
  policy, so a transient Postgres outage never triggers a restart loop.
- **`/readyz`** runs `SELECT 1` under a 1 s timeout and returns `503` with
  `{"status":"error","db":"down"}` when Postgres is unreachable. Use it for the
  readiness probe and for deploy gating.
- **`/health`** is retained unchanged because `railway.json`, the compose
  healthcheck, and the runbooks point at it.

Kubernetes-style:

```yaml
livenessProbe:
  httpGet: { path: /healthz, port: 3000 }
readinessProbe:
  httpGet: { path: /readyz, port: 3000 }
```

## Metrics

### Enabling and protecting `/metrics`

`/metrics` is **not registered at all** by default, so a default install has no
scrape surface to find. Enable it explicitly:

```bash
INKWELL_METRICS_ENABLED=true
INKWELL_METRICS_TOKEN=$(openssl rand -hex 32)   # strongly recommended
```

When `INKWELL_METRICS_TOKEN` is set, a scrape must present it:

```bash
curl -fsS -H "Authorization: Bearer $INKWELL_METRICS_TOKEN" "$BASE/metrics"
```

A wrong or missing token gets `401`. The comparison is constant-time over SHA-256
digests. Enabling metrics **without** a token logs a startup warning; do that only
when the port is genuinely private.

Pick whichever protection fits your deployment:

1. **Token (portable).** Set `INKWELL_METRICS_TOKEN` and give Prometheus a
   `bearer_token`. Works anywhere, including a single public port.
2. **Network isolation.** Keep `/metrics` enabled but reachable only from your
   monitoring network — e.g. bind Inkwell to a private interface with
   `HOST=10.0.0.5`, or only expose the port inside the compose/Kubernetes network
   rather than publishing it.
3. **Reverse-proxy ACL.** Terminate publicly on nginx/Caddy/Traefik and refuse
   `/metrics` from outside your monitoring CIDR, or require the proxy's own auth.

Layering (2) or (3) on top of (1) is fine and recommended for anything internet-facing.

### Scraping

`prometheus.yml`:

```yaml
scrape_configs:
  - job_name: inkwell
    metrics_path: /metrics
    scheme: https
    authorization:
      credentials: "<INKWELL_METRICS_TOKEN>"
    static_configs:
      - targets: ["blog.example.com"]
```

The response is Prometheus text exposition version 0.0.4
(`Content-Type: text/plain; version=0.0.4; charset=utf-8`). Metrics are
**in-process and non-persistent**: counters reset on restart (which is normal for
Prometheus counters — `rate()` handles it) and each process reports only its own
traffic, so scrape every replica.

### Metric reference

| Metric | Type | Labels | Meaning |
|--------|------|--------|---------|
| `inkwell_http_requests_total` | counter | `method`, `route`, `status` | Requests completed. Use `rate()` for throughput and a `status=~"5.."` ratio for error rate. |
| `inkwell_http_request_duration_seconds` | histogram | `method`, `route`, `status` | Request latency. `_bucket` / `_sum` / `_count`; default bucket ladder from 5 ms to 10 s plus `+Inf`. Use `histogram_quantile()` for p50/p95/p99. |
| `inkwell_db_pool_connections` | gauge | `state="total"` / `state="idle"` | Postgres pool connections held and idle. `total - idle` is in-flight; `total` pinned at the max with `idle` at 0 means pool saturation. |
| `inkwell_process_uptime_seconds` | gauge | — | Seconds since this process started serving. A reset means a restart. |
| `inkwell_build_info` | gauge | `version` | Always `1`; the label carries the running crate version. Use it to confirm a rollout. |
| `inkwell_http_metrics_series` | gauge | — | Distinct label sets tracked in this process. Should sit near the size of the route table. |
| `inkwell_http_metrics_series_dropped_total` | counter | — | Requests folded into the `<overflow>` series after the 2000-series cap. Should stay `0`; anything else means a cardinality bug. |

### Label values

- `method` — a fixed list of HTTP verbs (`GET`, `HEAD`, `POST`, `PUT`, `PATCH`,
  `DELETE`, `OPTIONS`, `TRACE`, `CONNECT`); anything else becomes `OTHER`.
- `route` — the axum route **template**, e.g. `/documents/{slug}`,
  `/tags/{tag}/page/{page}`. Two special values: `<unmatched>` for requests that
  matched no route, and `<overflow>` for anything recorded after the 2000-series
  cap.
- `status` — the numeric HTTP status of the response actually sent, including
  `429`s from the rate limiter and errors from middleware.

No user-supplied value is ever used as a label, so `/metrics` cannot leak note
content, slugs, tokens, or query strings.

### Useful queries

```promql
# Requests per second, by route
sum by (route) (rate(inkwell_http_requests_total[5m]))

# Error ratio
sum(rate(inkwell_http_requests_total{status=~"5.."}[5m]))
  / sum(rate(inkwell_http_requests_total[5m]))

# p95 latency by route
histogram_quantile(0.95,
  sum by (route, le) (rate(inkwell_http_request_duration_seconds_bucket[5m])))

# Pool saturation: connections in use
inkwell_db_pool_connections{state="total"} - inkwell_db_pool_connections{state="idle"}

# Cardinality alarm
increase(inkwell_http_metrics_series_dropped_total[1h]) > 0
```

### Smoke-testing it locally

```bash
export INKWELL_METRICS_ENABLED=true INKWELL_METRICS_TOKEN=dev-scrape-token
cargo run -- serve &

curl -fsS localhost:3000/healthz >/dev/null
curl -fsS localhost:3000/healthz >/dev/null
curl -fsS -H 'Authorization: Bearer dev-scrape-token' localhost:3000/metrics \
  | grep inkwell_http_requests_total
# inkwell_http_requests_total{method="GET",route="/healthz",status="200"} 2
```
