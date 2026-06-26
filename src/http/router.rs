use std::sync::Arc;

use axum::Router;
use axum::middleware;
use axum::routing::{any, get};
use tower_http::compression::CompressionLayer;
use tower_http::trace::TraceLayer;

use crate::ai as semantic;
use crate::ai::{Embedder, Llm};
use crate::config::Config;
use crate::http::AppState;

use super::{
    admin, ai, api, assets, auth_session, feed, media, pages, rate_limit, request_id, search,
    security_headers, sitemap, webmention,
};

pub fn build_router(config: Arc<Config>, pool: sqlx::PgPool) -> Router {
    // Provider selection from config: real providers when keys are set, else the
    // deterministic mock embedder and a `None` LLM (the no-key fallbacks).
    let embedder = semantic::build_embedder(&config);
    let llm = semantic::build_llm(&config);
    build_router_with_providers(config, pool, embedder, llm)
}

/// Build the router with explicit AI providers, bypassing config-based selection.
///
/// Production goes through [`build_router`]; this exists so the eval suite can
/// wire in the deterministic [`MockEmbedder`](crate::ai::MockEmbedder) /
/// [`MockLlm`](crate::ai::MockLlm) and exercise the RAG surfaces end-to-end
/// without any API keys.
pub fn build_router_with_providers(
    config: Arc<Config>,
    pool: sqlx::PgPool,
    embedder: Arc<dyn Embedder>,
    llm: Option<Arc<dyn Llm>>,
) -> Router {
    let browser_login = config.browser_login;
    // One shared GCRA limiter for the whole process. Built before `state` moves
    // `config`; `None` internally when `write_rate_limit == 0` (disabled).
    let rate_limiter = Arc::new(rate_limit::RateLimitState::new(
        config.write_rate_limit,
        config.browser_login,
    ));
    let state = AppState {
        config,
        pool,
        embedder,
        llm,
    };
    let mut router = Router::new()
        .route("/health", any(api::health))
        .route("/ask", any(ai::ask))
        .route("/webmention", any(webmention::webmention))
        .route("/documents", any(api::documents))
        .route("/documents/{slug}", any(api::document))
        .route("/documents/{slug}/backlinks", any(api::document_backlinks))
        .route("/documents/{slug}/graph", any(api::document_graph))
        .route("/documents/{slug}/related", any(ai::document_related))
        .route("/graph", any(api::graph))
        .route("/documents/{slug}/publish", any(api::publish_document))
        .route("/documents/{slug}/unpublish", any(api::unpublish_document))
        // Raise the body limit on upload to MAX_MEDIA_BYTES; axum's default 2 MiB
        // `DefaultBodyLimit` would otherwise reject 2–5 MiB uploads before the
        // handler's own size check runs.
        .route(
            "/media",
            any(media::media_upload)
                .layer(axum::extract::DefaultBodyLimit::max(media::MAX_MEDIA_BYTES)),
        )
        // `get(...)` so axum answers HEAD automatically.
        .route("/media/{id}", get(media::media_serve))
        .route("/admin/tokens", any(admin::tokens))
        .route("/admin/tokens/prune", any(admin::prune_tokens))
        .route("/admin/tokens/{prefix}/revoke", any(admin::revoke_token))
        .route("/feed.xml", get(feed::feed))
        .route("/sitemap.xml", get(sitemap::sitemap))
        .route("/sitemap-static.xml", get(sitemap::sitemap_static))
        .route(
            "/sitemaps/documents/{page}",
            get(sitemap::sitemap_documents_page),
        )
        .route("/sitemaps/tags/{page}", get(sitemap::sitemap_tags_page))
        .route("/assets/fonts/nunito.woff2", get(assets::nunito_font))
        .route("/search", get(search::search))
        .route("/tags", get(pages::tags_index))
        .route("/tags/{tag}", get(pages::tag_page))
        .route("/tags/{tag}/page/{page}", get(pages::tag_page_numbered))
        .route("/page/{page}", get(pages::index_page))
        .route("/{slug}", get(pages::document_page))
        .route("/", get(pages::index));

    // Register login/logout routes only when the flag is on. When off, any
    // request to /auth/* hits the fallback and returns 404 — the routes do not
    // exist, so no auth surface is exposed.
    if browser_login {
        router = router
            .route("/auth/login", any(auth_session::login))
            .route("/auth/logout", any(auth_session::logout));
    }

    router
        .layer(CompressionLayer::new())
        // Add the correlation id to the per-request span so EVERY log line for
        // the request carries `request_id`. The id is read from the task-local
        // populated by `propagate_request_id`, which sits outside this layer and
        // is therefore already in scope when the span is built.
        .layer(
            TraceLayer::new_for_http().make_span_with(|request: &axum::extract::Request| {
                let request_id = request_id::current().unwrap_or_default();
                tracing::info_span!(
                    "http_request",
                    method = %request.method(),
                    uri = %request.uri(),
                    %request_id,
                )
            }),
        )
        // Rate limiting sits inside the security-headers layer so a 429 still
        // gets the standard security headers, but outside the handlers so an
        // over-limit write is rejected before any DB or AI work runs.
        .layer(middleware::from_fn_with_state(
            rate_limiter,
            rate_limit::rate_limit,
        ))
        .layer(middleware::from_fn(
            security_headers::apply_security_headers,
        ))
        // Outermost app layer: assign/propagate the correlation id before any
        // other layer (notably TraceLayer) runs, and echo it on the response.
        .layer(middleware::from_fn(request_id::propagate_request_id))
        .with_state(state)
}
