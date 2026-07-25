//! Liveness and readiness probes (CYP-46).
//!
//! Two distinct signals, because an orchestrator needs to tell "this process is
//! wedged, restart it" from "this process is fine but its database isn't yet":
//!
//! - [`liveness`] (`GET /healthz`) touches nothing external. It answers `200`
//!   whenever the HTTP stack can serve, so a transient Postgres outage never
//!   triggers a restart loop.
//! - [`readiness`] (`GET /readyz`, and the legacy `GET /health`) runs
//!   `SELECT 1` under a 1 s timeout and answers `503` when the DB is
//!   unreachable, so a rolling deploy waits for a usable instance.
//!
//! `/health` is retained as an alias of `/readyz` because deploy configs
//! (`railway.json`, compose healthchecks, the runbooks) point at it and its
//! response body is a documented wire contract.

use axum::Json;
use axum::extract::State;
use axum::http::{Method, StatusCode};
use tokio::time::{Duration, timeout};

use crate::error::AppError;
use crate::http::AppState;

/// Upper bound on the readiness DB probe. Short enough that a hung Postgres
/// surfaces as `503` rather than a stuck probe.
const READINESS_DB_TIMEOUT: Duration = Duration::from_millis(1000);

/// `GET /healthz` — liveness. No dependencies consulted.
pub async fn liveness(method: Method) -> Result<(StatusCode, Json<serde_json::Value>), AppError> {
    if method != Method::GET {
        return Err(AppError::MethodNotAllowed(vec!["GET"]));
    }
    Ok((StatusCode::OK, Json(serde_json::json!({"status": "ok"}))))
}

/// `GET /readyz` (and `GET /health`) — readiness. `200` only when Postgres
/// answers within [`READINESS_DB_TIMEOUT`].
pub async fn readiness(
    State(state): State<AppState>,
    method: Method,
) -> Result<(StatusCode, Json<serde_json::Value>), AppError> {
    if method != Method::GET {
        return Err(AppError::MethodNotAllowed(vec!["GET"]));
    }
    let query = timeout(
        READINESS_DB_TIMEOUT,
        sqlx::query("SELECT 1").execute(&state.pool),
    )
    .await;
    if matches!(query, Ok(Ok(_))) {
        Ok((
            StatusCode::OK,
            Json(serde_json::json!({"status": "ok", "db": "up"})),
        ))
    } else {
        Ok((
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"status": "error", "db": "down"})),
        ))
    }
}
