use axum::body::Bytes;
use serde_json::Value;

use crate::error::AppError;

pub fn parse_json_body(body: Bytes) -> Result<Value, AppError> {
    serde_json::from_slice::<Value>(&body)
        .map_err(|_| AppError::BadRequest("Request body must be valid JSON.".to_string()))
}

pub fn require_object(value: Value) -> Result<serde_json::Map<String, Value>, AppError> {
    match value {
        Value::Object(map) => Ok(map),
        _ => Err(AppError::BadRequest(
            "Request body must be a JSON object.".to_string(),
        )),
    }
}

pub fn parse_non_negative_int(value: Option<&str>, field: &str) -> Result<Option<u32>, AppError> {
    let Some(value) = value else {
        return Ok(None);
    };
    if !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(AppError::BadRequest(format!(
            r#"Query param \"{}\" must be a non-negative integer."#,
            field
        )));
    }
    value.parse::<u32>().map(Some).map_err(|_| {
        AppError::BadRequest(format!(
            r#"Query param \"{}\" must be a non-negative integer."#,
            field
        ))
    })
}
