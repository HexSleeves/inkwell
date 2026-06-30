//! Contract tests for the `/settings` "About this garden" page. DB-gated like
//! the other contract suites (skips when `DATABASE_URL` is unset).
//!
//! Covers the two handler concerns that the pure-render view tests cannot: the
//! `Cache-Control: no-store` response (the page reflects per-request auth state,
//! so it must never be shared-cached) and the `INKWELL_BROWSER_LOGIN` gating of
//! the account panel.

mod common;

use std::sync::Arc;

use axum::body::{Body, to_bytes};
use http::{Method, Request, StatusCode, header};
use inkwell::http::router::build_router;
use std::sync::LazyLock;
use tokio::sync::{Mutex, MutexGuard};
use tower::ServiceExt;

const SHARED_KEY: &str = "test-secret-key";

static DB_TEST_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

async fn db_guard() -> MutexGuard<'static, ()> {
    DB_TEST_LOCK.lock().await
}

fn header_str(response: &http::Response<Body>, name: header::HeaderName) -> String {
    response
        .headers()
        .get(name)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_string()
}

/// Build a router with `INKWELL_BROWSER_LOGIN` on, over an already-acquired pool.
fn browser_login_router(pool: sqlx::PgPool) -> axum::Router {
    let database_url = std::env::var("DATABASE_URL").unwrap_or_default();
    let mut config = (*common::test_config(database_url)).clone();
    config.browser_login = true;
    build_router(Arc::new(config), pool)
}

async fn create_published_note(
    router: &axum::Router,
    title: &str,
    slug: &str,
) -> anyhow::Result<()> {
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/documents")
                .header("content-type", "application/json")
                .header("x-api-key", SHARED_KEY)
                .body(Body::from(format!(
                    r##"{{"title":"{title}","bodyMarkdown":"# {title}"}}"##
                )))?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::CREATED);
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri(format!("/documents/{slug}/publish"))
                .header("x-api-key", SHARED_KEY)
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    Ok(())
}

#[tokio::test]
async fn settings_is_no_store_html_with_stats_and_no_panel_when_login_off() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(router) = common::maybe_router().await? else {
        return Ok(());
    };
    create_published_note(&router, "Hello Garden", "hello-garden").await?;

    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/settings")
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    assert!(header_str(&response, header::CONTENT_TYPE).starts_with("text/html"));

    // Must never be shared-cached: the page reflects per-request auth state.
    let cache_control = header_str(&response, header::CACHE_CONTROL);
    assert!(
        cache_control.contains("no-store"),
        "settings must be no-store, got {cache_control:?}"
    );
    assert!(!cache_control.contains("public"));

    let body = to_bytes(response.into_body(), usize::MAX).await?;
    let html = String::from_utf8(body.to_vec())?;
    assert!(html.contains("About this garden"));
    assert!(html.contains("Published notes"));
    assert!(html.contains("Internal links"));
    // Browser login is off, so the account panel is omitted entirely.
    assert!(!html.contains("account-panel"));
    assert!(!html.contains("Your account"));
    Ok(())
}

#[tokio::test]
async fn settings_shows_login_link_when_login_on_and_anonymous() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };
    let router = browser_login_router(pool);

    let response = router
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/settings")
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    assert!(header_str(&response, header::CACHE_CONTROL).contains("no-store"));

    let body = to_bytes(response.into_body(), usize::MAX).await?;
    let html = String::from_utf8(body.to_vec())?;
    assert!(
        html.contains("Your account"),
        "panel is shown when login is on"
    );
    assert!(
        html.contains(r#"href="/login""#),
        "anonymous visitor gets a login link"
    );
    assert!(
        !html.contains("Signed in as"),
        "anonymous visitor is not signed in"
    );
    Ok(())
}
