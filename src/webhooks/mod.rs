//! Outbound signed webhooks on publish / unpublish (CYP-53) — OPT-IN, default OFF.
//!
//! When `INKWELL_WEBHOOKS_ENABLED=true` (plus a secret and at least one endpoint
//! URL), publishing or unpublishing a document POSTs a small versioned JSON
//! document to every configured endpoint, signed with HMAC-SHA256 over the raw
//! request body. With the flag off — the default — [`maybe_dispatch`] returns
//! before building a payload, so the whole path is inert: no serialization, no
//! task spawned, no network touched.
//!
//! ## Guarantees
//!
//! - **Never blocks or fails a publish.** Delivery runs in a detached task, so a
//!   receiver that is down, slow, or hostile cannot delay or fail the HTTP
//!   request that triggered it.
//! - **Bounded work.** [`MAX_ATTEMPTS`] attempts per endpoint with fixed backoff,
//!   a hard [`ATTEMPT_TIMEOUT`] per attempt, and at most
//!   [`crate::config::MAX_WEBHOOK_ENDPOINTS`] endpoints. Then we give up and log.
//! - **No secret leakage.** The secret is only ever used as an HMAC key. It is
//!   never logged, never placed in a header, and never part of the payload.
//!
//! ## Not SSRF-guarded — on purpose
//!
//! Unlike Webmention targets (which are derived from *note content* and so pass
//! through [`crate::federation::ssrf`]), webhook endpoints are configured by the
//! operator in the process environment. Filtering them would break the common
//! self-hosted case of delivering to another service on the same private network
//! or `localhost`. The trust boundary is the operator, not the author.

use std::time::Duration;

use hmac::{Hmac, KeyInit, Mac};
use serde_json::{Value, json};
use sha2::Sha256;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::config::Config;
use crate::domain::document::{Document, timestamp};
use crate::http::AppState;
use crate::http::metrics::Metrics;

/// Payload schema version. Bump only for a breaking change to the JSON shape;
/// additive fields keep version 1.
pub const PAYLOAD_VERSION: u32 = 1;

/// Total attempts per endpoint, including the first. Two retries is enough to
/// ride out a receiver restart without turning a dead endpoint into a queue.
pub const MAX_ATTEMPTS: usize = 3;

/// Fixed backoff before retry N (so: attempt, wait 250ms, attempt, wait 750ms,
/// attempt). Deliberately short and jitter-free: this is best-effort delivery,
/// not a durable queue, and a predictable schedule is testable.
const RETRY_BACKOFF: [Duration; MAX_ATTEMPTS - 1] =
    [Duration::from_millis(250), Duration::from_millis(750)];

/// Hard per-attempt timeout. A receiver that hangs is treated as a failure and
/// retried rather than holding a task open.
pub const ATTEMPT_TIMEOUT: Duration = Duration::from_secs(5);

/// `User-Agent` sent on every delivery so receivers can identify us.
const USER_AGENT: &str = concat!("inkwell-webhooks/", env!("CARGO_PKG_VERSION"));

/// Header carrying the hex HMAC-SHA256 of the raw body, `sha256=` prefixed.
pub const SIGNATURE_HEADER: &str = "X-Inkwell-Signature";
/// Header carrying the event name, mirroring the payload's `event`.
pub const EVENT_HEADER: &str = "X-Inkwell-Event";
/// Header carrying the delivery id, mirroring the payload's `deliveryId`.
pub const DELIVERY_HEADER: &str = "X-Inkwell-Delivery";
/// Header carrying Unix seconds, mirroring the payload's `timestamp`.
pub const TIMESTAMP_HEADER: &str = "X-Inkwell-Timestamp";

type HmacSha256 = Hmac<Sha256>;

/// The document lifecycle transitions that emit a webhook.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Event {
    Published,
    Unpublished,
}

impl Event {
    /// Wire name, also used as the bounded `event` metric label.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Published => "document.published",
            Self::Unpublished => "document.unpublished",
        }
    }
}

/// Compute the signature header value for `body`: `sha256=<lowercase hex>` of
/// HMAC-SHA256(secret, body). Receivers must compare in constant time.
pub fn sign(secret: &str, body: &[u8]) -> String {
    // HMAC accepts a key of any length, so this cannot fail for any `&str`.
    let mut mac =
        HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC accepts keys of any length");
    mac.update(body);
    format!("sha256={}", hex::encode(mac.finalize().into_bytes()))
}

/// Build the JSON payload for one delivery.
///
/// `timestamp` appears **inside** the signed body as well as in the
/// `X-Inkwell-Timestamp` header, so a receiver's replay window is covered by the
/// signature: an attacker replaying an old body cannot refresh its timestamp
/// without invalidating the digest.
pub fn payload(
    event: Event,
    document: &Document,
    site_base_url: &str,
    delivery_id: Uuid,
    now: OffsetDateTime,
) -> Value {
    json!({
        "version": PAYLOAD_VERSION,
        "event": event.as_str(),
        "deliveryId": delivery_id,
        "timestamp": timestamp::serialize_to_string(&now),
        "document": {
            "id": document.id,
            "slug": document.slug,
            "title": document.title,
            "status": document.status.as_str(),
            "growth": document.growth.as_str(),
            // Inkwell has no separate "collections" concept: tags are the
            // grouping primitive, and `growth` carries the garden stage.
            "tags": document.tags,
            "url": public_url(site_base_url, &document.slug),
            "createdAt": timestamp::serialize_to_string(&document.created_at),
            "updatedAt": timestamp::serialize_to_string(&document.updated_at),
        }
    })
}

/// Canonical public URL of a note: the public route is `/{slug}`.
fn public_url(site_base_url: &str, slug: &str) -> String {
    format!("{}/{}", site_base_url.trim_end_matches('/'), slug)
}

/// Fire webhooks for `event` on `document`. **Inert** unless webhooks are fully
/// configured (see [`Config::webhooks_active`]). Returns immediately either way:
/// delivery happens in a detached task.
pub fn maybe_dispatch(state: &AppState, event: Event, document: &Document) {
    if !state.config.webhooks_active() {
        // Flag off (or unconfigured) ⇒ fully inert. No payload, no task.
        return;
    }
    let Some(secret) = state.config.webhook_secret.clone() else {
        // Unreachable while `webhooks_active` requires a secret; belt-and-braces
        // so an unsigned delivery can never be sent.
        return;
    };

    let base_url = crate::views::layout::normalize_site_url(state.config.site_url.as_deref());
    let delivery_id = Uuid::new_v4();
    let body = payload(
        event,
        document,
        &base_url,
        delivery_id,
        OffsetDateTime::now_utc(),
    );
    // Serialize once: every endpoint receives byte-identical bytes, so one
    // signature covers them all and receivers can re-hash exactly what arrived.
    let body = match serde_json::to_vec(&body) {
        Ok(bytes) => bytes,
        Err(error) => {
            tracing::error!(%error, event = event.as_str(), "could not serialize webhook payload; dropping");
            return;
        }
    };
    let signature = sign(&secret, &body);
    let urls = state.config.webhook_urls.clone();
    let metrics = state.metrics.clone();
    let unix_seconds = OffsetDateTime::now_utc().unix_timestamp();

    tokio::spawn(async move {
        for url in urls {
            deliver_with_retries(
                &metrics,
                event,
                &url,
                &body,
                &signature,
                delivery_id,
                unix_seconds,
            )
            .await;
        }
    });
}

/// Whether a response status should be retried. 5xx and 429 are transient; every
/// other non-2xx status is a receiver-side rejection that a retry won't fix.
fn is_retryable_status(status: reqwest::StatusCode) -> bool {
    status.is_server_error() || status == reqwest::StatusCode::TOO_MANY_REQUESTS
}

/// POST one delivery to one endpoint, retrying transient failures up to
/// [`MAX_ATTEMPTS`] times. Always returns `()`: a permanently failed delivery is
/// counted and logged, never propagated.
async fn deliver_with_retries(
    metrics: &Metrics,
    event: Event,
    url: &str,
    body: &[u8],
    signature: &str,
    delivery_id: Uuid,
    unix_seconds: i64,
) {
    let client = match reqwest::Client::builder()
        .timeout(ATTEMPT_TIMEOUT)
        .user_agent(USER_AGENT)
        .build()
    {
        Ok(client) => client,
        Err(error) => {
            tracing::error!(%error, "could not build webhook HTTP client; dropping delivery");
            metrics.record_webhook_delivery(event.as_str(), false);
            return;
        }
    };

    for attempt in 1..=MAX_ATTEMPTS {
        let result = client
            .post(url)
            .header("content-type", "application/json")
            .header(EVENT_HEADER, event.as_str())
            .header(DELIVERY_HEADER, delivery_id.to_string())
            .header(TIMESTAMP_HEADER, unix_seconds.to_string())
            .header(SIGNATURE_HEADER, signature)
            .body(body.to_vec())
            .send()
            .await;

        let retryable = match result {
            Ok(response) if response.status().is_success() => {
                metrics.record_webhook_attempt(event.as_str(), true);
                metrics.record_webhook_delivery(event.as_str(), true);
                tracing::info!(
                    event = event.as_str(),
                    %delivery_id,
                    endpoint = %url,
                    attempt,
                    status = response.status().as_u16(),
                    "webhook delivered"
                );
                return;
            }
            Ok(response) => {
                let status = response.status();
                metrics.record_webhook_attempt(event.as_str(), false);
                tracing::warn!(
                    event = event.as_str(),
                    %delivery_id,
                    endpoint = %url,
                    attempt,
                    status = status.as_u16(),
                    "webhook attempt rejected"
                );
                is_retryable_status(status)
            }
            Err(error) => {
                metrics.record_webhook_attempt(event.as_str(), false);
                tracing::warn!(
                    event = event.as_str(),
                    %delivery_id,
                    endpoint = %url,
                    attempt,
                    // `error` renders the URL and cause, never the secret.
                    %error,
                    "webhook attempt failed"
                );
                // Transport errors (timeout, connection refused, DNS) are all
                // worth one more try.
                true
            }
        };

        if !retryable {
            break;
        }
        if let Some(delay) = RETRY_BACKOFF.get(attempt - 1) {
            tokio::time::sleep(*delay).await;
        }
    }

    metrics.record_webhook_delivery(event.as_str(), false);
    tracing::warn!(
        event = event.as_str(),
        %delivery_id,
        endpoint = %url,
        attempts = MAX_ATTEMPTS,
        "webhook delivery gave up"
    );
}

/// Validate one operator-configured endpoint URL. Returns the reason it is
/// unusable, or `None` when it is fine. Used by [`Config::from_env`] so a typo
/// fails startup instead of silently never delivering.
pub fn endpoint_url_problem(url: &str) -> Option<String> {
    match reqwest::Url::parse(url) {
        Ok(parsed) => {
            if !matches!(parsed.scheme(), "http" | "https") {
                return Some(format!(
                    "\"{url}\" must use http or https (got \"{}\")",
                    parsed.scheme()
                ));
            }
            if parsed.host_str().is_none() {
                return Some(format!("\"{url}\" has no host"));
            }
            None
        }
        Err(error) => Some(format!("\"{url}\" is not a valid URL ({error})")),
    }
}

/// Whether webhooks are configured well enough to deliver. Kept as a method on
/// [`Config`] so both startup validation and the dispatch path agree.
impl Config {
    pub fn webhooks_active(&self) -> bool {
        self.webhooks_enabled && self.webhook_secret.is_some() && !self.webhook_urls.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::document::{DocumentStatus, GrowthStage};

    /// Fixed vector so the documented algorithm is pinned. Cross-checked with
    /// `printf '%s' '{"a":1}' | openssl dgst -sha256 -hmac 'shhh'`.
    #[test]
    fn signature_matches_known_vector() {
        assert_eq!(
            sign("shhh", b"{\"a\":1}"),
            "sha256=82a2822723ef5d74e78b2082b74ec3369cc9cf94e58ed4dc61f5c1e2887fd7c7"
        );
    }

    #[test]
    fn signature_changes_with_secret_and_body() {
        let a = sign("secret-one", b"payload");
        let b = sign("secret-two", b"payload");
        let c = sign("secret-one", b"payload!");
        assert_ne!(a, b);
        assert_ne!(a, c);
        assert!(a.starts_with("sha256="));
        // Hex digest of SHA-256 is 64 characters.
        assert_eq!(a.len(), "sha256=".len() + 64);
    }

    fn document() -> Document {
        let created = OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap();
        Document {
            id: Uuid::nil(),
            slug: "hello-world".to_string(),
            title: "Hello, world".to_string(),
            body_markdown: "body".to_string(),
            rendered_html: "<p>body</p>".to_string(),
            status: DocumentStatus::Published,
            growth: GrowthStage::Seedling,
            tags: vec!["rust".to_string(), "notes".to_string()],
            version: 3,
            created_at: created,
            updated_at: created,
        }
    }

    #[test]
    fn payload_carries_the_documented_fields() {
        let now = OffsetDateTime::from_unix_timestamp(1_800_000_000).unwrap();
        let id = Uuid::nil();
        let body = payload(
            Event::Published,
            &document(),
            "https://blog.example.com",
            id,
            now,
        );

        assert_eq!(body["version"], 1);
        assert_eq!(body["event"], "document.published");
        assert_eq!(body["deliveryId"], id.to_string());
        // Millisecond-precision RFC 3339 in UTC, matching every other timestamp
        // Inkwell puts on the wire.
        assert_eq!(body["timestamp"], "2027-01-15T08:00:00.000Z");
        assert_eq!(body["document"]["slug"], "hello-world");
        assert_eq!(body["document"]["title"], "Hello, world");
        assert_eq!(body["document"]["status"], "published");
        assert_eq!(
            body["document"]["url"],
            "https://blog.example.com/hello-world"
        );
        assert_eq!(body["document"]["tags"][0], "rust");
    }

    #[test]
    fn unpublish_event_name_differs() {
        let body = payload(
            Event::Unpublished,
            &document(),
            "https://blog.example.com/",
            Uuid::nil(),
            OffsetDateTime::from_unix_timestamp(0).unwrap(),
        );
        assert_eq!(body["event"], "document.unpublished");
        // A trailing slash on the site URL must not double up in the note URL.
        assert_eq!(
            body["document"]["url"],
            "https://blog.example.com/hello-world"
        );
    }

    #[test]
    fn payload_never_contains_the_secret() {
        let body = payload(
            Event::Published,
            &document(),
            "https://blog.example.com",
            Uuid::nil(),
            OffsetDateTime::from_unix_timestamp(0).unwrap(),
        );
        let rendered = serde_json::to_string(&body).unwrap();
        assert!(!rendered.contains("sentinel-webhook-secret"));
        assert!(!rendered.to_lowercase().contains("secret"));
    }

    #[test]
    fn only_5xx_and_429_are_retried() {
        use reqwest::StatusCode;
        assert!(is_retryable_status(StatusCode::INTERNAL_SERVER_ERROR));
        assert!(is_retryable_status(StatusCode::BAD_GATEWAY));
        assert!(is_retryable_status(StatusCode::TOO_MANY_REQUESTS));
        assert!(!is_retryable_status(StatusCode::BAD_REQUEST));
        assert!(!is_retryable_status(StatusCode::UNAUTHORIZED));
        assert!(!is_retryable_status(StatusCode::NOT_FOUND));
        assert!(!is_retryable_status(StatusCode::GONE));
    }

    #[test]
    fn retry_schedule_matches_the_attempt_cap() {
        // One backoff between each pair of attempts, no more.
        assert_eq!(RETRY_BACKOFF.len(), MAX_ATTEMPTS - 1);
    }

    #[test]
    fn endpoint_urls_are_validated() {
        assert!(endpoint_url_problem("https://hooks.example.com/inkwell").is_none());
        assert!(endpoint_url_problem("http://localhost:9000/hook").is_none());
        assert!(endpoint_url_problem("ftp://example.com/hook").is_some());
        assert!(endpoint_url_problem("not a url").is_some());
        assert!(endpoint_url_problem("").is_some());
    }
}
