use std::sync::Arc;

use axum::Router;
use axum::middleware;
use axum::routing::{any, get};
use tower_http::compression::CompressionLayer;

use crate::ai as semantic;
use crate::ai::{Embedder, Llm};
use crate::config::Config;
use crate::http::AppState;
use crate::http::metrics::Metrics;

use super::{
    admin, ai, assets, auth_session, documents, editor, feed, graph, health, media, observability,
    pages, preview, publish, rate_limit, request_id, search, security_headers, settings, sitemap,
    webmention,
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
    let metrics_enabled = config.metrics_enabled;
    // One registry per router, shared by the recording middleware and the
    // `/metrics` handler. Built even when the endpoint is off so enabling it is
    // a pure exposure change, not a behavior change.
    let metrics = Arc::new(Metrics::new());
    // One shared GCRA limiter for the whole process. It validates credentials
    // via `authenticate`, so it gets clones of `config`/`pool` before `state`
    // takes ownership. Internally a no-op when `write_rate_limit == 0`.
    let rate_limiter = Arc::new(rate_limit::RateLimitState::new(
        config.clone(),
        pool.clone(),
    ));
    // Body limit for `POST /media`: axum's 2 MiB default would reject a legal
    // upload before the handler's own check runs, so the route carries the
    // configured cap. Read before `state` takes ownership of `config`.
    let media_max_bytes = config.media_max_bytes;
    let media_store = crate::media::build_store(&config, &pool);
    let state = AppState {
        config,
        pool,
        embedder,
        llm,
        metrics: metrics.clone(),
        media_store,
    };
    let mut router = Router::new()
        // `/health` predates the liveness/readiness split and stays wired to the
        // readiness probe: deploy configs and runbooks point at it and its body
        // is a documented wire contract.
        .route("/health", any(health::readiness))
        .route("/healthz", any(health::liveness))
        .route("/readyz", any(health::readiness))
        .route("/ask", any(ai::ask))
        .route("/webmention", any(webmention::webmention))
        .route("/documents", any(documents::documents))
        .route("/documents/{slug}", any(documents::document))
        .route(
            "/documents/{slug}/backlinks",
            any(graph::document_backlinks),
        )
        .route("/documents/{slug}/graph", any(graph::document_graph))
        .route(
            "/documents/{slug}/history",
            any(documents::document_history),
        )
        .route("/documents/{slug}/related", any(ai::document_related))
        .route("/graph", any(graph::graph))
        .route("/documents/{slug}/publish", any(publish::publish_document))
        .route(
            "/documents/{slug}/unpublish",
            any(publish::unpublish_document),
        )
        // Raise the body limit on upload to the configured cap; axum's default
        // 2 MiB `DefaultBodyLimit` would otherwise reject legal uploads before
        // the handler's own size check runs.
        .route(
            "/media",
            any(media::media_upload).layer(axum::extract::DefaultBodyLimit::max(media_max_bytes)),
        )
        // `get(...)` so axum answers HEAD automatically; `delete(...)` adds the
        // author-scoped delete without opening the other verbs.
        .route(
            "/media/{id}",
            get(media::media_serve).delete(media::media_delete),
        )
        .route(
            "/documents/{slug}/preview-tokens",
            any(preview::preview_tokens),
        )
        .route(
            "/documents/{slug}/preview-tokens/{prefix}",
            any(preview::revoke_preview_token),
        )
        .route("/documents/{slug}/preview", any(preview::preview_document))
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
        .route("/assets/site.css", get(assets::site_css))
        .route("/assets/fonts/nunito.woff2", get(assets::nunito_font))
        .route("/search", get(search::search))
        .route("/notes", get(pages::notes_index))
        .route("/settings", get(settings::settings))
        .route("/tags", get(pages::tags_index))
        .route("/tags/{tag}", get(pages::tag_page))
        .route("/tags/{tag}/page/{page}", get(pages::tag_page_numbered))
        .route("/archive", get(pages::archive_index))
        .route("/archive/{year}/{month}", get(pages::archive_month))
        .route(
            "/archive/{year}/{month}/page/{page}",
            get(pages::archive_month_page),
        )
        .route("/page/{page}", get(pages::index_page))
        .route("/{slug}", get(pages::document_page))
        .route("/", get(pages::index));

    // `/metrics` exists only when `INKWELL_METRICS_ENABLED=true`, so a default
    // install exposes no scrape surface at all (the path falls through to the
    // `/{slug}` document route, i.e. a 404 unless a note owns that slug). When
    // on, `INKWELL_METRICS_TOKEN` gates it. See ADR 0012.
    if metrics_enabled {
        router = router.route("/metrics", any(observability::metrics));
    }

    // Register the login page + login/logout routes only when the flag is on.
    // When off, `/auth/*` hits the fallback and `/login` falls through to the
    // `/{slug}` document route (a 404 unless a doc owns that slug) — no auth
    // surface is exposed either way.
    if browser_login {
        router = router
            .route("/login", get(auth_session::login_page))
            .route("/media/new", get(media::media_new_page))
            .route("/auth/login", any(auth_session::login))
            .route("/auth/logout", any(auth_session::logout))
            // Authoring web UI (CYP-42). Static segments, so matchit prefers the
            // `/editor*` routes over the public `/{slug}` catch-all, and `/editor/new`
            // over `/editor/{slug}`. The pages drive the existing `/documents` JSON
            // API; auth/scope are enforced there, not by these HTML shells.
            .route("/editor", get(editor::editor_list_page))
            .route("/editor/new", get(editor::editor_new_page))
            .route("/editor/{slug}", get(editor::editor_edit_page));
    }

    router
        .layer(CompressionLayer::new())
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
        // Observability wraps the rate limiter and security headers so a 429 or
        // an error response is still counted and logged with its final status.
        // It builds the per-request span — so EVERY log line for the request
        // carries `request_id` and the route template — and emits the single
        // request-completed event. It reads the correlation id from the
        // task-local populated by `propagate_request_id`, which sits outside it
        // and is therefore already in scope.
        .layer(middleware::from_fn_with_state(
            metrics,
            observability::observe,
        ))
        // Outermost app layer: assign/propagate the correlation id before any
        // other layer runs, and echo it on the response.
        .layer(middleware::from_fn(request_id::propagate_request_id))
        .with_state(state)
}
