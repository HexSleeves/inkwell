# Outbound webhooks

Inkwell can POST a signed JSON payload to your own services when a document is
**published** or **unpublished** — enough to invalidate a CDN, rebuild a static
mirror, post to a social account, or index the note somewhere else.

Webhooks are **off by default**. With the flag off, the delivery path is fully
inert: no payload is built, no task is spawned, nothing leaves the process.

## Configuration

| Variable | Default | Meaning |
| --- | --- | --- |
| `INKWELL_WEBHOOKS_ENABLED` | `false` | Master switch. Only the exact string `true` (case-insensitive) enables it. |
| `INKWELL_WEBHOOK_URLS` | *(empty)* | Comma-separated `http(s)` endpoints, max 10. Each endpoint receives its own delivery of every event. |
| `INKWELL_WEBHOOK_SECRET` | *(unset)* | Shared secret used as the HMAC-SHA256 key. Minimum 16 characters. |

Turning the flag on with a missing secret, a too-short secret, an empty URL list,
or an unparseable URL **fails startup** with a message naming the problem. That is
deliberate: the two silent failure modes — delivering unsigned, or never
delivering at all — are both worse than a refused boot.

The secret is never logged, never placed in a header, and never part of a
payload. Endpoint URLs *are* logged with each delivery so failures are debuggable.

Compose forwards all three variables (see `docker-compose.yml`), so setting them
in `.env` is enough:

```bash
INKWELL_WEBHOOKS_ENABLED=true
INKWELL_WEBHOOK_URLS=https://hooks.example.com/inkwell
INKWELL_WEBHOOK_SECRET=$(openssl rand -hex 32)
```

### Endpoints are trusted, not filtered

Unlike Webmention targets — which come from *note content* and so pass through
the SSRF guard — webhook endpoints come from the operator's own environment. They
are **not** SSRF-filtered, so delivering to `http://indexer:8080/hook` on a
private Compose network or to `localhost` works as expected. The trust boundary
is the operator, not the author.

## Events

| Event | Fires when |
| --- | --- |
| `document.published` | `POST /documents/{slug}/publish` succeeds. |
| `document.unpublished` | `POST /documents/{slug}/unpublish` succeeds. |

## Request

```
POST /your-endpoint HTTP/1.1
Content-Type: application/json
User-Agent: inkwell-webhooks/0.2.0
X-Inkwell-Event: document.published
X-Inkwell-Delivery: 9f1c2c1e-2c4a-4a1f-9f3b-6f0a0f2a55d1
X-Inkwell-Timestamp: 1795862400
X-Inkwell-Signature: sha256=1b2c3d…
```

| Header | Meaning |
| --- | --- |
| `X-Inkwell-Event` | Event name, mirroring the payload's `event`. |
| `X-Inkwell-Delivery` | UUID for this delivery. **Stable across retries** — use it to dedupe. |
| `X-Inkwell-Timestamp` | Unix seconds, mirroring the payload's `timestamp`. |
| `X-Inkwell-Signature` | `sha256=` followed by the lowercase hex HMAC-SHA256 of the raw body. |

### Payload

```json
{
  "version": 1,
  "event": "document.published",
  "deliveryId": "9f1c2c1e-2c4a-4a1f-9f3b-6f0a0f2a55d1",
  "timestamp": "2026-07-25T19:20:00.000Z",
  "document": {
    "id": "1b8b6f36-9c1e-4f8e-9a0e-6d4b4b3f2a11",
    "slug": "hello-world",
    "title": "Hello, world",
    "status": "published",
    "growth": "seedling",
    "tags": ["rust", "notes"],
    "url": "https://blog.example.com/hello-world",
    "createdAt": "2026-07-01T10:00:00.000Z",
    "updatedAt": "2026-07-25T19:19:58.000Z"
  }
}
```

`version` is the payload-schema version. Additive fields keep version `1`; a
breaking change to the shape bumps it. Treat unknown fields as forward-compatible
and ignore them.

Inkwell has no separate "collections" concept — `tags` are the grouping
primitive, and `growth` carries the digital-garden maturity stage.

`url` is built from `INKWELL_SITE_URL`. If that is unset it falls back to
`http://localhost`, so set it on any deployment that consumes `url`.

## Verifying the signature

The signature is HMAC-SHA256 over the **raw request body bytes**, keyed with your
secret, hex-encoded and `sha256=` prefixed. Verify *before* parsing the JSON, and
compare in constant time.

```js
// Node.js / Express — note `express.raw`: re-serialized JSON will not verify.
import crypto from "node:crypto";

app.post("/inkwell", express.raw({ type: "application/json" }), (req, res) => {
  const secret = process.env.INKWELL_WEBHOOK_SECRET;
  const expected =
    "sha256=" + crypto.createHmac("sha256", secret).update(req.body).digest("hex");
  const presented = req.get("X-Inkwell-Signature") ?? "";

  // Constant-time compare; timingSafeEqual throws on a length mismatch.
  const ok =
    presented.length === expected.length &&
    crypto.timingSafeEqual(Buffer.from(presented), Buffer.from(expected));
  if (!ok) return res.status(401).end();

  const payload = JSON.parse(req.body.toString("utf8"));

  // Reject replays: the timestamp is INSIDE the signed body, so an attacker
  // cannot refresh it without invalidating the signature above.
  const age = Date.now() - Date.parse(payload.timestamp);
  if (!Number.isFinite(age) || Math.abs(age) > 5 * 60 * 1000) return res.status(401).end();

  // Deduplicate: retries reuse deliveryId.
  if (alreadyHandled(payload.deliveryId)) return res.status(204).end();

  handle(payload);
  res.status(204).end();
});
```

```bash
# Same check from a shell, given the raw body in body.json:
openssl dgst -sha256 -hmac "$INKWELL_WEBHOOK_SECRET" -hex < body.json
```

Two things make replay protection work:

1. `timestamp` appears **inside the signed body**, not only in a header. Rejecting
   old timestamps is therefore meaningful — the value is covered by the digest.
   The `X-Inkwell-Timestamp` header is a convenience for cheap pre-parse checks.
2. `deliveryId` is stable across retries, so a receiver can make handling
   idempotent.

## Delivery semantics

Delivery is **best-effort and non-blocking**. It runs in a detached task, so a
receiver that is down, slow, or hostile can never fail or delay the publish
request that triggered it.

| Property | Value |
| --- | --- |
| Attempts per endpoint | 3 (initial + 2 retries) |
| Backoff | 250 ms, then 750 ms |
| Per-attempt timeout | 5 s |
| Retried | transport errors (timeout, refused, DNS), `5xx`, `429` |
| Not retried | any other non-`2xx` — a `400`/`404` is a receiver-side rejection a retry won't fix |
| Success | any `2xx` |

After the last attempt Inkwell gives up and logs a warning. There is no durable
queue and no dead-letter store: a receiver that is down for the length of the
retry window misses that event. If you need at-least-once delivery, reconcile
against `GET /documents` rather than relying on webhooks alone.

Respond quickly (ideally `204` immediately, then work asynchronously) — a
receiver that takes longer than 5 s is treated as a failed attempt and retried.

## Observability

Structured logs are emitted per attempt (`webhook delivered`, `webhook attempt
rejected`, `webhook attempt failed`, `webhook delivery gave up`), each carrying
`event`, `delivery_id`, `endpoint`, and `attempt`. No secret is ever logged.

With `INKWELL_METRICS_ENABLED=true`, `/metrics` exposes:

| Metric | Type | Labels | Meaning |
| --- | --- | --- | --- |
| `inkwell_webhook_attempts_total` | counter | `event`, `result` | Individual attempts, retries included. |
| `inkwell_webhook_deliveries_total` | counter | `event`, `result` | Terminal outcomes, one per endpoint per event. |

`result` is `success` or `failure`. A healthy stream has
`attempts_total{result="success"}` tracking
`deliveries_total{result="success"}`; a gap between them means retries are
happening. See [Observability](OBSERVABILITY.md).

## Out of scope (for now)

- Per-document or per-user webhook subscriptions (needs accounts).
- Inbound webhooks, queues, and durable delivery storage.
