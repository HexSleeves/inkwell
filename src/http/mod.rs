use std::sync::Arc;

use sqlx::PgPool;

use crate::ai::{Embedder, Llm};
use crate::config::Config;

pub mod admin;
pub mod ai;
pub mod api;
pub mod assets;
pub mod auth;
pub mod cache;
pub mod extractors;
pub mod feed;
pub mod pages;
pub mod router;
pub mod search;
pub mod security_headers;
pub mod sitemap;
pub mod webmention;
pub mod webmention_send;

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub pool: PgPool,
    /// Embedding provider for note indexing and semantic retrieval. Always set:
    /// the real Voyage embedder when `VOYAGE_API_KEY` is configured, else the
    /// deterministic mock (so search/related/ask all work without keys).
    pub embedder: Arc<dyn Embedder>,
    /// Answer-synthesis provider for ask-your-garden. `None` when
    /// `ANTHROPIC_API_KEY` is unset, in which case `/ask` reports "AI features
    /// not configured" instead of 500ing.
    pub llm: Option<Arc<dyn Llm>>,
}
