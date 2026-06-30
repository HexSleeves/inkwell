//! Contract tests for `/graph` content negotiation (the interactive HTML page
//! vs. the historical JSON wire contract). DB-gated like the other contract
//! suites: a no-database environment skips via `common::maybe_router()`.
//!
//! The invariant under test: a browser (`Accept: text/html`) gets the HTML
//! graph page; everything else (`application/json`, `*/*`, or no `Accept`) gets
//! the JSON envelope byte-for-byte, so the documented API contract is preserved.

mod common;

use axum::body::{Body, to_bytes};
use http::{Method, Request, StatusCode, header};
use std::sync::LazyLock;
use tokio::sync::{Mutex, MutexGuard};
use tower::ServiceExt;

const SHARED_KEY: &str = "test-secret-key";

static DB_TEST_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

async fn db_guard() -> MutexGuard<'static, ()> {
    DB_TEST_LOCK.lock().await
}

/// Seed one published and one draft note so the graph has a draft to (not) leak.
async fn seed_published_and_draft(router: &axum::Router) -> anyhow::Result<()> {
    for (title, published) in [("Public Note", true), ("Draft Note", false)] {
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
        if published {
            let response = router
                .clone()
                .oneshot(
                    Request::builder()
                        .method(Method::POST)
                        .uri("/documents/public-note/publish")
                        .header("x-api-key", SHARED_KEY)
                        .body(Body::empty())?,
                )
                .await?;
            assert_eq!(response.status(), StatusCode::OK);
        }
    }
    Ok(())
}

fn header_str(response: &http::Response<Body>, name: header::HeaderName) -> String {
    response
        .headers()
        .get(name)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_string()
}

async fn get_graph(
    router: &axum::Router,
    accept: Option<&str>,
    api_key: Option<&str>,
) -> anyhow::Result<http::Response<Body>> {
    let mut builder = Request::builder().method(Method::GET).uri("/graph");
    if let Some(accept) = accept {
        builder = builder.header(header::ACCEPT, accept);
    }
    if let Some(key) = api_key {
        builder = builder.header("x-api-key", key);
    }
    Ok(router.clone().oneshot(builder.body(Body::empty())?).await?)
}

#[tokio::test]
async fn no_accept_and_json_and_wildcard_all_get_json() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(router) = common::maybe_router().await? else {
        return Ok(());
    };
    seed_published_and_draft(&router).await?;

    // Includes a quality-value case: `text/html;q=0` lists HTML but refuses it,
    // and a JSON-preferred header — both must fall through to JSON.
    for accept in [
        None,
        Some("application/json"),
        Some("*/*"),
        Some("application/json, text/html;q=0"),
        Some("text/html;q=0.3, application/json;q=0.9"),
    ] {
        let response = get_graph(&router, accept, None).await?;
        assert_eq!(response.status(), StatusCode::OK);
        assert!(
            header_str(&response, header::CONTENT_TYPE).contains("application/json"),
            "Accept {accept:?} must yield JSON, not HTML"
        );
        // The same URL is content-negotiated, so caches must key on Accept.
        assert!(
            header_str(&response, header::VARY).contains("Accept"),
            "Accept {accept:?} response must carry Vary: Accept"
        );
        let body = to_bytes(response.into_body(), usize::MAX).await?;
        let json: serde_json::Value = serde_json::from_slice(&body)?;
        assert!(json["nodes"].is_array(), "JSON envelope has a nodes array");
        assert!(json["edges"].is_array(), "JSON envelope has an edges array");
    }
    Ok(())
}

#[tokio::test]
async fn authenticated_json_graph_is_no_store() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(router) = common::maybe_router().await? else {
        return Ok(());
    };
    seed_published_and_draft(&router).await?;

    // Authenticated JSON can include the caller's drafts, so it must not be
    // shared-cached even though it is the API representation.
    let response = get_graph(&router, Some("application/json"), Some(SHARED_KEY)).await?;
    assert_eq!(response.status(), StatusCode::OK);
    assert!(header_str(&response, header::CONTENT_TYPE).contains("application/json"));
    assert!(header_str(&response, header::CACHE_CONTROL).contains("no-store"));
    assert!(header_str(&response, header::VARY).contains("Accept"));

    let body = to_bytes(response.into_body(), usize::MAX).await?;
    let json: serde_json::Value = serde_json::from_slice(&body)?;
    let slugs: Vec<&str> = json["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|node| node["slug"].as_str().unwrap())
        .collect();
    assert!(
        slugs.contains(&"draft-note"),
        "owner JSON includes the draft"
    );
    Ok(())
}

#[tokio::test]
async fn browser_accept_gets_the_html_page_and_hides_drafts() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(router) = common::maybe_router().await? else {
        return Ok(());
    };
    seed_published_and_draft(&router).await?;

    let response = get_graph(
        &router,
        Some("text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8"),
        None,
    )
    .await?;
    assert_eq!(response.status(), StatusCode::OK);
    assert!(header_str(&response, header::CONTENT_TYPE).starts_with("text/html"));
    // Anonymous HTML graph is the same for everyone, so it rides the shared cache.
    assert!(header_str(&response, header::CACHE_CONTROL).contains("public"));
    assert!(header_str(&response, header::VARY).contains("Accept"));

    let body = to_bytes(response.into_body(), usize::MAX).await?;
    let html = String::from_utf8(body.to_vec())?;
    assert!(
        html.contains(r#"id="graph-data""#),
        "renders the data island"
    );
    assert!(
        html.contains("graph-fallback"),
        "renders the no-JS fallback"
    );
    assert!(html.contains("/public-note"), "published node is present");
    assert!(
        !html.contains("draft-note"),
        "a draft must never appear in the public HTML graph"
    );
    Ok(())
}

#[tokio::test]
async fn authenticated_html_graph_includes_drafts_and_is_not_shared_cached() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(router) = common::maybe_router().await? else {
        return Ok(());
    };
    seed_published_and_draft(&router).await?;

    let response = get_graph(&router, Some("text/html"), Some(SHARED_KEY)).await?;
    assert_eq!(response.status(), StatusCode::OK);
    assert!(header_str(&response, header::CONTENT_TYPE).starts_with("text/html"));
    // An authenticated graph can contain the caller's drafts — it must never be
    // stored where another visitor could be served it.
    let cache_control = header_str(&response, header::CACHE_CONTROL);
    assert!(
        cache_control.contains("no-store"),
        "authenticated HTML graph must be no-store, got {cache_control:?}"
    );
    assert!(
        !cache_control.contains("public"),
        "authenticated HTML graph must not be publicly cacheable"
    );

    let body = to_bytes(response.into_body(), usize::MAX).await?;
    let html = String::from_utf8(body.to_vec())?;
    assert!(
        html.contains("/draft-note"),
        "owner visibility includes the draft node in the HTML graph"
    );
    Ok(())
}
