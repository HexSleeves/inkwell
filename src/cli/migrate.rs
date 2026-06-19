use anyhow::Result;
use sqlx::PgPool;

use crate::db::migrations;

pub async fn db_migrate(pool: &PgPool) -> Result<()> {
    migrations::migrate(pool).await
}

pub async fn db_rollback(pool: &PgPool, steps: usize) -> Result<()> {
    let _ = migrations::rollback(pool, steps).await?;
    Ok(())
}

pub async fn db_status(pool: &PgPool) -> Result<()> {
    let rows = migrations::status(pool).await?;
    for row in rows {
        println!("{} {}", row.version, row.description);
    }
    Ok(())
}
