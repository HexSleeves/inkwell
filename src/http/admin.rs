//! Admin token-management surface (ADR 0009, plan 023, slice 2).
//!
//! `POST /admin/tokens` mints a scoped token for an author (creating the author
//! on first use), returning the secret **once**. `GET /admin/tokens` lists token
//! metadata (never the secret — it is unrecoverable). `POST
//! /admin/tokens/{prefix}/revoke` revokes a token immediately.
//!
//! These routes are **admin-only from the moment they exist**: minting tokens is
//! a privilege-granting operation, so even though slice 2 defers scope/ownership
//! enforcement on document routes to slice 3, this surface checks for
//! [`Scope::Admin`] up front. Otherwise a `write`-scoped token could mint an
//! `admin` token (privilege escalation). The shared/MCP key carries admin.

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::Serialize;
use serde_json::Value;

use crate::db::tokens;
use crate::domain::author::Scope;
use crate::domain::token;
use crate::error::AppError;
use crate::http::AppState;
use crate::http::auth::require_principal;
use crate::http::extractors::{parse_json_body, require_object};

/// Upper bound on an author name, mirroring the document title cap's spirit:
/// generous for real names, bounded so a request can't store an unbounded blob.
const MAX_AUTHOR_NAME_LENGTH: usize = 200;

/// The minted-token response. `token` is the full secret, shown exactly once.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CreatedToken {
    token: String,
    prefix: String,
    author: String,
    scopes: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TokenListResponse {
    tokens: Vec<tokens::TokenInfo>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RevokeResponse {
    prefix: String,
    revoked: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PruneResponse {
    pruned: u64,
}

/// Query parameters for `GET /admin/tokens`.
#[derive(Default, serde::Deserialize)]
pub struct TokensQuery {
    /// When `true`, include revoked tokens in the listing. Default: omit them.
    all: Option<bool>,
}

/// `GET|POST /admin/tokens` — list tokens, or mint a new one. Admin only.
///
/// `GET /admin/tokens` hides revoked tokens by default; pass `?all=true` to
/// include them.
pub async fn tokens(
    State(state): State<AppState>,
    method: Method,
    headers: HeaderMap,
    query: Query<TokensQuery>,
    body: Bytes,
) -> Result<Response, AppError> {
    require_admin(&headers, &state).await?;
    match method {
        Method::GET => {
            let include_revoked = query.all.unwrap_or(false);
            list_tokens_handler(&state, include_revoked).await
        }
        Method::POST => create_token(&state, body).await,
        _ => Err(AppError::MethodNotAllowed(vec!["GET", "POST"])),
    }
}

/// `POST /admin/tokens/prune` — hard-delete all revoked tokens. Admin only.
///
/// Only rows with `revoked_at IS NOT NULL` are removed; live tokens are
/// untouched. Returns `{ "pruned": <count> }`.
pub async fn prune_tokens(
    State(state): State<AppState>,
    method: Method,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    if method != Method::POST {
        return Err(AppError::MethodNotAllowed(vec!["POST"]));
    }
    require_admin(&headers, &state).await?;
    let pruned = tokens::prune_revoked_tokens(&state.pool).await?;
    Ok((StatusCode::OK, Json(PruneResponse { pruned })).into_response())
}

/// `POST /admin/tokens/{prefix}/revoke` — revoke a token by its prefix. Admin only.
pub async fn revoke_token(
    State(state): State<AppState>,
    Path(prefix): Path<String>,
    method: Method,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    if method != Method::POST {
        return Err(AppError::MethodNotAllowed(vec!["POST"]));
    }
    require_admin(&headers, &state).await?;
    if tokens::revoke_token(&state.pool, &prefix).await? {
        Ok((
            StatusCode::OK,
            Json(RevokeResponse {
                prefix,
                revoked: true,
            }),
        )
            .into_response())
    } else {
        Err(AppError::NotFound(format!(
            "No live token with prefix \"{prefix}\"."
        )))
    }
}

async fn create_token(state: &AppState, body: Bytes) -> Result<Response, AppError> {
    let value = parse_json_body(body)?;
    let map = require_object(value)?;

    let name = match map.get("name") {
        Some(Value::String(name)) if !name.trim().is_empty() => name.trim().to_string(),
        _ => {
            return Err(AppError::BadRequest(
                "Field \"name\" is required and must be a non-empty string.".to_string(),
            ));
        }
    };
    if name.len() > MAX_AUTHOR_NAME_LENGTH {
        return Err(AppError::BadRequest(format!(
            "Field \"name\" must be at most {MAX_AUTHOR_NAME_LENGTH} characters."
        )));
    }
    let scopes = parse_scopes(map.get("scopes"))?;

    let author = tokens::find_or_create_author(&state.pool, &name).await?;
    let minted = token::generate();
    tokens::insert_token(
        &state.pool,
        author.id,
        &minted.prefix,
        &minted.token_hash,
        &scopes,
    )
    .await?;

    Ok((
        StatusCode::CREATED,
        Json(CreatedToken {
            token: minted.token,
            prefix: minted.prefix,
            author: author.name,
            scopes,
        }),
    )
        .into_response())
}

async fn list_tokens_handler(
    state: &AppState,
    include_revoked: bool,
) -> Result<Response, AppError> {
    let tokens = tokens::list_tokens(&state.pool, include_revoked).await?;
    Ok((StatusCode::OK, Json(TokenListResponse { tokens })).into_response())
}

/// Resolve the request's principal and require the `admin` scope.
async fn require_admin(headers: &HeaderMap, state: &AppState) -> Result<(), AppError> {
    let principal = require_principal(headers, &state.config, &state.pool).await?;
    if principal.has(Scope::Admin) {
        Ok(())
    } else {
        Err(AppError::Forbidden(
            "This action requires an admin token.".to_string(),
        ))
    }
}

/// Parse and validate the `scopes` array into canonical scope strings. A
/// non-array, an empty array, or an unknown scope is a client error rather than
/// a silently lower-privilege token (the migration 0012 CHECK is the backstop).
fn parse_scopes(value: Option<&Value>) -> Result<Vec<String>, AppError> {
    let Some(Value::Array(items)) = value else {
        return Err(AppError::BadRequest(
            "Field \"scopes\" is required and must be a non-empty array of: read, write, publish, admin."
                .to_string(),
        ));
    };
    if items.is_empty() {
        return Err(AppError::BadRequest(
            "Field \"scopes\" must contain at least one of: read, write, publish, admin."
                .to_string(),
        ));
    }
    let mut scopes = Vec::with_capacity(items.len());
    for item in items {
        let Value::String(raw) = item else {
            return Err(AppError::BadRequest(
                "Each scope must be a string: read, write, publish, or admin.".to_string(),
            ));
        };
        let scope = Scope::parse(raw).ok_or_else(|| {
            AppError::BadRequest(format!(
                "Unknown scope {raw:?}; must be one of: read, write, publish, admin."
            ))
        })?;
        let canonical = scope.as_str().to_string();
        if !scopes.contains(&canonical) {
            scopes.push(canonical);
        }
    }
    Ok(scopes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn scopes_of(value: Value) -> Result<Vec<String>, AppError> {
        parse_scopes(Some(&value))
    }

    #[test]
    fn parse_scopes_accepts_known_scopes_and_dedupes() {
        let parsed = scopes_of(json!(["write", "publish", "write"])).unwrap();
        assert_eq!(parsed, vec!["write".to_string(), "publish".to_string()]);
    }

    #[test]
    fn parse_scopes_rejects_empty_missing_and_unknown() {
        assert!(matches!(scopes_of(json!([])), Err(AppError::BadRequest(_))));
        assert!(matches!(parse_scopes(None), Err(AppError::BadRequest(_))));
        assert!(matches!(
            scopes_of(json!(["write", "wat"])),
            Err(AppError::BadRequest(_))
        ));
        assert!(matches!(
            scopes_of(json!("write")),
            Err(AppError::BadRequest(_))
        ));
    }
}
