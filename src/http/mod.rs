use std::sync::Arc;

use sqlx::PgPool;

use crate::config::Config;

pub mod api;
pub mod auth;
pub mod cache;
pub mod extractors;
pub mod feed;
pub mod pages;
pub mod router;
pub mod search;
pub mod sitemap;

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub pool: PgPool,
}
