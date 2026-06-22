use axum::http::HeaderMap;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

/// Authenticate a write request against the configured credentials.
///
/// A request is authenticated when its single `x-api-key` header matches
/// **either** the configured `api_key` (the human authoring key) **or** the
/// configured `mcp_key` (the MCP server's separate, independently
/// grant/revocable credential). Comparison is constant-time against each
/// configured key. The single-header and non-empty-key rules are preserved:
/// missing, duplicated, or non-ASCII headers fail, as does an empty configured
/// key (a blank key never authenticates).
pub fn is_authenticated(headers: &HeaderMap, api_key: Option<&str>, mcp_key: Option<&str>) -> bool {
    // Reject anything but exactly one `x-api-key` header.
    let values = headers.get_all("x-api-key");
    let mut iter = values.iter();
    let Some(value) = iter.next() else {
        return false;
    };
    if iter.next().is_some() {
        return false;
    }
    let Ok(provided) = value.to_str() else {
        return false;
    };
    let provided = Sha256::digest(provided.as_bytes());

    // Accept the request if the provided key matches any configured credential.
    // Both candidates are checked so the comparison cost doesn't reveal which
    // (if any) key was configured.
    let mut matched = false;
    for candidate in [api_key, mcp_key].into_iter().flatten() {
        if candidate.is_empty() {
            continue;
        }
        let expected = Sha256::digest(candidate.as_bytes());
        matched |= bool::from(provided.ct_eq(&expected));
    }
    matched
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn headers_with_key(key: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert("x-api-key", HeaderValue::from_str(key).unwrap());
        headers
    }

    #[test]
    fn accepts_either_the_api_key_or_the_mcp_key() {
        let headers = headers_with_key("author-key");
        assert!(is_authenticated(
            &headers,
            Some("author-key"),
            Some("mcp-key")
        ));

        let headers = headers_with_key("mcp-key");
        assert!(is_authenticated(
            &headers,
            Some("author-key"),
            Some("mcp-key")
        ));
    }

    #[test]
    fn rejects_a_key_matching_neither_credential() {
        let headers = headers_with_key("wrong");
        assert!(!is_authenticated(
            &headers,
            Some("author-key"),
            Some("mcp-key")
        ));
    }

    #[test]
    fn mcp_key_works_even_when_no_api_key_is_configured() {
        let headers = headers_with_key("mcp-key");
        assert!(is_authenticated(&headers, None, Some("mcp-key")));
    }

    #[test]
    fn empty_configured_keys_never_authenticate() {
        let headers = headers_with_key("");
        assert!(!is_authenticated(&headers, Some(""), Some("")));
        assert!(!is_authenticated(&headers, None, None));
    }

    #[test]
    fn rejects_missing_or_duplicated_header() {
        let empty = HeaderMap::new();
        assert!(!is_authenticated(&empty, Some("k"), None));

        let mut dup = HeaderMap::new();
        dup.append("x-api-key", HeaderValue::from_static("k"));
        dup.append("x-api-key", HeaderValue::from_static("k"));
        assert!(!is_authenticated(&dup, Some("k"), None));
    }
}
