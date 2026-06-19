use anyhow::{Result, anyhow};
use sqlx::{Executor, PgPool, Postgres};

pub struct MigrationDef {
    pub version: i64,
    pub description: &'static str,
    pub down_sql: &'static str,
}

pub const MIGRATIONS: [MigrationDef; 3] = [
    MigrationDef {
        version: 1,
        description: "create_documents",
        down_sql: "DROP TABLE IF EXISTS documents;",
    },
    MigrationDef {
        version: 2,
        description: "add_document_status",
        down_sql: "ALTER TABLE documents DROP COLUMN IF EXISTS status;",
    },
    MigrationDef {
        version: 3,
        description: "add_document_tags",
        down_sql: "DROP INDEX IF EXISTS documents_tags_idx; ALTER TABLE documents DROP COLUMN IF EXISTS tags;",
    },
];

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

pub async fn migrate(pool: &PgPool) -> Result<()> {
    MIGRATOR.run(pool).await?;
    Ok(())
}

pub async fn rollback(pool: &PgPool, steps: usize) -> Result<Vec<i64>> {
    let applied = status(pool).await?;
    let targets: Vec<_> = applied.into_iter().rev().take(steps).collect();
    if targets.is_empty() {
        return Ok(Vec::new());
    }

    let mut tx = pool.begin().await?;
    let versions = targets.iter().map(|row| row.version).collect::<Vec<_>>();
    for row in targets {
        let migration = MIGRATIONS
            .iter()
            .find(|migration| migration.version == row.version)
            .ok_or_else(|| anyhow!("no migration definition found for {}", row.version))?;
        tx.execute(sqlx::query(migration.down_sql)).await?;
        tx.execute(
            sqlx::query("DELETE FROM _sqlx_migrations WHERE version = $1").bind(row.version),
        )
        .await?;
    }
    tx.commit().await?;
    Ok(versions)
}

#[derive(Clone, Debug, sqlx::FromRow)]
pub struct AppliedMigration {
    pub version: i64,
    pub description: String,
}

pub async fn status(pool: &PgPool) -> Result<Vec<AppliedMigration>> {
    sqlx::query_as::<Postgres, AppliedMigration>(
        "SELECT version, description FROM _sqlx_migrations ORDER BY version ASC",
    )
    .fetch_all(pool)
    .await
    .map_err(Into::into)
}
