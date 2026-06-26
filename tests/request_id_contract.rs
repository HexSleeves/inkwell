//! Contract: request correlation ids (CIL-125).
//!
//! Asserts the observable behaviour of the `X-Request-Id` middleware:
//!   (a) a generated id appears on the response when the client sends none,
//!   (b) a well-formed inbound id is echoed back unchanged,
//!   (c) the id appears in the error response body (and matches the header) on a
//!       4xx path, so a user-reported error is traceable to its logs.
//! A malformed inbound id is rejected and replaced with a generated one.

mod common;

use axum::body::{Body, to_bytes};
use http::{Method, Request, StatusCode};
use std::sync::LazyLock;
use tokio::sync::{Mutex, MutexGuard};
use tower::ServiceExt;

// Shares the same single test database as the other contract suites; serialize
// so the truncation in `maybe_pool` cannot race a sibling test.
static DB_TEST_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

async fn db_guard() -> MutexGuard<'static, ()> {
    DB_TEST_LOCK.lock().await
}

#[tokio::test]
async fn generates_request_id_when_none_supplied() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(router) = common::maybe_router().await? else {
        return Ok(());
    };

    let response = router
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/health")
                .body(Body::empty())?,
        )
        .await?;

    let id = response
        .headers()
        .get("x-request-id")
        .expect("response carries an X-Request-Id header")
        .to_str()?
        .to_owned();
    // A freshly minted id is a parseable UUID.
    uuid::Uuid::parse_str(&id).expect("generated id is a valid UUID");

    Ok(())
}

#[tokio::test]
async fn echoes_well_formed_inbound_request_id() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(router) = common::maybe_router().await? else {
        return Ok(());
    };

    let sent = "trace-abc_123";
    let response = router
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/health")
                .header("x-request-id", sent)
                .body(Body::empty())?,
        )
        .await?;

    let echoed = response
        .headers()
        .get("x-request-id")
        .expect("response carries an X-Request-Id header")
        .to_str()?;
    assert_eq!(echoed, sent, "a well-formed inbound id is echoed unchanged");

    Ok(())
}

#[tokio::test]
async fn malformed_inbound_request_id_is_replaced() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(router) = common::maybe_router().await? else {
        return Ok(());
    };

    // Contains a space → rejected; the server mints a fresh UUID instead.
    let response = router
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/health")
                .header("x-request-id", "not a valid id")
                .body(Body::empty())?,
        )
        .await?;

    let id = response
        .headers()
        .get("x-request-id")
        .expect("response carries an X-Request-Id header")
        .to_str()?;
    assert_ne!(id, "not a valid id", "a malformed id must not be echoed");
    uuid::Uuid::parse_str(id).expect("replacement id is a valid UUID");

    Ok(())
}

#[tokio::test]
async fn error_body_carries_request_id_matching_the_header() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(router) = common::maybe_router().await? else {
        return Ok(());
    };

    // Unauthenticated write → 401 with the JSON error envelope. This is the
    // "user-reported error" path: the id in the body must match the header so
    // the report ties back to the logs.
    let sent = "user-report-42";
    let response = router
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/documents")
                .header("content-type", "application/json")
                .header("x-request-id", sent)
                .body(Body::from(r##"{"title":"x","bodyMarkdown":"x"}"##))?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    let header_id = response
        .headers()
        .get("x-request-id")
        .expect("error response carries an X-Request-Id header")
        .to_str()?
        .to_owned();
    assert_eq!(header_id, sent);

    // The error envelope is a tiny JSON object; cap the read so a regressed
    // handler can't turn this assertion into an unbounded allocation.
    let body = to_bytes(response.into_body(), 64 * 1024).await?;
    let json: serde_json::Value = serde_json::from_slice(&body)?;
    assert_eq!(
        json["error"]["requestId"], sent,
        "the error envelope includes the correlation id"
    );
    assert_eq!(
        json["error"]["requestId"], header_id,
        "body id and header id agree, so a report traces to one request"
    );

    Ok(())
}
