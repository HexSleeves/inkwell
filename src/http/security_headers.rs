use axum::{
    extract::Request,
    http::{
        HeaderValue,
        header::{self, HeaderName},
    },
    middleware::Next,
    response::Response,
};
use uuid::Uuid;

#[derive(Clone, Debug)]
pub struct CspNonce(String);

impl CspNonce {
    pub fn generate() -> Self {
        Self(Uuid::new_v4().simple().to_string())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

pub async fn apply_security_headers(mut request: Request, next: Next) -> Response {
    let nonce = CspNonce::generate();
    request.extensions_mut().insert(nonce.clone());

    let mut response = next.run(request).await;
    let is_html = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.starts_with("text/html"));
    let headers = response.headers_mut();

    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("strict-origin-when-cross-origin"),
    );
    headers.insert(
        HeaderName::from_static("permissions-policy"),
        HeaderValue::from_static(
            "accelerometer=(), camera=(), geolocation=(), gyroscope=(), magnetometer=(), microphone=(), payment=(), usb=()",
        ),
    );

    if is_html {
        let policy = format!(
            "default-src 'self'; object-src 'none'; base-uri 'self'; frame-ancestors 'none'; img-src 'self' http https; style-src 'self' 'unsafe-inline'; script-src 'self' 'nonce-{}'",
            nonce.as_str()
        );
        if let Ok(value) = HeaderValue::from_str(&policy) {
            headers.insert(header::CONTENT_SECURITY_POLICY, value);
        }
    }

    response
}
