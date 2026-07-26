//! Outbound signed webhooks on publish / unpublish (CYP-53).
//!
//! Every test here stands up a **real receiver** on loopback rather than mocking
//! the HTTP client, because the properties that matter are wire-level: exactly
//! one delivery per event, a signature a third party can verify, and — above all
//! — a publish that neither fails nor slows down when the receiver misbehaves.

mod common;

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::body::{Body, Bytes, to_bytes};
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode as AxumStatus};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use http::{Method, Request, StatusCode};
use serde_json::{Value, json};
use std::sync::LazyLock;
use tokio::sync::{Mutex as AsyncMutex, MutexGuard};
use tokio::task::JoinHandle;
use tower::ServiceExt;

const SHARED_KEY: &str = "test-secret-key";
/// At least `MIN_WEBHOOK_SECRET_LEN` characters, matching what startup enforces.
const WEBHOOK_SECRET: &str = "test-webhook-secret-value";

/// These tests share one database (`maybe_pool` truncates on entry), so they must
/// not run concurrently within this binary. See `tests/api_contract.rs`.
static DB_TEST_LOCK: LazyLock<AsyncMutex<()>> = LazyLock::new(|| AsyncMutex::new(()));

async fn db_guard() -> MutexGuard<'static, ()> {
    DB_TEST_LOCK.lock().await
}

// ---------------------------------------------------------------------------
// Mock receiver
// ---------------------------------------------------------------------------

/// One delivery as it arrived on the wire.
#[derive(Clone, Debug)]
struct Delivery {
    event: String,
    delivery_id: String,
    timestamp: String,
    signature: String,
    content_type: String,
    body: Vec<u8>,
}

impl Delivery {
    fn json(&self) -> Value {
        serde_json::from_slice(&self.body).expect("delivery body is JSON")
    }
}

/// How the receiver answers. Each variant models a failure mode the publish path
/// must survive.
#[derive(Clone, Copy, Debug)]
enum Behavior {
    /// 204: a healthy receiver.
    Accept,
    /// 500 forever: transient-looking failure, so deliveries retry to the cap.
    ServerError,
    /// 400 forever: a permanent rejection that must NOT be retried.
    BadRequest,
    /// Never answers: exercises the per-attempt timeout without waiting for it.
    Hang,
}

#[derive(Clone)]
struct ReceiverState {
    behavior: Behavior,
    deliveries: Arc<Mutex<Vec<Delivery>>>,
}

/// A live receiver bound to an ephemeral loopback port.
struct Receiver {
    url: String,
    deliveries: Arc<Mutex<Vec<Delivery>>>,
    server: JoinHandle<()>,
}

impl Receiver {
    async fn start(behavior: Behavior) -> anyhow::Result<Self> {
        let deliveries = Arc::new(Mutex::new(Vec::new()));
        let state = ReceiverState {
            behavior,
            deliveries: deliveries.clone(),
        };
        let app = axum::Router::new()
            .route("/hook", post(receive))
            .with_state(state);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let addr: SocketAddr = listener.local_addr()?;
        let server = tokio::spawn(async move {
            // Ignored: the test aborts this task, which surfaces as a shutdown.
            let _ = axum::serve(listener, app).await;
        });
        Ok(Self {
            url: format!("http://{addr}/hook"),
            deliveries,
            server,
        })
    }

    fn count(&self) -> usize {
        self.deliveries.lock().unwrap().len()
    }

    fn all(&self) -> Vec<Delivery> {
        self.deliveries.lock().unwrap().clone()
    }

    /// Wait until at least `n` deliveries have arrived, or `budget` elapses.
    /// Returns whether the count was reached.
    async fn wait_for(&self, n: usize, budget: Duration) -> bool {
        let deadline = Instant::now() + budget;
        while Instant::now() < deadline {
            if self.count() >= n {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        self.count() >= n
    }
}

impl Drop for Receiver {
    fn drop(&mut self) {
        self.server.abort();
    }
}

async fn receive(State(state): State<ReceiverState>, headers: HeaderMap, body: Bytes) -> Response {
    let header = |name: &str| {
        headers
            .get(name)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_string()
    };
    state.deliveries.lock().unwrap().push(Delivery {
        event: header("x-inkwell-event"),
        delivery_id: header("x-inkwell-delivery"),
        timestamp: header("x-inkwell-timestamp"),
        signature: header("x-inkwell-signature"),
        content_type: header("content-type"),
        body: body.to_vec(),
    });
    match state.behavior {
        Behavior::Accept => AxumStatus::NO_CONTENT.into_response(),
        Behavior::ServerError => AxumStatus::INTERNAL_SERVER_ERROR.into_response(),
        Behavior::BadRequest => AxumStatus::BAD_REQUEST.into_response(),
        Behavior::Hang => {
            // Longer than the per-attempt timeout; the task is aborted on drop.
            tokio::time::sleep(Duration::from_secs(60)).await;
            AxumStatus::NO_CONTENT.into_response()
        }
    }
}

// ---------------------------------------------------------------------------
// Inkwell-side helpers
// ---------------------------------------------------------------------------

async fn create_note(router: &axum::Router, title: &str) -> anyhow::Result<()> {
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/documents")
                .header("content-type", "application/json")
                .header("x-api-key", SHARED_KEY)
                .body(Body::from(
                    json!({"title": title, "bodyMarkdown": "Hello from a webhook test.", "tags": ["rust"]})
                        .to_string(),
                ))?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::CREATED);
    Ok(())
}

/// POST `/documents/{slug}/{action}` and return the status plus how long the
/// request took, so a test can prove delivery never blocks the caller.
async fn transition(
    router: &axum::Router,
    slug: &str,
    action: &str,
) -> anyhow::Result<(StatusCode, Duration)> {
    let started = Instant::now();
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri(format!("/documents/{slug}/{action}"))
                .header("x-api-key", SHARED_KEY)
                .body(Body::empty())?,
        )
        .await?;
    Ok((response.status(), started.elapsed()))
}

async fn metrics_body(router: &axum::Router) -> anyhow::Result<String> {
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/metrics")
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await?;
    Ok(String::from_utf8(bytes.to_vec())?)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Publish and unpublish each deliver exactly one signed webhook, and the payload
/// carries what the docs promise.
#[tokio::test]
async fn publish_and_unpublish_each_deliver_one_signed_webhook() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };
    let receiver = Receiver::start(Behavior::Accept).await?;
    let router = common::router_for_with_webhooks(pool, &[&receiver.url], WEBHOOK_SECRET);

    create_note(&router, "Webhook Note").await?;

    let (status, _) = transition(&router, "webhook-note", "publish").await?;
    assert_eq!(status, StatusCode::OK);
    assert!(
        receiver.wait_for(1, Duration::from_secs(5)).await,
        "publish did not deliver a webhook"
    );

    let (status, _) = transition(&router, "webhook-note", "unpublish").await?;
    assert_eq!(status, StatusCode::OK);
    assert!(
        receiver.wait_for(2, Duration::from_secs(5)).await,
        "unpublish did not deliver a webhook"
    );

    // Exactly one delivery per event — no duplicates from the retry loop.
    tokio::time::sleep(Duration::from_millis(200)).await;
    let deliveries = receiver.all();
    assert_eq!(deliveries.len(), 2, "expected exactly two deliveries");

    let published = &deliveries[0];
    let unpublished = &deliveries[1];
    assert_eq!(published.event, "document.published");
    assert_eq!(unpublished.event, "document.unpublished");
    assert_ne!(
        published.delivery_id, unpublished.delivery_id,
        "each delivery needs its own id"
    );

    for delivery in &deliveries {
        assert_eq!(delivery.content_type, "application/json");
        // The signature verifies against the documented algorithm over the raw body.
        assert_eq!(
            delivery.signature,
            inkwell::webhooks::sign(WEBHOOK_SECRET, &delivery.body),
            "signature does not verify"
        );
        // ...and does NOT verify under a different secret.
        assert_ne!(
            delivery.signature,
            inkwell::webhooks::sign("some-other-secret-value", &delivery.body)
        );

        let body = delivery.json();
        assert_eq!(body["version"], 1);
        assert_eq!(body["event"], delivery.event.as_str());
        assert_eq!(body["deliveryId"], delivery.delivery_id.as_str());
        assert_eq!(body["document"]["slug"], "webhook-note");
        assert_eq!(body["document"]["title"], "Webhook Note");
        assert_eq!(body["document"]["tags"][0], "rust");
        assert_eq!(
            body["document"]["url"],
            "https://blog.example.com/webhook-note"
        );
        assert!(body["document"]["id"].is_string());

        // The timestamp header mirrors the signed body timestamp, so a receiver
        // can reject replays using a value the signature actually covers.
        let header_seconds: i64 = delivery.timestamp.parse()?;
        let body_time = time::OffsetDateTime::parse(
            body["timestamp"].as_str().expect("timestamp is a string"),
            &time::format_description::well_known::Rfc3339,
        )?;
        assert!(
            (header_seconds - body_time.unix_timestamp()).abs() <= 2,
            "header timestamp {header_seconds} disagrees with body {body_time}"
        );

        // No secret anywhere on the wire.
        let raw = String::from_utf8_lossy(&delivery.body);
        assert!(!raw.contains(WEBHOOK_SECRET));
        assert!(!delivery.signature.contains(WEBHOOK_SECRET));
    }

    // Both deliveries are visible in /metrics: one attempt and one delivery each.
    let metrics = metrics_body(&router).await?;
    assert!(metrics.contains(
        "inkwell_webhook_deliveries_total{event=\"document.published\",result=\"success\"} 1"
    ));
    assert!(metrics.contains(
        "inkwell_webhook_deliveries_total{event=\"document.unpublished\",result=\"success\"} 1"
    ));
    assert!(metrics.contains(
        "inkwell_webhook_attempts_total{event=\"document.published\",result=\"success\"} 1"
    ));
    Ok(())
}

/// With the flag off (the shared test config default), the delivery path is inert:
/// a configured receiver hears nothing at all.
#[tokio::test]
async fn disabled_webhooks_deliver_nothing() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };
    let receiver = Receiver::start(Behavior::Accept).await?;
    // Default router: webhooks unconfigured and disabled.
    let router = common::router_for(pool);

    create_note(&router, "Silent Note").await?;
    let (status, _) = transition(&router, "silent-note", "publish").await?;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = transition(&router, "silent-note", "unpublish").await?;
    assert_eq!(status, StatusCode::OK);

    // Generously longer than a delivery would need.
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(
        receiver.count(),
        0,
        "webhooks fired while INKWELL_WEBHOOKS_ENABLED was off"
    );
    Ok(())
}

/// A receiver that always 500s must not fail or delay the publish, and the
/// delivery must stop at the retry cap instead of hammering forever.
#[tokio::test]
async fn failing_receiver_neither_fails_nor_delays_publish() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };
    let receiver = Receiver::start(Behavior::ServerError).await?;
    let router = common::router_for_with_webhooks(pool, &[&receiver.url], WEBHOOK_SECRET);

    create_note(&router, "Broken Receiver").await?;
    let (status, elapsed) = transition(&router, "broken-receiver", "publish").await?;
    assert_eq!(
        status,
        StatusCode::OK,
        "publish failed because of a webhook"
    );
    assert!(
        elapsed < Duration::from_secs(1),
        "publish waited {elapsed:?} on a failing receiver"
    );

    // Retries happen, then stop: exactly MAX_ATTEMPTS attempts, no more.
    let attempts = inkwell::webhooks::MAX_ATTEMPTS;
    assert!(
        receiver.wait_for(attempts, Duration::from_secs(10)).await,
        "expected {attempts} attempts, saw {}",
        receiver.count()
    );
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(
        receiver.count(),
        attempts,
        "delivery exceeded the retry cap of {attempts}"
    );

    // Every attempt carries the same delivery id, so a receiver can dedupe.
    let deliveries = receiver.all();
    let first_id = deliveries[0].delivery_id.clone();
    assert!(
        deliveries.iter().all(|d| d.delivery_id == first_id),
        "retries changed the delivery id"
    );

    let metrics = metrics_body(&router).await?;
    assert!(metrics.contains(&format!(
        "inkwell_webhook_attempts_total{{event=\"document.published\",result=\"failure\"}} {attempts}"
    )));
    assert!(metrics.contains(
        "inkwell_webhook_deliveries_total{event=\"document.published\",result=\"failure\"} 1"
    ));
    Ok(())
}

/// A receiver that never answers must not hold up the publish either. The
/// per-attempt timeout bounds the detached task; the caller never waits on it.
#[tokio::test]
async fn hanging_receiver_does_not_delay_publish() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };
    let receiver = Receiver::start(Behavior::Hang).await?;
    let router = common::router_for_with_webhooks(pool, &[&receiver.url], WEBHOOK_SECRET);

    create_note(&router, "Hanging Receiver").await?;
    let (status, elapsed) = transition(&router, "hanging-receiver", "publish").await?;
    assert_eq!(status, StatusCode::OK);
    assert!(
        elapsed < Duration::from_secs(1),
        "publish waited {elapsed:?} on a hanging receiver"
    );
    // The attempt did leave the process — the timeout applies to it, not to us.
    assert!(
        receiver.wait_for(1, Duration::from_secs(5)).await,
        "no attempt reached the hanging receiver"
    );
    // Deliberately does NOT wait out the retries: the point is that nothing here
    // depends on them. The detached task dies with the runtime.
    Ok(())
}

/// A permanent rejection (4xx) is not retried — retrying a 400 forever is just
/// noise for the receiver.
#[tokio::test]
async fn permanent_rejection_is_not_retried() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };
    let receiver = Receiver::start(Behavior::BadRequest).await?;
    let router = common::router_for_with_webhooks(pool, &[&receiver.url], WEBHOOK_SECRET);

    create_note(&router, "Rejecting Receiver").await?;
    let (status, _) = transition(&router, "rejecting-receiver", "publish").await?;
    assert_eq!(status, StatusCode::OK);

    assert!(receiver.wait_for(1, Duration::from_secs(5)).await);
    // Long enough that a retry would have arrived (backoff is 250ms then 750ms).
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert_eq!(receiver.count(), 1, "a 4xx rejection was retried");

    let metrics = metrics_body(&router).await?;
    assert!(metrics.contains(
        "inkwell_webhook_deliveries_total{event=\"document.published\",result=\"failure\"} 1"
    ));
    Ok(())
}

/// Two endpoints each get their own delivery of the same event, byte-identical
/// body and signature.
#[tokio::test]
async fn every_configured_endpoint_receives_the_event() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };
    let first = Receiver::start(Behavior::Accept).await?;
    let second = Receiver::start(Behavior::Accept).await?;
    let router = common::router_for_with_webhooks(pool, &[&first.url, &second.url], WEBHOOK_SECRET);

    create_note(&router, "Fanout Note").await?;
    let (status, _) = transition(&router, "fanout-note", "publish").await?;
    assert_eq!(status, StatusCode::OK);

    assert!(first.wait_for(1, Duration::from_secs(5)).await);
    assert!(second.wait_for(1, Duration::from_secs(5)).await);
    let a = first.all().remove(0);
    let b = second.all().remove(0);
    assert_eq!(a.body, b.body, "endpoints received different bodies");
    assert_eq!(a.signature, b.signature);
    assert_eq!(a.delivery_id, b.delivery_id);
    Ok(())
}
