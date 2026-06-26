//! Request correlation IDs (CIL-125).
//!
//! A single [`from_fn`](axum::middleware::from_fn) middleware assigns every
//! request a correlation id so a user-reported error can be traced back to its
//! logs. The id is:
//!
//! - taken from an inbound `X-Request-Id` header when present and well-formed,
//!   otherwise freshly generated as a UUID v4;
//! - stashed in a task-local for the duration of the request, so the error
//!   envelope ([`crate::error::AppError`]) and the tracing span can read it
//!   without threading it through every handler signature;
//! - echoed back on every response via the `X-Request-Id` header.
//!
//! The middleware sits OUTSIDE `TraceLayer` in the stack so the id is already in
//! scope when `TraceLayer`'s span is built — `make_span_with` calls
//! [`current`] to add `request_id` to the span, and every log emitted while
//! handling the request inherits it. We do not duplicate request logging here;
//! `TraceLayer` still owns the request/response log lines.

use axum::{
    extract::Request,
    http::{HeaderName, HeaderValue},
    middleware::Next,
    response::Response,
};
use uuid::Uuid;

/// The correlation-id header, used for both the inbound read and the echoed
/// response value.
pub const X_REQUEST_ID: HeaderName = HeaderName::from_static("x-request-id");

/// Upper bound on an accepted inbound id. Long enough for a UUID or a typical
/// trace id, short enough to keep logs and headers bounded.
const MAX_LEN: usize = 128;

tokio::task_local! {
    /// The current request's correlation id. Set by [`propagate_request_id`]
    /// for the lifetime of the request; read by [`current`].
    static REQUEST_ID: String;
}

/// The correlation id of the in-flight request, or `None` when called outside a
/// request scope (e.g. startup/background code).
pub fn current() -> Option<String> {
    REQUEST_ID.try_with(String::clone).ok()
}

/// An inbound id is accepted only when it is a short, non-empty token of ASCII
/// alphanumerics, `-`, or `_`. This both rejects junk and prevents header/log
/// injection from an attacker-supplied value.
fn is_well_formed(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_LEN
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// Honour a well-formed inbound `X-Request-Id`, else mint a UUID v4.
fn resolve_request_id(request: &Request) -> String {
    request
        .headers()
        .get(X_REQUEST_ID)
        .and_then(|value| value.to_str().ok())
        .filter(|value| is_well_formed(value))
        .map(str::to_owned)
        .unwrap_or_else(|| Uuid::new_v4().to_string())
}

/// Middleware: assign/propagate the correlation id, expose it via the
/// task-local for the duration of the request, and echo it on the response.
pub async fn propagate_request_id(request: Request, next: Next) -> Response {
    // `request_id` is always header-safe here (a UUID or a validated token). If
    // that invariant ever breaks, regenerate a fresh UUID and use it for BOTH
    // the header and the task-local, so the header, error body, and logs never
    // disagree on the id (and the fallback can't re-introduce injection).
    let (request_id, header_value) = {
        let candidate = resolve_request_id(&request);
        match HeaderValue::from_str(&candidate) {
            Ok(value) => (candidate, value),
            Err(_) => {
                let fresh = Uuid::new_v4().to_string();
                let value = HeaderValue::from_str(&fresh)
                    .expect("a UUID v4 string is always a valid header value");
                (fresh, value)
            }
        }
    };

    REQUEST_ID
        .scope(request_id, async move {
            let mut response = next.run(request).await;
            response.headers_mut().insert(X_REQUEST_ID, header_value);
            response
        })
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_uuid_and_typical_trace_ids() {
        assert!(is_well_formed("550e8400-e29b-41d4-a716-446655440000"));
        assert!(is_well_formed("trace_id-123"));
        assert!(is_well_formed("A"));
    }

    #[test]
    fn rejects_empty_overlong_or_unsafe_ids() {
        assert!(!is_well_formed(""));
        assert!(!is_well_formed(&"a".repeat(MAX_LEN + 1)));
        // Spaces, control chars, and header/log-injection vectors are rejected.
        assert!(!is_well_formed("has space"));
        assert!(!is_well_formed("inject\r\nX-Evil: 1"));
        assert!(!is_well_formed("semi;colon"));
    }

    #[test]
    fn current_is_none_outside_a_request_scope() {
        assert_eq!(current(), None);
    }
}
