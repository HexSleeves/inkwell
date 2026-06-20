use axum::{
    Json, Router,
    body::Body,
    http::{HeaderValue, Request, StatusCode, header},
    middleware,
    response::Html,
    routing::get,
};
use serde_json::json;
use tower::ServiceExt;

use inkwell::http::security_headers::apply_security_headers;

fn browser_runtime_src() -> String {
    ["https://cdn.", "tailwind", "css.com"].concat()
}

fn runtime_config_marker() -> String {
    ["tailwind", ".config"].concat()
}

fn permissions_policy() -> HeaderValue {
    HeaderValue::from_static(
        "accelerometer=(), camera=(), geolocation=(), gyroscope=(), magnetometer=(), microphone=(), payment=(), usb=()",
    )
}

#[tokio::test]
async fn html_responses_include_csp_and_hardening_headers() -> anyhow::Result<()> {
    let browser_runtime_src = browser_runtime_src();
    let runtime_config_marker = runtime_config_marker();
    let app = Router::new()
        .route("/", get(|| async { Html("<p>Hello</p>") }))
        .layer(middleware::from_fn(apply_security_headers));

    let response = app
        .oneshot(Request::builder().uri("/").body(Body::empty())?)
        .await?;

    assert_eq!(response.status(), StatusCode::OK);

    let csp = response
        .headers()
        .get(header::CONTENT_SECURITY_POLICY)
        .and_then(|value| value.to_str().ok())
        .expect("missing content-security-policy header");
    assert!(csp.contains("default-src 'self'"));
    assert!(csp.contains("object-src 'none'"));
    assert!(csp.contains("base-uri 'self'"));
    assert!(csp.contains("frame-ancestors 'none'"));
    assert!(csp.contains("img-src 'self' http https"));
    assert!(csp.contains("style-src 'self' 'unsafe-inline'"));
    assert!(csp.contains("script-src 'self' 'nonce-"));
    assert!(!csp.contains(&browser_runtime_src));
    assert!(!csp.contains(&runtime_config_marker));

    assert_eq!(
        response.headers().get(header::X_CONTENT_TYPE_OPTIONS),
        Some(&header::HeaderValue::from_static("nosniff"))
    );
    assert_eq!(
        response.headers().get(header::REFERRER_POLICY),
        Some(&HeaderValue::from_static("strict-origin-when-cross-origin"))
    );
    assert_eq!(
        response.headers().get("permissions-policy"),
        Some(&permissions_policy())
    );

    Ok(())
}

#[tokio::test]
async fn json_responses_keep_hardening_headers_without_csp() -> anyhow::Result<()> {
    let app = Router::new()
        .route("/", get(|| async { Json(json!({ "ok": true })) }))
        .layer(middleware::from_fn(apply_security_headers));

    let response = app
        .oneshot(Request::builder().uri("/").body(Body::empty())?)
        .await?;

    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        response
            .headers()
            .get(header::CONTENT_SECURITY_POLICY)
            .is_none()
    );
    assert_eq!(
        response.headers().get(header::X_CONTENT_TYPE_OPTIONS),
        Some(&header::HeaderValue::from_static("nosniff"))
    );

    Ok(())
}
