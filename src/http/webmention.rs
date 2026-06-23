//! Webmention receive endpoint (card T11, federation P3).
//!
//! `POST /webmention` accepts a form-encoded `source` + `target` per the W3C
//! Webmention spec. The request is validated synchronously and, on success,
//! records a `pending` mention and returns **202 Accepted** — verification (an
//! SSRF-guarded fetch of `source` confirming it links to `target`) runs
//! asynchronously and best-effort, so a slow or hostile source never blocks the
//! request and never 500s it.
//!
//! Validation gates (all before any network touch):
//!   - `target` must be a syntactically valid http(s) URL whose host matches this
//!     site (`INKWELL_SITE_URL`) and whose path resolves to an existing PUBLISHED
//!     note — a draft or unknown target 400s, never revealing draft existence.
//!   - `source` must be a syntactically valid http(s) URL and must not equal the
//!     target (a page can't webmention itself here).

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{Method, StatusCode};
use axum::response::{IntoResponse, Response};
use reqwest::Url;

use crate::db::documents;
use crate::db::webmentions;
use crate::domain::document::{DocumentStatus, StatusFilter};
use crate::error::AppError;
use crate::federation::ssrf::validate_public_url;
use crate::federation::webmention as wm;
use crate::http::AppState;

/// `POST /webmention` — receive a Webmention. Form-encoded `source` + `target`.
///
/// Returns 202 when the mention is accepted and queued for verification; 400 for
/// any validation failure (bad URL, target not on this site, target not a
/// published note, source == target). GET/other methods are 405.
pub async fn webmention(
    State(state): State<AppState>,
    method: Method,
    body: Bytes,
) -> Result<Response, AppError> {
    if method != Method::POST {
        return Err(AppError::MethodNotAllowed(vec!["POST"]));
    }

    let (source, target) = parse_source_target(&body)?;

    // `source` must be a syntactically valid http(s) URL. We do NOT fetch it
    // here — that happens in async verification, behind the SSRF guard.
    let source_url = validate_public_url(&source)
        .map_err(|err| AppError::BadRequest(format!("Invalid \"source\": {err}.")))?;

    // `target` must be a valid http(s) URL on THIS site.
    let target_url = validate_public_url(&target)
        .map_err(|err| AppError::BadRequest(format!("Invalid \"target\": {err}.")))?;

    if source_url == target_url {
        return Err(AppError::BadRequest(
            "\"source\" and \"target\" must differ.".to_string(),
        ));
    }

    let site_url = state.config.site_url.as_deref().ok_or_else(|| {
        // Without a configured site URL we cannot safely decide what "this site"
        // is, so we refuse rather than guess.
        AppError::BadRequest("This site does not accept Webmentions.".to_string())
    })?;
    if !target_is_on_site(&target_url, site_url) {
        return Err(AppError::BadRequest(
            "\"target\" is not a URL on this site.".to_string(),
        ));
    }

    // Resolve the target path to a PUBLISHED note. A draft/unknown target 400s
    // with the same message, so receiving a mention never reveals whether a
    // draft exists at that path.
    let target_note = match target_slug(&target_url) {
        Some(slug) => {
            documents::get_document_by_slug(
                &state.pool,
                &slug,
                StatusFilter {
                    status: Some(DocumentStatus::Published),
                },
            )
            .await?
        }
        None => None,
    };
    let Some(target_note) = target_note else {
        return Err(AppError::BadRequest(
            "\"target\" does not resolve to a published note on this site.".to_string(),
        ));
    };

    // Record the pending mention. The canonical source/target strings are the
    // normalized (parsed) URLs so re-sends collapse onto the same row.
    let source_canonical = source_url.to_string();
    let target_canonical = target_url.to_string();
    let mention_id =
        webmentions::upsert_pending(&state.pool, &source_canonical, target_note.id).await?;

    // Verify asynchronously, best-effort: fetch the source (through the SSRF
    // guard) and confirm it links to the target, then flip to verified; on any
    // failure, drop the pending row. This never blocks the 202 and never 500s.
    let pool = state.pool.clone();
    tokio::spawn(async move {
        match wm::verify_source_links_to_target(&source_canonical, &target_canonical).await {
            Ok(true) => {
                if let Err(error) = webmentions::mark_verified(&pool, mention_id).await {
                    tracing::warn!(%error, mention_id = %mention_id, "failed to mark webmention verified");
                }
            }
            Ok(false) => {
                tracing::info!(mention_id = %mention_id, "webmention source does not link to target; dropping");
                drop_mention(&pool, mention_id).await;
            }
            Err(error) => {
                tracing::info!(%error, mention_id = %mention_id, "webmention source verification failed; dropping");
                drop_mention(&pool, mention_id).await;
            }
        }
    });

    Ok(StatusCode::ACCEPTED.into_response())
}

/// Drop an unverifiable mention, logging (but never surfacing) a DB error.
async fn drop_mention(pool: &sqlx::PgPool, id: uuid::Uuid) {
    if let Err(error) = webmentions::delete_mention(pool, id).await {
        tracing::warn!(%error, mention_id = %id, "failed to drop unverified webmention");
    }
}

/// Parse the `source` and `target` fields from a form-encoded body. Both are
/// required and non-empty; a missing/blank field is a 400.
fn parse_source_target(body: &Bytes) -> Result<(String, String), AppError> {
    let mut source = None;
    let mut target = None;
    for (key, value) in form_urlencoded::parse(body) {
        match key.as_ref() {
            "source" if source.is_none() => source = Some(value.into_owned()),
            "target" if target.is_none() => target = Some(value.into_owned()),
            _ => {}
        }
    }
    let source = source
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| AppError::BadRequest("Form field \"source\" is required.".to_string()))?;
    let target = target
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| AppError::BadRequest("Form field \"target\" is required.".to_string()))?;
    Ok((source, target))
}

/// Whether `target` is a URL on this site, i.e. its host (and port, if any)
/// matches the configured `INKWELL_SITE_URL`. Scheme is intentionally NOT
/// compared so an `http`/`https` mismatch between config and request still
/// matches the site; the host comparison is case-insensitive.
fn target_is_on_site(target: &Url, site_url: &str) -> bool {
    let Ok(site) = Url::parse(site_url) else {
        return false;
    };
    let (Some(site_host), Some(target_host)) = (site.host_str(), target.host_str()) else {
        return false;
    };
    if !site_host.eq_ignore_ascii_case(target_host) {
        return false;
    }
    // Compare EXPLICIT ports so blog.example.com:8080 ≠ blog.example.com. Scheme
    // is deliberately not compared (an http/https mismatch between config and
    // request still matches the site), so comparing known-default ports would
    // wrongly reject an http request against an https site URL — compare only the
    // ports actually present in each URL.
    site.port() == target.port()
}

/// Extract the note slug a target URL points at. The public note route is
/// `/{slug}` (see the router), so the slug is the single non-empty path segment.
/// A multi-segment or empty path yields `None` (no resolvable note).
fn target_slug(target: &Url) -> Option<String> {
    let mut segments = target.path_segments()?.filter(|s| !s.is_empty());
    let slug = segments.next()?;
    if segments.next().is_some() {
        // More than one path segment ⇒ not a top-level note URL.
        return None;
    }
    Some(slug.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(s: &str) -> Url {
        Url::parse(s).unwrap()
    }

    #[test]
    fn target_is_on_site_matches_host_and_port() {
        let site = "https://blog.example.com";
        assert!(target_is_on_site(
            &url("https://blog.example.com/hello"),
            site
        ));
        // Scheme differences are tolerated.
        assert!(target_is_on_site(
            &url("http://blog.example.com/hello"),
            site
        ));
        // Case-insensitive host.
        assert!(target_is_on_site(
            &url("https://BLOG.example.com/hello"),
            site
        ));
    }

    #[test]
    fn target_is_on_site_rejects_other_hosts_and_ports() {
        let site = "https://blog.example.com";
        assert!(!target_is_on_site(
            &url("https://evil.example.com/hello"),
            site
        ));
        assert!(!target_is_on_site(
            &url("https://blog.example.com.evil.com/x"),
            site
        ));
        assert!(!target_is_on_site(
            &url("https://blog.example.com:8080/hello"),
            site
        ));
    }

    #[test]
    fn target_slug_extracts_single_segment() {
        assert_eq!(
            target_slug(&url("https://blog.example.com/hello")).as_deref(),
            Some("hello")
        );
        assert_eq!(
            target_slug(&url("https://blog.example.com/hello/")).as_deref(),
            Some("hello")
        );
    }

    #[test]
    fn target_slug_rejects_root_and_nested_paths() {
        assert_eq!(target_slug(&url("https://blog.example.com/")), None);
        assert_eq!(target_slug(&url("https://blog.example.com")), None);
        // Nested paths are not top-level notes (e.g. /tags/foo, /page/2).
        assert_eq!(target_slug(&url("https://blog.example.com/tags/foo")), None);
    }

    #[test]
    fn parse_source_target_requires_both_fields() {
        let body = Bytes::from("source=https://a.example/post");
        assert!(parse_source_target(&body).is_err());
        let body = Bytes::from("target=https://blog.example.com/x");
        assert!(parse_source_target(&body).is_err());
        let body = Bytes::from("source=https://a.example/post&target=https://blog.example.com/x");
        let (s, t) = parse_source_target(&body).unwrap();
        assert_eq!(s, "https://a.example/post");
        assert_eq!(t, "https://blog.example.com/x");
    }
}
