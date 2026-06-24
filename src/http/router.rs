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

use super::{admin, ai, api, assets, feed, pages, search, security_headers, sitemap, webmention};

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
    let state = AppState {
        config,
        pool,
        embedder,
        llm,
    };
    Router::new()
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
        .route("/admin/tokens", any(admin::tokens))
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
        .route("/", get(pages::index))
        .layer(CompressionLayer::new())
        .layer(TraceLayer::new_for_http())
        .layer(middleware::from_fn(
            security_headers::apply_security_headers,
        ))
        .with_state(state)
}
