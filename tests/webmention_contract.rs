//! Database-backed contract tests for federation / Webmention (card T11, P3).
//!
//! Skipped unless `DATABASE_URL` is set (or forced via `INKWELL_REQUIRE_DB_TESTS=1`),
//! matching the rest of the db-backed suite. The SSRF classifier and the
//! URL/host/discovery helpers are unit-tested in-crate (no network); these tests
//! cover the receive endpoint's validation + 202 behavior, the visibility-filtered
//! mentions surface, and that send is inert with the flag off.

mod common;

use axum::body::{Body, to_bytes};
use http::{Method, Request, StatusCode};
use inkwell::db::documents::create_document;
use inkwell::db::links::Visibility;
use inkwell::db::webmentions;
use inkwell::domain::document::{DocumentStatus, NewDocument};
use std::sync::LazyLock;
use tokio::sync::{Mutex, MutexGuard};
use tower::ServiceExt;

/// Serialize the db-backed tests in this binary; `maybe_pool` truncates on entry.
static DB_TEST_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

async fn db_guard() -> MutexGuard<'static, ()> {
    DB_TEST_LOCK.lock().await
}

fn doc(slug: &str, status: DocumentStatus) -> NewDocument {
    NewDocument {
        slug: slug.to_string(),
        title: format!("Title {slug}"),
        body_markdown: format!("# {slug}"),
        rendered_html: format!("<h1>{slug}</h1>"),
        status: Some(status),
        growth: None,
        tags: Vec::new(),
        owner_id: None,
    }
}

/// A well-formed receive request 202s and queues verification (the async fetch
/// of the source fails in tests and drops the row — that's fine, the endpoint
/// must not block on it).
#[tokio::test]
async fn webmention_receive_accepts_valid_target_on_site() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };
    create_document(&pool, doc("hello", DocumentStatus::Published)).await?;
    let router = common::router_for(pool);

    let response = router
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/webmention")
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from(
                    "source=https://remote.example/post&target=https://blog.example.com/hello",
                ))?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    Ok(())
}

/// A target that is NOT on this site is a 400, never queued.
#[tokio::test]
async fn webmention_receive_rejects_off_site_target() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };
    let router = common::router_for(pool);

    let response = router
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/webmention")
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from(
                    "source=https://remote.example/post&target=https://evil.example.com/hello",
                ))?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    Ok(())
}

/// A target on this site whose slug is a DRAFT (or missing) is a 400 with the
/// same message — receiving must never reveal that a draft exists at the path.
#[tokio::test]
async fn webmention_receive_rejects_draft_and_unknown_targets() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };
    create_document(&pool, doc("secret", DocumentStatus::Draft)).await?;
    let router = common::router_for(pool);

    // Draft target → 400.
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/webmention")
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from(
                    "source=https://remote.example/post&target=https://blog.example.com/secret",
                ))?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    // Unknown target → 400 (same outcome, no existence signal).
    let response = router
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/webmention")
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from(
                    "source=https://remote.example/post&target=https://blog.example.com/nope",
                ))?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    Ok(())
}

/// A non-http(s) or malformed source is a 400.
#[tokio::test]
async fn webmention_receive_rejects_bad_source_scheme() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };
    create_document(&pool, doc("hello", DocumentStatus::Published)).await?;
    let router = common::router_for(pool);

    let response = router
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/webmention")
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from(
                    "source=file:///etc/passwd&target=https://blog.example.com/hello",
                ))?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    Ok(())
}

/// GET /webmention is 405 (the endpoint is POST-only).
#[tokio::test]
async fn webmention_endpoint_is_post_only() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };
    let router = common::router_for(pool);

    let response = router
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/webmention")
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
    Ok(())
}

/// Verified mentions surface in the backlinks JSON, and are visibility-filtered:
/// a mention of a DRAFT note is hidden from a public caller and shown to the
/// authenticated owner — exactly the no-draft-leak rule backlinks use.
#[tokio::test]
async fn verified_mentions_surface_in_backlinks_and_respect_visibility() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };

    let published = create_document(&pool, doc("public-note", DocumentStatus::Published)).await?;
    let draft = create_document(&pool, doc("draft-note", DocumentStatus::Draft)).await?;

    // A verified mention on each note.
    let pub_id = webmentions::upsert_pending(&pool, "https://a.example/post", published.id).await?;
    webmentions::mark_verified(&pool, pub_id).await?;
    let draft_id = webmentions::upsert_pending(&pool, "https://b.example/post", draft.id).await?;
    webmentions::mark_verified(&pool, draft_id).await?;

    // DB-level visibility contract: public sees the published note's mention,
    // not the draft's; the owner sees both.
    let pub_public =
        webmentions::verified_mentions(&pool, published.id, Visibility::Public).await?;
    assert_eq!(pub_public.len(), 1);
    assert_eq!(pub_public[0].source_url, "https://a.example/post");

    let draft_public = webmentions::verified_mentions(&pool, draft.id, Visibility::Public).await?;
    assert!(
        draft_public.is_empty(),
        "a mention targeting a draft must be hidden from public callers"
    );
    let draft_all = webmentions::verified_mentions(&pool, draft.id, Visibility::All).await?;
    assert_eq!(draft_all.len(), 1, "owner sees the draft's mention");

    // HTTP surface: the public backlinks JSON for the published note includes the
    // verified mention.
    let router = common::router_for(pool);
    let response = router
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/documents/public-note/backlinks")
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await?;
    let json: serde_json::Value = serde_json::from_slice(&body)?;
    let mentions = json["mentions"].as_array().expect("mentions array");
    assert_eq!(mentions.len(), 1);
    assert_eq!(mentions[0]["sourceUrl"], "https://a.example/post");
    Ok(())
}

/// A pending (unverified) mention must NOT surface — only verified ones do.
#[tokio::test]
async fn pending_mentions_are_not_surfaced() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };
    let note = create_document(&pool, doc("note", DocumentStatus::Published)).await?;
    // Record but do NOT verify.
    webmentions::upsert_pending(&pool, "https://pending.example/post", note.id).await?;

    let surfaced = webmentions::verified_mentions(&pool, note.id, Visibility::Public).await?;
    assert!(
        surfaced.is_empty(),
        "an unverified (pending) mention must never be surfaced"
    );
    Ok(())
}

/// Send is OFF in the test config, so publishing a note that links out is inert:
/// the publish still succeeds (the send path never blocks or fails it). This
/// asserts the publish path is unaffected when the flag is off.
#[tokio::test]
async fn send_is_inert_when_flag_off() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };
    // A draft note whose body links out to an external URL.
    create_document(&pool, {
        let mut d = doc("links-out", DocumentStatus::Draft);
        d.body_markdown = "See [example](https://example.com/post).".to_string();
        d
    })
    .await?;
    let router = common::router_for(pool);

    // Publishing it must succeed with send off (no panic, no hang, 200).
    let response = router
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/documents/links-out/publish")
                .header("x-api-key", "test-secret-key")
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    Ok(())
}
