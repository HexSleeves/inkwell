mod common;

use anyhow::Result;
use axum::body::{Body, to_bytes};
use http::{HeaderMap, Method, Request, Response, StatusCode, header};
use inkwell::build_router;
use inkwell::config::Config;
use inkwell::create_pool;
use inkwell::http::cache::{self, PUBLIC_CACHE_CONTROL};
use std::sync::Arc;
use tower::ServiceExt;

async fn send(
    router: &axum::Router,
    method: Method,
    uri: &str,
    body: Body,
    headers: &[(&str, &str)],
) -> Result<Response<Body>> {
    let mut builder = Request::builder().method(method).uri(uri);
    for (name, value) in headers {
        builder = builder.header(*name, *value);
    }
    Ok(router.clone().oneshot(builder.body(body)?).await?)
}

fn router_with_unreachable_database() -> Result<axum::Router> {
    let database_url = "postgres://inkwell:inkwell@no-such-host.invalid/inkwell";
    let pool = create_pool(database_url)?;
    Ok(build_router(
        Arc::new(Config {
            database_url: database_url.to_string(),
            host: "127.0.0.1".to_string(),
            port: 3000,
            api_key: Some("test-secret-key".to_string()),
            site_url: Some("https://blog.example.com".to_string()),
            voyage_api_key: None,
            anthropic_api_key: None,
            llm_model: inkwell::config::DEFAULT_LLM_MODEL.to_string(),
            webmention_send: false,
            browser_login: false,
            write_rate_limit: 0,
            trust_forwarded_headers: false,
            site_title: inkwell::config::DEFAULT_SITE_TITLE.to_string(),
            site_description: None,
            site_author: None,
            custom_css_url: None,
        }),
        pool,
    ))
}

#[tokio::test]
async fn cache_helper_emits_cache_headers_and_body_on_first_response() -> Result<()> {
    let response = cache::html_response(
        &HeaderMap::new(),
        "index",
        StatusCode::OK,
        "<html><body>cached</body></html>".to_string(),
    );

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get(header::CONTENT_TYPE).unwrap(),
        "text/html; charset=utf-8"
    );
    assert_eq!(
        response.headers().get(header::CACHE_CONTROL).unwrap(),
        PUBLIC_CACHE_CONTROL
    );
    assert!(response.headers().get(header::ETAG).is_some());
    assert_eq!(
        to_bytes(response.into_body(), usize::MAX).await?,
        "<html><body>cached</body></html>"
    );

    Ok(())
}

#[tokio::test]
async fn cache_helper_returns_304_without_body_when_etag_matches() -> Result<()> {
    let first = cache::xml_response(
        &HeaderMap::new(),
        "feed",
        StatusCode::OK,
        inkwell::http::feed::ATOM_CONTENT_TYPE,
        "<feed />".to_string(),
    );
    let etag = first
        .headers()
        .get(header::ETAG)
        .unwrap()
        .to_str()?
        .to_string();

    let mut headers = HeaderMap::new();
    headers.insert(header::IF_NONE_MATCH, etag.parse()?);

    let response = cache::xml_response(
        &headers,
        "feed",
        StatusCode::OK,
        inkwell::http::feed::ATOM_CONTENT_TYPE,
        "<feed />".to_string(),
    );

    assert_eq!(response.status(), StatusCode::NOT_MODIFIED);
    assert_eq!(response.headers().get(header::ETAG).unwrap(), etag.as_str());
    assert_eq!(
        response.headers().get(header::CACHE_CONTROL).unwrap(),
        PUBLIC_CACHE_CONTROL
    );
    assert_eq!(to_bytes(response.into_body(), usize::MAX).await?, "");

    Ok(())
}

#[tokio::test]
async fn write_api_responses_do_not_emit_cache_headers() -> Result<()> {
    let router = router_with_unreachable_database()?;

    let response = send(
        &router,
        Method::POST,
        "/documents",
        Body::from(r##"{"title":"No Key","bodyMarkdown":"# Missing"}"##),
        &[("content-type", "application/json")],
    )
    .await?;

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert!(response.headers().get(header::CACHE_CONTROL).is_none());
    assert!(response.headers().get(header::ETAG).is_none());

    Ok(())
}

#[tokio::test]
async fn site_css_asset_is_served_by_the_real_router_without_database() -> Result<()> {
    let router = router_with_unreachable_database()?;

    let response = send(&router, Method::GET, "/assets/site.css", Body::empty(), &[]).await?;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get(header::CONTENT_TYPE).unwrap(),
        "text/css; charset=utf-8"
    );
    assert_eq!(
        response.headers().get(header::CACHE_CONTROL).unwrap(),
        "public, max-age=3600"
    );
    assert!(!to_bytes(response.into_body(), usize::MAX).await?.is_empty());

    Ok(())
}

#[tokio::test]
async fn public_read_routes_return_500_when_database_queries_fail() -> Result<()> {
    let router = router_with_unreachable_database()?;

    for uri in [
        "/",
        "/hello-world",
        "/tags",
        "/tags/rust",
        "/search?q=hello",
        "/search?q=hello&format=json",
        "/feed.xml",
        "/sitemap.xml",
        "/sitemap-static.xml",
        "/sitemaps/documents/1",
        "/sitemaps/tags/1",
    ] {
        let response = send(&router, Method::GET, uri, Body::empty(), &[]).await?;
        assert_eq!(
            response.status(),
            StatusCode::INTERNAL_SERVER_ERROR,
            "{uri}"
        );
        assert!(
            response.headers().get(header::CACHE_CONTROL).is_none(),
            "{uri}"
        );
        assert!(response.headers().get(header::ETAG).is_none(), "{uri}");
    }

    Ok(())
}

#[tokio::test]
async fn html_and_xml_routes_support_conditional_get_when_database_is_available() -> Result<()> {
    let Some(router) = common::maybe_router().await? else {
        return Ok(());
    };

    send(
        &router,
        Method::POST,
        "/documents",
        Body::from(r##"{"title":"Hello World","bodyMarkdown":"# Hi","tags":["rust"]}"##),
        &[
            ("content-type", "application/json"),
            ("x-api-key", "test-secret-key"),
        ],
    )
    .await?;
    let publish = send(
        &router,
        Method::POST,
        "/documents/hello-world/publish",
        Body::empty(),
        &[("x-api-key", "test-secret-key")],
    )
    .await?;
    assert_eq!(publish.status(), StatusCode::OK);

    assert_conditional_get(&router, "/", "text/html; charset=utf-8", b"Hello World").await?;
    assert_conditional_get(
        &router,
        "/feed.xml",
        inkwell::http::feed::ATOM_CONTENT_TYPE,
        b"<feed",
    )
    .await?;
    assert_conditional_get(
        &router,
        "/sitemap.xml",
        inkwell::http::sitemap::SITEMAP_CONTENT_TYPE,
        b"<urlset",
    )
    .await?;
    assert_conditional_get(
        &router,
        "/sitemaps/documents/1",
        inkwell::http::sitemap::SITEMAP_CONTENT_TYPE,
        b"hello-world",
    )
    .await?;
    assert_conditional_get(
        &router,
        "/sitemaps/tags/1",
        inkwell::http::sitemap::SITEMAP_CONTENT_TYPE,
        b"/tags/rust",
    )
    .await?;

    Ok(())
}

async fn assert_conditional_get(
    router: &axum::Router,
    uri: &str,
    content_type: &str,
    body_snippet: &[u8],
) -> Result<()> {
    let first = send(router, Method::GET, uri, Body::empty(), &[]).await?;
    assert_eq!(first.status(), StatusCode::OK, "{uri}");
    assert_eq!(
        first.headers().get(header::CONTENT_TYPE).unwrap(),
        content_type
    );
    assert_eq!(
        first.headers().get(header::CACHE_CONTROL).unwrap(),
        PUBLIC_CACHE_CONTROL
    );
    let etag = first
        .headers()
        .get(header::ETAG)
        .unwrap()
        .to_str()?
        .to_string();
    let body = to_bytes(first.into_body(), usize::MAX).await?;
    assert!(
        body.windows(body_snippet.len())
            .any(|window| window == body_snippet),
        "{uri}"
    );

    let second = send(
        router,
        Method::GET,
        uri,
        Body::empty(),
        &[("if-none-match", etag.as_str())],
    )
    .await?;
    assert_eq!(second.status(), StatusCode::NOT_MODIFIED, "{uri}");
    assert_eq!(second.headers().get(header::ETAG).unwrap(), etag.as_str());
    assert_eq!(
        second.headers().get(header::CACHE_CONTROL).unwrap(),
        PUBLIC_CACHE_CONTROL
    );
    assert_eq!(to_bytes(second.into_body(), usize::MAX).await?, "", "{uri}");

    Ok(())
}
