//! Database-backed contract tests for flag-gated browser session login (ADR 0010).
//!
//! Two test groups:
//!
//! **Flag OFF (default `test_config`):**
//! - `POST /auth/login` and `POST /auth/logout` are 404 (routes not registered).
//! - Existing `x-api-key` authentication is unchanged.
//!
//! **Flag ON (custom config with `browser_login: true`):**
//! - Login with a valid scoped token → 200 + `Set-Cookie` with HttpOnly/Secure/SameSite=Strict.
//! - A request bearing that cookie authenticates (creates a document as the author).
//! - Logout deletes the session; the cookie no longer authenticates.
//! - Login with an invalid token → 401.
//! - Login with a revoked token → 401.
//! - The raw session token is never present in any response body.
//!
//! Skipped unless `DATABASE_URL` is set (or forced via `INKWELL_REQUIRE_DB_TESTS=1`).

mod common;

use axum::body::{Body, to_bytes};
use http::{Method, Request, StatusCode};
use inkwell::config::Config;
use inkwell::http::router::build_router;
use std::sync::{Arc, LazyLock};
use tokio::sync::{Mutex, MutexGuard};
use tower::ServiceExt;

/// Serialize all DB-backed tests in this binary.
static DB_TEST_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

async fn db_guard() -> MutexGuard<'static, ()> {
    DB_TEST_LOCK.lock().await
}

const SHARED_KEY: &str = "test-secret-key";

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

async fn body_json(response: axum::response::Response) -> anyhow::Result<serde_json::Value> {
    let bytes = to_bytes(response.into_body(), usize::MAX).await?;
    Ok(serde_json::from_slice(&bytes)?)
}

async fn body_bytes(response: axum::response::Response) -> anyhow::Result<Vec<u8>> {
    Ok(to_bytes(response.into_body(), usize::MAX).await?.to_vec())
}

/// Mint a scoped token via the admin surface. Returns (full_token, prefix).
async fn mint_token(
    router: &axum::Router,
    name: &str,
    scopes: &[&str],
) -> anyhow::Result<(String, String)> {
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
    let json = body_json(response).await?;
    let token = json["token"]
        .as_str()
        .expect("token in response")
        .to_string();
    let prefix = json["prefix"]
        .as_str()
        .expect("prefix in response")
        .to_string();
    Ok((token, prefix))
}

/// Revoke a token by prefix via the admin surface.
async fn revoke_token(router: &axum::Router, prefix: &str) -> anyhow::Result<()> {
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri(format!("/admin/tokens/{prefix}/revoke"))
                .header("x-api-key", SHARED_KEY)
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::OK, "revoke should succeed");
    Ok(())
}

/// POST /auth/login with the given token string. Returns the full response.
async fn do_login(router: &axum::Router, token: &str) -> anyhow::Result<axum::response::Response> {
    let payload = serde_json::json!({ "token": token });
    Ok(router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/auth/login")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&payload)?))?,
        )
        .await?)
}

/// Extract the Set-Cookie header value from a response, or None.
fn get_set_cookie(response: &axum::response::Response) -> Option<String> {
    response
        .headers()
        .get(http::header::SET_COOKIE)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string)
}

/// Extract the raw session token value from a `Set-Cookie` header like:
/// `inkwell_session=<token>; HttpOnly; ...`
fn parse_session_token(set_cookie: &str) -> Option<String> {
    let name_val = set_cookie.split(';').next()?;
    name_val
        .strip_prefix("inkwell_session=")
        .map(str::to_string)
}

/// Build a router with `browser_login: true`, sharing the pool from maybe_pool.
fn browser_login_router(pool: sqlx::PgPool) -> axum::Router {
    let database_url = std::env::var("DATABASE_URL").unwrap_or_default();
    build_router(
        Arc::new(Config {
            database_url,
            host: "127.0.0.1".to_string(),
            port: 3000,
            api_key: Some(SHARED_KEY.to_string()),
            site_url: Some("https://blog.example.com".to_string()),
            voyage_api_key: None,
            anthropic_api_key: None,
            llm_model: inkwell::config::DEFAULT_LLM_MODEL.to_string(),
            webmention_send: false,
            browser_login: true,
        }),
        pool,
    )
}

// ---------------------------------------------------------------------------
// FLAG OFF tests
// ---------------------------------------------------------------------------

/// With the flag off, /auth/login is 404 (the route is not registered).
#[tokio::test]
async fn flag_off_login_is_404() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(router) = common::maybe_router().await? else {
        return Ok(());
    };

    let payload = serde_json::json!({ "token": "ink_abc_def" });
    let response = router
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/auth/login")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&payload)?))?,
        )
        .await?;
    assert_eq!(
        response.status(),
        StatusCode::NOT_FOUND,
        "login route must not exist when flag is off"
    );
    Ok(())
}

/// With the flag off, /auth/logout is 404.
#[tokio::test]
async fn flag_off_logout_is_404() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(router) = common::maybe_router().await? else {
        return Ok(());
    };

    let response = router
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/auth/logout")
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(
        response.status(),
        StatusCode::NOT_FOUND,
        "logout route must not exist when flag is off"
    );
    Ok(())
}

/// With the flag off, the shared key still authenticates normally.
#[tokio::test]
async fn flag_off_existing_shared_key_auth_unchanged() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(router) = common::maybe_router().await? else {
        return Ok(());
    };

    // Creating a document with the shared key must still work.
    let payload = serde_json::json!({ "title": "flag-off test", "bodyMarkdown": "# hi" });
    let response = router
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/documents")
                .header("content-type", "application/json")
                .header("x-api-key", SHARED_KEY)
                .body(Body::from(serde_json::to_vec(&payload)?))?,
        )
        .await?;
    assert_eq!(
        response.status(),
        StatusCode::CREATED,
        "shared key must still create documents when flag is off"
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// FLAG ON tests
// ---------------------------------------------------------------------------

/// Login with a valid scoped token sets a session cookie with the required
/// security attributes: HttpOnly, Secure, SameSite=Strict.
#[tokio::test]
async fn flag_on_login_sets_secure_cookie() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };
    let router = browser_login_router(pool);

    let (token, _prefix) = mint_token(&router, "cookie-author", &["read", "write"]).await?;

    let response = do_login(&router, &token).await?;
    assert_eq!(response.status(), StatusCode::OK, "login must return 200");

    let set_cookie = get_set_cookie(&response).expect("Set-Cookie header must be present");
    // Verify all required security attributes.
    assert!(
        set_cookie.contains("inkwell_session="),
        "cookie must be named inkwell_session"
    );
    assert!(
        set_cookie.to_lowercase().contains("httponly"),
        "cookie must be HttpOnly; got: {set_cookie}"
    );
    assert!(
        set_cookie.to_lowercase().contains("secure"),
        "cookie must be Secure; got: {set_cookie}"
    );
    assert!(
        set_cookie.to_lowercase().contains("samesite=strict"),
        "cookie must be SameSite=Strict; got: {set_cookie}"
    );

    // The raw session token must NOT appear anywhere in the response body.
    let body = body_bytes(response).await?;
    let session_token = parse_session_token(&set_cookie).expect("session token in Set-Cookie");
    assert!(
        !body
            .windows(session_token.len())
            .any(|w| w == session_token.as_bytes()),
        "session token must not appear in response body"
    );

    Ok(())
}

/// A request bearing a valid session cookie authenticates as the owning author.
#[tokio::test]
async fn flag_on_session_cookie_authenticates() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };
    let router = browser_login_router(pool);

    let (token, _prefix) = mint_token(&router, "session-author", &["read", "write"]).await?;
    let response = do_login(&router, &token).await?;
    assert_eq!(response.status(), StatusCode::OK);

    let set_cookie = get_set_cookie(&response).expect("Set-Cookie header");
    let session_token = parse_session_token(&set_cookie).expect("session token");
    let cookie_header = format!("inkwell_session={session_token}");

    // Create a document using only the session cookie (no x-api-key).
    let payload = serde_json::json!({ "title": "session-auth doc", "bodyMarkdown": "# session" });
    let response = router
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/documents")
                .header("content-type", "application/json")
                .header("cookie", &cookie_header)
                .body(Body::from(serde_json::to_vec(&payload)?))?,
        )
        .await?;
    assert_eq!(
        response.status(),
        StatusCode::CREATED,
        "session cookie must authenticate document creation"
    );

    Ok(())
}

/// A session inherits ONLY its originating token's scopes — a read-only token's
/// session must not gain write/publish (no privilege escalation).
#[tokio::test]
async fn flag_on_read_only_session_cannot_write() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };
    let router = browser_login_router(pool);

    // Read-ONLY token → its session must stay read-only.
    let (token, _prefix) = mint_token(&router, "ro-author", &["read"]).await?;
    let response = do_login(&router, &token).await?;
    assert_eq!(response.status(), StatusCode::OK);
    let session_token =
        parse_session_token(&get_set_cookie(&response).expect("Set-Cookie")).expect("token");
    let cookie_header = format!("inkwell_session={session_token}");

    let payload = serde_json::json!({ "title": "should fail", "bodyMarkdown": "# x" });
    let response = router
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/documents")
                .header("content-type", "application/json")
                .header("cookie", &cookie_header)
                .body(Body::from(serde_json::to_vec(&payload)?))?,
        )
        .await?;
    assert_eq!(
        response.status(),
        StatusCode::FORBIDDEN,
        "a read-only token's session must NOT be able to create documents"
    );
    Ok(())
}

/// A present-but-malformed `x-api-key` (duplicated) must NOT fall through to the
/// session cookie — the key path takes full precedence.
#[tokio::test]
async fn flag_on_malformed_api_key_does_not_use_cookie() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };
    let router = browser_login_router(pool);

    let (token, _prefix) = mint_token(&router, "precedence-author", &["read", "write"]).await?;
    let session_token = parse_session_token(
        &get_set_cookie(&do_login(&router, &token).await?).expect("Set-Cookie"),
    )
    .expect("token");
    let cookie_header = format!("inkwell_session={session_token}");

    // Valid cookie BUT a duplicated x-api-key header present → reject via the key
    // path (the header's presence takes precedence); the cookie is not consulted.
    let payload = serde_json::json!({ "title": "via cookie?", "bodyMarkdown": "# x" });
    let response = router
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/documents")
                .header("content-type", "application/json")
                .header("x-api-key", "one")
                .header("x-api-key", "two")
                .header("cookie", &cookie_header)
                .body(Body::from(serde_json::to_vec(&payload)?))?,
        )
        .await?;
    assert_eq!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "a present (malformed) x-api-key must reject, not fall through to the cookie"
    );
    Ok(())
}

/// After logout, the session cookie no longer authenticates.
#[tokio::test]
async fn flag_on_logout_invalidates_session() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };
    let router = browser_login_router(pool);

    let (token, _prefix) = mint_token(&router, "logout-author", &["read", "write"]).await?;
    let login_response = do_login(&router, &token).await?;
    assert_eq!(login_response.status(), StatusCode::OK);

    let set_cookie = get_set_cookie(&login_response).expect("Set-Cookie after login");
    let session_token = parse_session_token(&set_cookie).expect("session token");
    let cookie_header = format!("inkwell_session={session_token}");

    // Logout.
    let logout_response = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/auth/logout")
                .header("cookie", &cookie_header)
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(
        logout_response.status(),
        StatusCode::OK,
        "logout must return 200"
    );

    // The logout Set-Cookie must clear the cookie (Max-Age=0 or empty value).
    let clear_cookie = get_set_cookie(&logout_response).expect("Set-Cookie on logout");
    assert!(
        clear_cookie.contains("Max-Age=0") || clear_cookie.contains("inkwell_session=;"),
        "logout must clear the cookie; got: {clear_cookie}"
    );

    // The same cookie must no longer authenticate.
    let payload = serde_json::json!({ "title": "post-logout doc", "bodyMarkdown": "# nope" });
    let response = router
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/documents")
                .header("content-type", "application/json")
                .header("cookie", &cookie_header)
                .body(Body::from(serde_json::to_vec(&payload)?))?,
        )
        .await?;
    assert_eq!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "session must be invalid after logout"
    );

    Ok(())
}

/// Login with an invalid / non-existent token returns 401.
#[tokio::test]
async fn flag_on_login_invalid_token_is_401() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };
    let router = browser_login_router(pool);

    // Completely invalid token (not ink_ format).
    let response = do_login(&router, "not-a-real-token").await?;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    // Well-formed ink_ token that doesn't exist in the DB.
    let response = do_login(
        &router,
        "ink_aabbccddeeff_0000000000000000000000000000000000000000000000000000000000000000",
    )
    .await?;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    Ok(())
}

/// Login with a revoked token returns 401.
#[tokio::test]
async fn flag_on_login_revoked_token_is_401() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };
    let router = browser_login_router(pool);

    let (token, prefix) = mint_token(&router, "revoked-author", &["read", "write"]).await?;
    // Revoke before attempting login.
    revoke_token(&router, &prefix).await?;

    let response = do_login(&router, &token).await?;
    assert_eq!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "revoked token must be rejected at login"
    );

    Ok(())
}

/// Session token is never returned in any response body (login returns 200 with
/// no body content containing the raw token).
#[tokio::test]
async fn flag_on_session_token_not_in_body() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };
    let router = browser_login_router(pool);

    let (token, _) = mint_token(&router, "secret-author", &["read", "write"]).await?;
    let response = do_login(&router, &token).await?;
    assert_eq!(response.status(), StatusCode::OK);

    let set_cookie = get_set_cookie(&response).expect("Set-Cookie");
    let session_token = parse_session_token(&set_cookie).expect("session token");

    let body = body_bytes(response).await?;
    // The 64-char session token must not appear verbatim in the body.
    assert!(
        !body
            .windows(session_token.len())
            .any(|w| w == session_token.as_bytes()),
        "session token ({session_token}) must not appear in response body"
    );

    Ok(())
}
