//! Contract tests for the server-rendered browser login UI page (ADR 0010).
//!
//! These complement `tests/browser_login.rs` (which covers the JSON
//! login/logout backend) by exercising the new `GET /login` page and a single
//! end-to-end UI round-trip:
//!
//! **Flag OFF (default `test_config` via `common::router_for`):**
//! - `GET /login` is a 404 — the page route is not registered, so the request
//!   falls through to the `/{slug}` document route, which 404s for that slug.
//!
//! **Flag ON (cloned `test_config` with `browser_login: true`):**
//! - `GET /login` is 200 `text/html`, contains the token input and the
//!   `/auth/login` form target, and carries the nonce'd CSP header.
//! - A login → write → logout round-trip works behind the flag: minting a
//!   write-scoped token, exchanging it at `POST /auth/login`, using the session
//!   cookie on a write, then `POST /auth/logout` invalidates that cookie (401).
//!
//! Skipped unless `DATABASE_URL` is set (or forced via
//! `INKWELL_REQUIRE_DB_TESTS=1`).

mod common;

use axum::body::{Body, to_bytes};
use http::{Method, Request, StatusCode};
use inkwell::http::router::build_router;
use std::sync::{Arc, LazyLock};
use tokio::sync::{Mutex, MutexGuard};
use tower::ServiceExt;

/// Serialize all DB-backed tests in this binary (one shared pool, truncated per
/// test in `maybe_pool`).
static DB_TEST_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

async fn db_guard() -> MutexGuard<'static, ()> {
    DB_TEST_LOCK.lock().await
}

/// Shared admin key (matches `common::test_config`).
const SHARED_KEY: &str = "test-secret-key";

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Build a router with `browser_login: true` by cloning the shared test config,
/// sharing the pool from `maybe_pool`.
fn browser_login_router(pool: sqlx::PgPool) -> axum::Router {
    let database_url = std::env::var("DATABASE_URL").unwrap_or_default();
    let mut config = (*common::test_config(database_url)).clone();
    config.browser_login = true;
    build_router(Arc::new(config), pool)
}

async fn body_string(response: axum::response::Response) -> anyhow::Result<String> {
    let bytes = to_bytes(response.into_body(), usize::MAX).await?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// Mint a scoped token via the admin surface. Returns the full token string.
async fn mint_token(router: &axum::Router, name: &str, scopes: &[&str]) -> anyhow::Result<String> {
    let payload = serde_json::json!({ "name": name, "scopes": scopes });
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/admin/tokens")
                .header("content-type", "application/json")
                .header("x-api-key", SHARED_KEY)
                .body(Body::from(serde_json::to_vec(&payload)?))?,
        )
        .await?;
    assert_eq!(
        response.status(),
        StatusCode::CREATED,
        "mint should succeed"
    );
    let bytes = to_bytes(response.into_body(), usize::MAX).await?;
    let json: serde_json::Value = serde_json::from_slice(&bytes)?;
    Ok(json["token"]
        .as_str()
        .expect("token in response")
        .to_string())
}

/// Extract the `Set-Cookie` header value from a response, or `None`.
fn get_set_cookie(response: &axum::response::Response) -> Option<String> {
    response
        .headers()
        .get(http::header::SET_COOKIE)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string)
}

/// Extract the raw session token from a `Set-Cookie: inkwell_session=<token>; …`.
fn parse_session_token(set_cookie: &str) -> Option<String> {
    set_cookie
        .split(';')
        .next()?
        .strip_prefix("inkwell_session=")
        .map(str::to_string)
}

// ---------------------------------------------------------------------------
// (a) FLAG OFF — the page route is not registered
// ---------------------------------------------------------------------------

/// With the flag off, `GET /login` is a 404: the page route is not registered,
/// so the request falls through to the `/{slug}` document route which has no
/// such document.
#[tokio::test]
async fn flag_off_login_page_is_404() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };
    let router = common::router_for(pool);

    let response = router
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/login")
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(
        response.status(),
        StatusCode::NOT_FOUND,
        "GET /login must be 404 when the browser-login flag is off"
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// (b) FLAG ON — the page renders with the form and the nonce'd CSP
// ---------------------------------------------------------------------------

/// With the flag on, `GET /login` returns a 200 `text/html` page that contains
/// the token input and the `/auth/login` form target, and carries the nonce'd
/// CSP header from the security-headers middleware.
#[tokio::test]
async fn flag_on_login_page_renders_form_with_nonced_csp() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };
    let router = browser_login_router(pool);

    let response = router
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/login")
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::OK, "GET /login must be 200");

    let content_type = response
        .headers()
        .get(http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    assert!(
        content_type.starts_with("text/html"),
        "login page must be text/html; got: {content_type}"
    );

    // The CSP must carry a script nonce so the inline form script is allowed.
    let csp = response
        .headers()
        .get(http::header::CONTENT_SECURITY_POLICY)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    assert!(
        csp.contains("nonce-"),
        "login page CSP must carry a script nonce; got: {csp}"
    );

    let body = body_string(response).await?;
    assert!(
        body.contains(r#"id="token""#) && body.contains(r#"name="token""#),
        "login page must render the token input field"
    );
    assert!(
        body.contains("/auth/login"),
        "login page must target the JSON /auth/login endpoint"
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// (c) FLAG ON — login → write → logout round-trip
// ---------------------------------------------------------------------------

/// Behind the flag: mint a write-scoped token, exchange it for a session cookie,
/// use the cookie on a write (proving the session reaches an authenticated
/// action), then log out and confirm the same cookie no longer authenticates.
#[tokio::test]
async fn flag_on_login_logout_round_trip() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };
    let router = browser_login_router(pool);

    let token = mint_token(&router, "ui-roundtrip-author", &["read", "write"]).await?;

    // POST /auth/login with the minted token → 200 + Set-Cookie inkwell_session.
    let login_payload = serde_json::json!({ "token": token });
    let login = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/auth/login")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&login_payload)?))?,
        )
        .await?;
    assert_eq!(login.status(), StatusCode::OK, "login must return 200");

    let set_cookie = get_set_cookie(&login).expect("Set-Cookie on login");
    assert!(
        set_cookie.contains("inkwell_session="),
        "login must set the inkwell_session cookie; got: {set_cookie}"
    );
    let session_token = parse_session_token(&set_cookie).expect("session token");
    let cookie_header = format!("inkwell_session={session_token}");

    // Use the session cookie on a write request → the session authenticates it.
    let doc = serde_json::json!({ "title": "ui session doc", "bodyMarkdown": "# hi" });
    let create = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/documents")
                .header("content-type", "application/json")
                .header("cookie", &cookie_header)
                .body(Body::from(serde_json::to_vec(&doc)?))?,
        )
        .await?;
    assert_eq!(
        create.status(),
        StatusCode::CREATED,
        "the session cookie must authenticate a write"
    );

    // POST /auth/logout → 200 and the cookie is cleared.
    let logout = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/auth/logout")
                .header("cookie", &cookie_header)
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(logout.status(), StatusCode::OK, "logout must return 200");
    let clear = get_set_cookie(&logout).expect("Set-Cookie on logout");
    assert!(
        clear.contains("Max-Age=0") || clear.contains("inkwell_session=;"),
        "logout must clear the cookie; got: {clear}"
    );

    // The same cookie must no longer authenticate a write.
    let doc2 = serde_json::json!({ "title": "post-logout doc", "bodyMarkdown": "# no" });
    let after = router
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/documents")
                .header("content-type", "application/json")
                .header("cookie", &cookie_header)
                .body(Body::from(serde_json::to_vec(&doc2)?))?,
        )
        .await?;
    assert_eq!(
        after.status(),
        StatusCode::UNAUTHORIZED,
        "the session cookie must be invalid after logout"
    );
    Ok(())
}
