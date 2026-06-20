use std::sync::Arc;

use axum::Router;
use axum::routing::{any, get};
use tower_http::compression::CompressionLayer;
use tower_http::trace::TraceLayer;

use crate::config::Config;
use crate::http::AppState;

use super::{api, feed, pages, search, sitemap};

pub fn build_router(config: Arc<Config>, pool: sqlx::PgPool) -> Router {
    let state = AppState { config, pool };
    Router::new()
        .route("/health", any(api::health))
        .route("/documents", any(api::documents))
        .route("/documents/{slug}", any(api::document))
        .route("/documents/{slug}/publish", any(api::publish_document))
        .route("/documents/{slug}/unpublish", any(api::unpublish_document))
        .route("/feed.xml", get(feed::feed))
        .route("/sitemap.xml", get(sitemap::sitemap))
        .route("/sitemap-static.xml", get(sitemap::sitemap_static))
        .route(
            "/sitemaps/documents/{page}",
            get(sitemap::sitemap_documents_page),
        )
        .route("/sitemaps/tags/{page}", get(sitemap::sitemap_tags_page))
        .route("/search", get(search::search))
        .route("/tags", get(pages::tags_index))
        .route("/tags/{tag}", get(pages::tag_page))
        .route("/tags/{tag}/page/{page}", get(pages::tag_page_numbered))
        .route("/page/{page}", get(pages::index_page))
        .route("/{slug}", get(pages::document_page))
        .route("/", get(pages::index))
        .layer(CompressionLayer::new())
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}
