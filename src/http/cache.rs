use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use sha2::{Digest, Sha256};

pub const HTML_CONTENT_TYPE: &str = "text/html; charset=utf-8";
pub const PUBLIC_CACHE_CONTROL: &str = "public, max-age=60, stale-while-revalidate=300";

pub fn html_response(
    request_headers: &HeaderMap,
    route_key: &str,
    status: StatusCode,
    html: String,
) -> Response {
    cached_response(request_headers, route_key, status, HTML_CONTENT_TYPE, html)
}

pub fn xml_response(
    request_headers: &HeaderMap,
    route_key: &str,
    status: StatusCode,
    content_type: &'static str,
    xml: String,
) -> Response {
    cached_response(request_headers, route_key, status, content_type, xml)
}

fn cached_response(
    request_headers: &HeaderMap,
    route_key: &str,
    status: StatusCode,
    content_type: &'static str,
    body: String,
) -> Response {
    let etag = build_etag(route_key, body.as_bytes());
    let etag_header = HeaderValue::from_str(&etag).expect("generated etag is valid");

    if status == StatusCode::OK && request_has_matching_etag(request_headers, &etag) {
        return (
            StatusCode::NOT_MODIFIED,
            [
                (header::ETAG, etag_header),
                (
                    header::CACHE_CONTROL,
                    HeaderValue::from_static(PUBLIC_CACHE_CONTROL),
                ),
            ],
        )
            .into_response();
    }

    (
        status,
        [
            (header::CONTENT_TYPE, HeaderValue::from_static(content_type)),
            (header::ETAG, etag_header),
            (
                header::CACHE_CONTROL,
                HeaderValue::from_static(PUBLIC_CACHE_CONTROL),
            ),
        ],
        body,
    )
        .into_response()
}

fn build_etag(route_key: &str, body: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(route_key.as_bytes());
    hasher.update([0]);
    hasher.update(body);

    let digest = hasher.finalize();
    let mut value = String::with_capacity((digest.len() * 2) + 2);
    value.push('"');
    for byte in digest {
        value.push_str(&format!("{byte:02x}"));
    }
    value.push('"');
    value
}

fn request_has_matching_etag(headers: &HeaderMap, etag: &str) -> bool {
    let weak_etag = format!("W/{etag}");
    headers
        .get(header::IF_NONE_MATCH)
        .and_then(|value| value.to_str().ok())
        .map(|value| {
            value
                .split(',')
                .map(str::trim)
                .any(|candidate| candidate == "*" || candidate == etag || candidate == weak_etag)
        })
        .unwrap_or(false)
}
