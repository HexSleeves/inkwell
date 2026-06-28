//! Contract tests for the authoring web UI (CYP-42).
//!
//! Complements `tests/browser_login_ui.rs`. Covers:
//!
//! **Flag OFF (default `test_config`):**
//! - `GET /editor` is a 404 — the editor routes are not registered, so the
//!   request falls through to the `/{slug}` document route (404 for that slug).
//!
//! **Flag ON (`browser_login: true`):**
//! - `GET /editor` without a session cookie → 303 redirect to `/login` (a UX
//!   convenience; the API is the real auth boundary).
//! - `GET /editor`, `/editor/new`, `/editor/{slug}` with a session cookie → 200
//!   `text/html` carrying the nonce'd CSP and the expected editor markup.
//! - A full create → edit → publish → unpublish round-trip driven entirely
//!   through the same `/documents` JSON API the editor pages call, authenticated
//!   by the session cookie a browser login mints.
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

static DB_TEST_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

async fn db_guard() -> MutexGuard<'static, ()> {
    DB_TEST_LOCK.lock().await
}

const SHARED_KEY: &str = "test-secret-key";

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
    Ok(json["token"].as_str().expect("token").to_string())
}

/// Log in with a token and return the `inkwell_session=<token>` cookie header.
async fn login_cookie(router: &axum::Router, token: &str) -> anyhow::Result<String> {
    let payload = serde_json::json!({ "token": token });
    let login = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/auth/login")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&payload)?))?,
        )
        .await?;
    assert_eq!(login.status(), StatusCode::OK, "login must return 200");
    let set_cookie = login
        .headers()
        .get(http::header::SET_COOKIE)
        .and_then(|v| v.to_str().ok())
        .expect("Set-Cookie on login")
        .to_string();
    let session = set_cookie
        .split(';')
        .next()
        .and_then(|p| p.strip_prefix("inkwell_session="))
        .expect("session token");
    Ok(format!("inkwell_session={session}"))
}

// ---------------------------------------------------------------------------
// Flag OFF
// ---------------------------------------------------------------------------

#[tokio::test]
async fn flag_off_editor_is_404() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };
    let router = common::router_for(pool);
    let response = router
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/editor")
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(
        response.status(),
        StatusCode::NOT_FOUND,
        "GET /editor must be 404 when the browser-login flag is off"
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// Flag ON — auth-gated redirect
// ---------------------------------------------------------------------------

#[tokio::test]
async fn flag_on_editor_redirects_when_signed_out() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };
    let router = browser_login_router(pool);
    let response = router
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/editor")
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(
        response.status(),
        StatusCode::SEE_OTHER,
        "a signed-out visitor must be redirected"
    );
    let location = response
        .headers()
        .get(http::header::LOCATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    assert_eq!(location, "/login", "redirect must target the login page");
    Ok(())
}

// ---------------------------------------------------------------------------
// Flag ON — pages render with a session cookie
// ---------------------------------------------------------------------------

#[tokio::test]
async fn flag_on_editor_pages_render_with_session() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };
    let router = browser_login_router(pool);
    let token = mint_token(
        &router,
        "editor-pages-author",
        &["read", "write", "publish"],
    )
    .await?;
    let cookie = login_cookie(&router, &token).await?;

    for (uri, needle) in [
        ("/editor", "doc-list"),
        ("/editor/new", "new-form"),
        ("/editor/some-slug", r#"data-slug="some-slug""#),
    ] {
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri(uri)
                    .header("cookie", &cookie)
                    .body(Body::empty())?,
            )
            .await?;
        assert_eq!(response.status(), StatusCode::OK, "GET {uri} must be 200");
        let content_type = response
            .headers()
            .get(http::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default();
        assert!(
            content_type.starts_with("text/html"),
            "{uri} must be text/html; got {content_type}"
        );
        let csp = response
            .headers()
            .get(http::header::CONTENT_SECURITY_POLICY)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_string();
        assert!(
            csp.contains("nonce-"),
            "{uri} CSP must carry a script nonce"
        );
        let body = body_string(response).await?;
        assert!(body.contains(needle), "{uri} must contain {needle}");
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Flag ON — full create → edit → publish → unpublish round-trip via the API
// ---------------------------------------------------------------------------

#[tokio::test]
async fn editor_create_edit_publish_unpublish_round_trip() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };
    let router = browser_login_router(pool);
    let token = mint_token(&router, "editor-e2e-author", &["read", "write", "publish"]).await?;
    let cookie = login_cookie(&router, &token).await?;

    // Create (POST /documents) — same call the new-document page makes.
    let create_body =
        serde_json::json!({ "title": "Editor E2E", "bodyMarkdown": "# hello", "tags": ["a", "b"] });
    let create = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/documents")
                .header("content-type", "application/json")
                .header("cookie", &cookie)
                .body(Body::from(serde_json::to_vec(&create_body)?))?,
        )
        .await?;
    assert_eq!(create.status(), StatusCode::CREATED, "create must 201");
    let created: serde_json::Value =
        serde_json::from_slice(&to_bytes(create.into_body(), usize::MAX).await?)?;
    let slug = created["slug"].as_str().expect("slug").to_string();
    assert_eq!(created["status"], "draft", "new docs start as draft");
    let version = created["version"].as_i64().expect("version");
    // The rendered HTML the preview pane shows is present on the envelope.
    assert!(
        created["renderedHtml"]
            .as_str()
            .unwrap_or_default()
            .contains("hello"),
        "create response must carry renderedHtml for the preview"
    );

    // Edit (PATCH /documents/{slug}) with If-Match — the save-draft path.
    let patch_body = serde_json::json!({ "bodyMarkdown": "# hello\n\nedited", "tags": ["a"] });
    let patch = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::PATCH)
                .uri(format!("/documents/{slug}"))
                .header("content-type", "application/json")
                .header("if-match", version.to_string())
                .header("cookie", &cookie)
                .body(Body::from(serde_json::to_vec(&patch_body)?))?,
        )
        .await?;
    assert_eq!(patch.status(), StatusCode::OK, "edit must 200");

    // Publish (POST /documents/{slug}/publish).
    let publish = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri(format!("/documents/{slug}/publish"))
                .header("cookie", &cookie)
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(publish.status(), StatusCode::OK, "publish must 200");
    let published: serde_json::Value =
        serde_json::from_slice(&to_bytes(publish.into_body(), usize::MAX).await?)?;
    assert_eq!(published["status"], "published", "doc must be published");

    // The published doc is now resolvable on the public page path.
    let public = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(format!("/{slug}"))
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(
        public.status(),
        StatusCode::OK,
        "published doc must render on the public path"
    );

    // Unpublish (POST /documents/{slug}/unpublish).
    let unpublish = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri(format!("/documents/{slug}/unpublish"))
                .header("cookie", &cookie)
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(unpublish.status(), StatusCode::OK, "unpublish must 200");
    let back_to_draft: serde_json::Value =
        serde_json::from_slice(&to_bytes(unpublish.into_body(), usize::MAX).await?)?;
    assert_eq!(
        back_to_draft["status"], "draft",
        "doc must be a draft again"
    );
    Ok(())
}
