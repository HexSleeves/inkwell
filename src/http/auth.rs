use axum::http::HeaderMap;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

pub fn is_authenticated(headers: &HeaderMap, configured_key: Option<&str>) -> bool {
    let Some(expected) = configured_key else {
        return false;
    };
    if expected.is_empty() {
        return false;
    }
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
    let expected = Sha256::digest(expected.as_bytes());
    provided.ct_eq(&expected).into()
}
