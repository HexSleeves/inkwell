//! Database-backed contract tests for owner-aware read visibility
//! (ADR 0009, plan 023, **slice 3b**).
//!
//! Slice 3b narrows draft visibility so that a non-admin `read`-scoped author
//! sees ONLY their own drafts (plus all published notes), never another author's
//! drafts.  The key invariants:
//!
//!  - `GET /documents/{slug}` → 200 for own draft, 404 for another's draft.
//!  - `GET /documents` (list) → includes own draft, excludes other's draft.
//!  - `GET /search?q=...&format=json` → finds own draft, not other's draft.
//!  - `/documents/{slug}/related` → never surfaces the other author's draft.
//!  - `GET /ask?q=...` → never cites the other author's draft.
//!  - Anonymous → sees only published (no drafts).
//!  - Admin (shared key) → sees both drafts.
//!  - A *rejected* `x-api-key` (revoked/unknown/not token-shaped) → `401` on read
//!    routes as well as writes (ADR 0015, CYP-55).
//!  - A valid token with no `read` scope → `200`, published only (authorization,
//!    not rejection).
//!
//! Skipped unless `DATABASE_URL` is set (or forced via `INKWELL_REQUIRE_DB_TESTS=1`).

mod common;

use axum::body::{Body, to_bytes};
use http::{Method, Request, StatusCode};
use std::sync::LazyLock;
use tokio::sync::{Mutex, MutexGuard};
use tower::ServiceExt;

static DB_TEST_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

async fn db_guard() -> MutexGuard<'static, ()> {
    DB_TEST_LOCK.lock().await
}

const SHARED_KEY: &str = "test-secret-key";

/// Deserialize the response body as JSON.
async fn body_json(response: axum::response::Response) -> anyhow::Result<serde_json::Value> {
    let bytes = to_bytes(response.into_body(), usize::MAX).await?;
    Ok(serde_json::from_slice(&bytes)?)
}

/// Mint a scoped token for a new author via `POST /admin/tokens`.
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
    let token = body_json(response)
        .await?
        .get("token")
        .and_then(|t| t.as_str())
        .expect("token in response")
        .to_string();
    Ok(token)
}

/// Create a draft note, return the slug.
async fn create_draft(
    router: &axum::Router,
    title: &str,
    body: &str,
    key: &str,
) -> anyhow::Result<String> {
    let payload = serde_json::json!({ "title": title, "bodyMarkdown": body });
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/documents")
                .header("content-type", "application/json")
                .header("x-api-key", key)
                .body(Body::from(serde_json::to_vec(&payload)?))?,
        )
        .await?;
    assert_eq!(
        response.status(),
        StatusCode::CREATED,
        "create should succeed"
    );
    let slug = body_json(response)
        .await?
        .get("slug")
        .and_then(|s| s.as_str())
        .expect("slug in response")
        .to_string();
    Ok(slug)
}

/// Publish a note. Returns 200 on success.
async fn publish(router: &axum::Router, slug: &str, key: &str) -> anyhow::Result<StatusCode> {
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri(format!("/documents/{slug}/publish"))
                .header("x-api-key", key)
                .body(Body::empty())?,
        )
        .await?;
    Ok(response.status())
}

/// Simple GET helper: return status code + response body as JSON.
async fn get_json(
    router: &axum::Router,
    uri: &str,
    key: Option<&str>,
) -> anyhow::Result<(StatusCode, serde_json::Value)> {
    let mut builder = Request::builder().method(Method::GET).uri(uri);
    if let Some(k) = key {
        builder = builder.header("x-api-key", k);
    }
    let response = router.clone().oneshot(builder.body(Body::empty())?).await?;
    let status = response.status();
    let json = body_json(response).await.unwrap_or(serde_json::Value::Null);
    Ok((status, json))
}

// ───────────────────────────────────────────────────────────────────────────
// Core isolation test
// ───────────────────────────────────────────────────────────────────────────

/// An author with `read` scope sees their own draft but NOT another author's draft
/// in GET /documents/{slug}, GET /documents (list), and GET /search?format=json.
/// Anonymous sees only published.  Admin sees both.
#[tokio::test]
async fn owner_aware_read_visibility() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };
    let router = common::router_for_with_ai(pool.clone());

    // Mint author A (read + write + publish) and author B (read + write + publish).
    let token_a = mint_token(&router, "Alice", &["read", "write", "publish"]).await?;
    let token_b = mint_token(&router, "Bob", &["read", "write", "publish"]).await?;

    // Create drafts: A owns "alice-draft-note", B owns "bob-draft-note".
    let a_slug = create_draft(
        &router,
        "Alice Draft Note",
        "Alice's secret draft content about quantum computing",
        &token_a,
    )
    .await?;
    let b_slug = create_draft(
        &router,
        "Bob Draft Note",
        "Bob's secret draft content about quantum computing",
        &token_b,
    )
    .await?;

    // Also create a published note (owned by A) so we can verify published notes
    // are visible to all.
    let pub_slug =
        create_draft(&router, "Public Knowledge", "Everyone sees this", &token_a).await?;
    assert_eq!(publish(&router, &pub_slug, &token_a).await?, StatusCode::OK);

    // ── GET /documents/{slug} ────────────────────────────────────────────────

    // A can see their own draft.
    let (status, _) = get_json(&router, &format!("/documents/{a_slug}"), Some(&token_a)).await?;
    assert_eq!(status, StatusCode::OK, "A should see their own draft");

    // A CANNOT see B's draft — must be 404 (not 200, not 403).
    let (status, _) = get_json(&router, &format!("/documents/{b_slug}"), Some(&token_a)).await?;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "A must NOT see B's draft (cross-author leak!)"
    );

    // B can see their own draft.
    let (status, _) = get_json(&router, &format!("/documents/{b_slug}"), Some(&token_b)).await?;
    assert_eq!(status, StatusCode::OK, "B should see their own draft");

    // B CANNOT see A's draft.
    let (status, _) = get_json(&router, &format!("/documents/{a_slug}"), Some(&token_b)).await?;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "B must NOT see A's draft (cross-author leak!)"
    );

    // Anonymous sees neither draft.
    let (status, _) = get_json(&router, &format!("/documents/{a_slug}"), None).await?;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "anonymous must not see A's draft"
    );
    let (status, _) = get_json(&router, &format!("/documents/{b_slug}"), None).await?;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "anonymous must not see B's draft"
    );

    // Admin sees both drafts.
    let (status, _) = get_json(&router, &format!("/documents/{a_slug}"), Some(SHARED_KEY)).await?;
    assert_eq!(status, StatusCode::OK, "admin must see A's draft");
    let (status, _) = get_json(&router, &format!("/documents/{b_slug}"), Some(SHARED_KEY)).await?;
    assert_eq!(status, StatusCode::OK, "admin must see B's draft");

    // ── GET /documents (list) ────────────────────────────────────────────────

    // A's list must include A's draft and the published note, but NOT B's draft.
    let (status, body) = get_json(&router, "/documents", Some(&token_a)).await?;
    assert_eq!(status, StatusCode::OK, "list should succeed for A");
    let slugs_a: Vec<&str> = body["documents"]
        .as_array()
        .expect("documents array")
        .iter()
        .filter_map(|d| d["slug"].as_str())
        .collect();
    assert!(
        slugs_a.contains(&a_slug.as_str()),
        "A's list must include own draft: {slugs_a:?}"
    );
    assert!(
        slugs_a.contains(&pub_slug.as_str()),
        "A's list must include published: {slugs_a:?}"
    );
    assert!(
        !slugs_a.contains(&b_slug.as_str()),
        "A's list must NOT include B's draft (cross-author leak!): {slugs_a:?}"
    );

    // B's list must include B's draft and the published note, but NOT A's draft.
    let (status, body) = get_json(&router, "/documents", Some(&token_b)).await?;
    assert_eq!(status, StatusCode::OK, "list should succeed for B");
    let slugs_b: Vec<&str> = body["documents"]
        .as_array()
        .expect("documents array")
        .iter()
        .filter_map(|d| d["slug"].as_str())
        .collect();
    assert!(
        slugs_b.contains(&b_slug.as_str()),
        "B's list must include own draft: {slugs_b:?}"
    );
    assert!(
        slugs_b.contains(&pub_slug.as_str()),
        "B's list must include published: {slugs_b:?}"
    );
    assert!(
        !slugs_b.contains(&a_slug.as_str()),
        "B's list must NOT include A's draft (cross-author leak!): {slugs_b:?}"
    );

    // Anonymous list sees only the published note.
    let (status, body) = get_json(&router, "/documents", None).await?;
    assert_eq!(status, StatusCode::OK);
    let slugs_anon: Vec<&str> = body["documents"]
        .as_array()
        .expect("documents array")
        .iter()
        .filter_map(|d| d["slug"].as_str())
        .collect();
    assert!(
        !slugs_anon.contains(&a_slug.as_str()),
        "anonymous must not see A's draft: {slugs_anon:?}"
    );
    assert!(
        !slugs_anon.contains(&b_slug.as_str()),
        "anonymous must not see B's draft: {slugs_anon:?}"
    );
    assert!(
        slugs_anon.contains(&pub_slug.as_str()),
        "anonymous must see published: {slugs_anon:?}"
    );

    // Admin list sees all three.
    let (status, body) = get_json(&router, "/documents", Some(SHARED_KEY)).await?;
    assert_eq!(status, StatusCode::OK);
    let slugs_admin: Vec<&str> = body["documents"]
        .as_array()
        .expect("documents array")
        .iter()
        .filter_map(|d| d["slug"].as_str())
        .collect();
    assert!(
        slugs_admin.contains(&a_slug.as_str()),
        "admin must see A's draft: {slugs_admin:?}"
    );
    assert!(
        slugs_admin.contains(&b_slug.as_str()),
        "admin must see B's draft: {slugs_admin:?}"
    );

    // ── GET /search?q=...&format=json ────────────────────────────────────────

    // A's search finds A's draft (same keyword), not B's draft.
    let (status, body) = get_json(
        &router,
        "/search?q=quantum+computing&format=json",
        Some(&token_a),
    )
    .await?;
    assert_eq!(status, StatusCode::OK, "search should succeed for A");
    let result_slugs_a: Vec<&str> = body["results"]
        .as_array()
        .expect("results array")
        .iter()
        .filter_map(|r| r["slug"].as_str())
        .collect();
    assert!(
        result_slugs_a.contains(&a_slug.as_str()),
        "A's search must find own draft: {result_slugs_a:?}"
    );
    assert!(
        !result_slugs_a.contains(&b_slug.as_str()),
        "A's search must NOT find B's draft (cross-author leak!): {result_slugs_a:?}"
    );

    // B's search finds B's draft, not A's.
    let (status, body) = get_json(
        &router,
        "/search?q=quantum+computing&format=json",
        Some(&token_b),
    )
    .await?;
    assert_eq!(status, StatusCode::OK, "search should succeed for B");
    let result_slugs_b: Vec<&str> = body["results"]
        .as_array()
        .expect("results array")
        .iter()
        .filter_map(|r| r["slug"].as_str())
        .collect();
    assert!(
        result_slugs_b.contains(&b_slug.as_str()),
        "B's search must find own draft: {result_slugs_b:?}"
    );
    assert!(
        !result_slugs_b.contains(&a_slug.as_str()),
        "B's search must NOT find A's draft (cross-author leak!): {result_slugs_b:?}"
    );

    // Anonymous search finds no drafts.
    let (status, body) = get_json(&router, "/search?q=quantum+computing&format=json", None).await?;
    assert_eq!(status, StatusCode::OK);
    let result_slugs_anon: Vec<&str> = body["results"]
        .as_array()
        .expect("results array")
        .iter()
        .filter_map(|r| r["slug"].as_str())
        .collect();
    assert!(
        !result_slugs_anon.contains(&a_slug.as_str()),
        "anonymous search must not find A's draft: {result_slugs_anon:?}"
    );
    assert!(
        !result_slugs_anon.contains(&b_slug.as_str()),
        "anonymous search must not find B's draft: {result_slugs_anon:?}"
    );

    // Admin search finds both drafts.
    let (status, body) = get_json(
        &router,
        "/search?q=quantum+computing&format=json",
        Some(SHARED_KEY),
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    let result_slugs_admin: Vec<&str> = body["results"]
        .as_array()
        .expect("results array")
        .iter()
        .filter_map(|r| r["slug"].as_str())
        .collect();
    assert!(
        result_slugs_admin.contains(&a_slug.as_str()),
        "admin search must find A's draft: {result_slugs_admin:?}"
    );
    assert!(
        result_slugs_admin.contains(&b_slug.as_str()),
        "admin search must find B's draft: {result_slugs_admin:?}"
    );

    Ok(())
}

// ───────────────────────────────────────────────────────────────────────────
// /related and /ask cross-author draft leak test
// ───────────────────────────────────────────────────────────────────────────

/// `/documents/{slug}/related` and `/ask` must never surface another author's
/// draft as a related note or a citation, even when vector similarity is high.
#[tokio::test]
async fn related_and_ask_do_not_leak_cross_author_drafts() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };
    let router = common::router_for_with_ai(pool.clone());

    let token_a = mint_token(&router, "Alice", &["read", "write", "publish"]).await?;
    let token_b = mint_token(&router, "Bob", &["read", "write", "publish"]).await?;

    // A owns a published "anchor" note and a draft.
    let anchor_slug = create_draft(
        &router,
        "A Anchor Published",
        "published anchor note for related-notes test",
        &token_a,
    )
    .await?;
    assert_eq!(
        publish(&router, &anchor_slug, &token_a).await?,
        StatusCode::OK
    );

    let a_draft_slug = create_draft(
        &router,
        "A Secret Draft",
        "published anchor note for related-notes test",
        &token_a,
    )
    .await?;

    // B owns a draft with very similar content (to trigger embedding similarity).
    let b_draft_slug = create_draft(
        &router,
        "B Secret Draft",
        "published anchor note for related-notes test",
        &token_b,
    )
    .await?;

    // ── /documents/{slug}/related ────────────────────────────────────────────

    // A viewing the published anchor's related notes: B's draft must NOT appear.
    let (status, body) = get_json(
        &router,
        &format!("/documents/{anchor_slug}/related"),
        Some(&token_a),
    )
    .await?;
    assert_eq!(status, StatusCode::OK, "related should succeed");
    let related_slugs: Vec<&str> = body["related"]
        .as_array()
        .expect("related array")
        .iter()
        .filter_map(|r| r["slug"].as_str())
        .collect();
    assert!(
        !related_slugs.contains(&b_draft_slug.as_str()),
        "A's /related must NOT surface B's draft (cross-author leak!): {related_slugs:?}"
    );
    // A's own draft MAY appear in related (it's visible to A), but the critical
    // invariant is that B's draft does NOT.

    // B viewing the published anchor's related notes: A's draft must NOT appear.
    let (status, body) = get_json(
        &router,
        &format!("/documents/{anchor_slug}/related"),
        Some(&token_b),
    )
    .await?;
    assert_eq!(status, StatusCode::OK, "related should succeed for B");
    let related_slugs_b: Vec<&str> = body["related"]
        .as_array()
        .expect("related array")
        .iter()
        .filter_map(|r| r["slug"].as_str())
        .collect();
    assert!(
        !related_slugs_b.contains(&a_draft_slug.as_str()),
        "B's /related must NOT surface A's draft (cross-author leak!): {related_slugs_b:?}"
    );

    // Anonymous /related: neither draft appears.
    let (status, body) =
        get_json(&router, &format!("/documents/{anchor_slug}/related"), None).await?;
    assert_eq!(status, StatusCode::OK);
    let related_slugs_anon: Vec<&str> = body["related"]
        .as_array()
        .expect("related array")
        .iter()
        .filter_map(|r| r["slug"].as_str())
        .collect();
    assert!(
        !related_slugs_anon.contains(&a_draft_slug.as_str()),
        "anonymous /related must not surface A's draft: {related_slugs_anon:?}"
    );
    assert!(
        !related_slugs_anon.contains(&b_draft_slug.as_str()),
        "anonymous /related must not surface B's draft: {related_slugs_anon:?}"
    );

    // ── /ask ────────────────────────────────────────────────────────────────

    // A asks a question: B's draft must never appear in citations.
    let (status, body) =
        get_json(&router, "/ask?q=anchor+note+related+test", Some(&token_a)).await?;
    assert_eq!(status, StatusCode::OK, "/ask should succeed for A");
    let cited_slugs_a: Vec<&str> = body["citations"]
        .as_array()
        .expect("citations array")
        .iter()
        .filter_map(|c| c["slug"].as_str())
        .collect();
    assert!(
        !cited_slugs_a.contains(&b_draft_slug.as_str()),
        "A's /ask must NOT cite B's draft (cross-author leak!): {cited_slugs_a:?}"
    );

    // B asks a question: A's draft must never appear in citations.
    let (status, body) =
        get_json(&router, "/ask?q=anchor+note+related+test", Some(&token_b)).await?;
    assert_eq!(status, StatusCode::OK, "/ask should succeed for B");
    let cited_slugs_b: Vec<&str> = body["citations"]
        .as_array()
        .expect("citations array")
        .iter()
        .filter_map(|c| c["slug"].as_str())
        .collect();
    assert!(
        !cited_slugs_b.contains(&a_draft_slug.as_str()),
        "B's /ask must NOT cite A's draft (cross-author leak!): {cited_slugs_b:?}"
    );

    // Anonymous /ask: no draft cited.
    let (status, body) = get_json(&router, "/ask?q=anchor+note+related+test", None).await?;
    assert_eq!(status, StatusCode::OK, "/ask should succeed for anonymous");
    let cited_slugs_anon: Vec<&str> = body["citations"]
        .as_array()
        .expect("citations array")
        .iter()
        .filter_map(|c| c["slug"].as_str())
        .collect();
    assert!(
        !cited_slugs_anon.contains(&a_draft_slug.as_str()),
        "anonymous /ask must not cite A's draft: {cited_slugs_anon:?}"
    );
    assert!(
        !cited_slugs_anon.contains(&b_draft_slug.as_str()),
        "anonymous /ask must not cite B's draft: {cited_slugs_anon:?}"
    );

    Ok(())
}

// ───────────────────────────────────────────────────────────────────────────
// Admin bypass test (admin sees all drafts)
// ───────────────────────────────────────────────────────────────────────────

/// The admin shared key must continue to see every note (both authors' drafts).
#[tokio::test]
async fn admin_sees_all_drafts() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };
    let router = common::router_for(pool.clone());

    let token_a = mint_token(&router, "Alice", &["read", "write", "publish"]).await?;
    let token_b = mint_token(&router, "Bob", &["read", "write", "publish"]).await?;

    let a_slug = create_draft(&router, "Admin Test A Draft", "body A", &token_a).await?;
    let b_slug = create_draft(&router, "Admin Test B Draft", "body B", &token_b).await?;

    // Admin GET: both visible.
    let (status, _) = get_json(&router, &format!("/documents/{a_slug}"), Some(SHARED_KEY)).await?;
    assert_eq!(status, StatusCode::OK, "admin must see A's draft");
    let (status, _) = get_json(&router, &format!("/documents/{b_slug}"), Some(SHARED_KEY)).await?;
    assert_eq!(status, StatusCode::OK, "admin must see B's draft");

    // Admin list: both appear.
    let (status, body) = get_json(&router, "/documents", Some(SHARED_KEY)).await?;
    assert_eq!(status, StatusCode::OK);
    let slugs: Vec<&str> = body["documents"]
        .as_array()
        .expect("documents array")
        .iter()
        .filter_map(|d| d["slug"].as_str())
        .collect();
    assert!(
        slugs.contains(&a_slug.as_str()),
        "admin list must include A's draft: {slugs:?}"
    );
    assert!(
        slugs.contains(&b_slug.as_str()),
        "admin list must include B's draft: {slugs:?}"
    );

    Ok(())
}

// ───────────────────────────────────────────────────────────────────────────
// Write-only token: no draft visibility (slice 3 coarse gate preserved)
// ───────────────────────────────────────────────────────────────────────────

/// A `write`-only token (no `read`) should still see no drafts — the coarse
/// read gate from slice 3 must not be broken by slice 3b.
#[tokio::test]
async fn write_only_token_still_cannot_see_drafts() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };
    let router = common::router_for(pool.clone());

    let writer = mint_token(&router, "WriteOnly", &["write"]).await?;

    // Writer creates a draft — but cannot see it (no `read` scope).
    let slug = create_draft(&router, "Invisible Draft", "body", &writer).await?;

    let (status, _) = get_json(&router, &format!("/documents/{slug}"), Some(&writer)).await?;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "write-only token must not see its own draft (no read scope)"
    );

    Ok(())
}

// ───────────────────────────────────────────────────────────────────────────
// Rejected credential on a read route: served as anonymous (ADR 0015)
// ───────────────────────────────────────────────────────────────────────────

/// Revoke a token by its public prefix using the shared admin key.
async fn revoke(router: &axum::Router, token: &str) -> anyhow::Result<StatusCode> {
    let prefix = token
        .split('_')
        .nth(1)
        .expect("token is ink_<prefix>_<secret>");
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
    Ok(response.status())
}

/// **Contract test for the ADR 0015 rule (CYP-55, shipped after `v0.2.0`).**
///
/// An `x-api-key` that is *presented and rejected* — a freshly revoked token, an
/// unknown prefix, or a value that is not even token-shaped — is `401` on read
/// routes as well as write routes. Up to and including `v0.2.0` the read routes
/// answered `200` with published-only content; that silent downgrade is gone on
/// this channel because nothing sets `x-api-key` by accident.
///
/// The cookie channel keeps failing open — see
/// `flag_on_stale_session_cookie_still_reads_as_anonymous` in `tests/browser_login.rs`.
#[tokio::test]
async fn rejected_api_key_is_401_on_read_routes() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };
    let router = common::router_for(pool.clone());

    let token = mint_token(&router, "Revokee", &["read", "write", "publish"]).await?;
    let draft_slug = create_draft(&router, "Revokee Draft", "unpublished body", &token).await?;
    let pub_slug = create_draft(&router, "Revokee Published", "public body", &token).await?;
    assert_eq!(publish(&router, &pub_slug, &token).await?, StatusCode::OK);

    // Sanity: while live, the token sees its own draft.
    let (status, _) = get_json(&router, &format!("/documents/{draft_slug}"), Some(&token)).await?;
    assert_eq!(status, StatusCode::OK, "live token sees its own draft");

    assert_eq!(revoke(&router, &token).await?, StatusCode::OK);

    // The write route rejects the revoked token outright.
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/documents")
                .header("content-type", "application/json")
                .header("x-api-key", token.clone())
                .body(Body::from(serde_json::to_vec(
                    &serde_json::json!({ "title": "Nope", "bodyMarkdown": "x" }),
                )?))?,
        )
        .await?;
    assert_eq!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "a revoked token must not authenticate a write"
    );

    // Every flavour of rejected x-api-key is 401 on read routes, not a silent
    // downgrade to anonymous (ADR 0015 / CYP-55).
    for (label, key) in [
        ("revoked", token.as_str()),
        (
            "unknown-prefix",
            "ink_zzzzzzzz_deadbeefdeadbeefdeadbeefdeadbeef",
        ),
        ("not-token-shaped", "definitely-not-a-credential"),
    ] {
        for route in [
            "/documents",
            "/graph",
            "/search?format=json&q=body",
            "/documents/{slug}",
            "/documents/{slug}/backlinks",
            "/documents/{slug}/graph",
            "/documents/{slug}/related",
            "/documents/{slug}/history",
        ] {
            let uri = route.replace("{slug}", &pub_slug);
            let (status, _) = get_json(&router, &uri, Some(key)).await?;
            assert_eq!(
                status,
                StatusCode::UNAUTHORIZED,
                "{label} credential must be 401 on read route {uri} (ADR 0015)"
            );
        }

        // The anonymous read of the same routes is untouched: no credential
        // presented, published content served.
        let (status, body) = get_json(&router, "/documents", None).await?;
        assert_eq!(status, StatusCode::OK, "anonymous read is unaffected");
        let slugs: Vec<&str> = body["documents"]
            .as_array()
            .expect("documents array")
            .iter()
            .filter_map(|d| d["slug"].as_str())
            .collect();
        assert!(
            slugs.contains(&pub_slug.as_str()),
            "anonymous read still sees published content: {slugs:?}"
        );
        assert!(
            !slugs.contains(&draft_slug.as_str()),
            "anonymous read must NOT see drafts: {slugs:?}"
        );
    }

    Ok(())
}

/// A token that authenticates but holds no `read` scope is **not** a rejected
/// credential (ADR 0015). It stays `200` with published-only content, so the
/// authorization case is never swept into the new rejection case.
#[tokio::test]
async fn write_only_token_reads_published_only_not_401() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };
    let router = common::router_for(pool.clone());

    // A separate read+write author owns the content, so the write-only token has
    // no ownership claim on it either way.
    let owner = mint_token(&router, "Scope Owner", &["read", "write", "publish"]).await?;
    let draft_slug = create_draft(&router, "Scope Draft", "unpublished body", &owner).await?;
    let pub_slug = create_draft(&router, "Scope Published", "public body", &owner).await?;
    assert_eq!(publish(&router, &pub_slug, &owner).await?, StatusCode::OK);

    let write_only = mint_token(&router, "Writer Only", &["write"]).await?;

    let (status, body) = get_json(&router, "/documents", Some(&write_only)).await?;
    assert_eq!(
        status,
        StatusCode::OK,
        "a valid write-only token authenticates; it is not a rejected credential"
    );
    let slugs: Vec<&str> = body["documents"]
        .as_array()
        .expect("documents array")
        .iter()
        .filter_map(|d| d["slug"].as_str())
        .collect();
    assert!(
        slugs.contains(&pub_slug.as_str()),
        "write-only token sees published content: {slugs:?}"
    );
    assert!(
        !slugs.contains(&draft_slug.as_str()),
        "write-only token must not see drafts: {slugs:?}"
    );

    let (status, _) = get_json(
        &router,
        &format!("/documents/{pub_slug}"),
        Some(&write_only),
    )
    .await?;
    assert_eq!(
        status,
        StatusCode::OK,
        "write-only token reads a published document"
    );

    Ok(())
}
