use anyhow::Result;
use inkwell::config::Config;
use inkwell::db::migrations;
use inkwell::db::pool::create_pool;
use inkwell::http::router::build_router;
use sqlx::PgPool;
use std::sync::Arc;

pub async fn maybe_pool() -> Result<Option<PgPool>> {
    let Ok(database_url) = std::env::var("DATABASE_URL") else {
        return Ok(None);
    };
    let pool = create_pool(&database_url)?;
    migrations::migrate(&pool).await?;
    sqlx::query("TRUNCATE TABLE documents RESTART IDENTITY")
        .execute(&pool)
        .await?;
    Ok(Some(pool))
}

pub fn test_config(database_url: String) -> Arc<Config> {
    Arc::new(Config {
        database_url,
        host: "127.0.0.1".to_string(),
        port: 3000,
        api_key: Some("test-secret-key".to_string()),
        site_url: Some("https://blog.example.com".to_string()),
    })
}

pub async fn maybe_router() -> Result<Option<axum::Router>> {
    let Some(pool) = maybe_pool().await? else {
        return Ok(None);
    };
    Ok(Some(build_router(
        test_config(std::env::var("DATABASE_URL")?),
        pool,
    )))
}
