use anyhow::{Result, anyhow};
use sqlx::{Executor, PgPool, Postgres};

pub struct MigrationDef {
    pub version: i64,
    pub description: &'static str,
    pub down_sql: &'static str,
}

pub const MIGRATIONS: [MigrationDef; 6] = [
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
    MigrationDef {
        version: 4,
        description: "add_documents_list_index",
        down_sql: "DROP INDEX IF EXISTS documents_status_created_at_id_idx;",
    },
    MigrationDef {
        version: 5,
        description: "create_links",
        down_sql: "DROP TABLE IF EXISTS links;",
    },
    MigrationDef {
        version: 6,
        description: "add_document_version",
        down_sql: "ALTER TABLE documents DROP COLUMN IF EXISTS version;",
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

#[cfg(test)]
mod tests {
    use super::MIGRATIONS;
    use std::fs;

    #[test]
    fn includes_documents_list_index_migration_definition() {
        let migration = MIGRATIONS
            .iter()
            .find(|migration| migration.version == 4)
            .expect("version 4 migration definition should exist");

        assert_eq!(migration.description, "add_documents_list_index");
        assert_eq!(
            migration.down_sql,
            "DROP INDEX IF EXISTS documents_status_created_at_id_idx;"
        );
        assert_eq!(
            fs::read_to_string("migrations/0004_add_documents_list_index.sql")
                .expect("migration 0004 should exist"),
            "CREATE INDEX IF NOT EXISTS documents_status_created_at_id_idx\n    ON documents (status, created_at DESC, id DESC);\n"
        );
    }

    #[test]
    fn includes_create_links_migration_definition() {
        let migration = MIGRATIONS
            .iter()
            .find(|migration| migration.version == 5)
            .expect("version 5 migration definition should exist");

        assert_eq!(migration.description, "create_links");
        assert_eq!(migration.down_sql, "DROP TABLE IF EXISTS links;");

        let sql = fs::read_to_string("migrations/0005_create_links.sql")
            .expect("migration 0005 should exist");
        assert!(sql.contains("CREATE TABLE IF NOT EXISTS links"));
        assert!(
            sql.contains(
                "source_note_id uuid NOT NULL REFERENCES documents (id) ON DELETE CASCADE"
            )
        );
        assert!(sql.contains("target_kind IN ('internal', 'external')"));
        assert!(sql.contains("link_type IN ('wikilink', 'embed')"));
        assert!(sql.contains("links_target_note_id_idx"));
        assert!(sql.contains("links_unresolved_target_text_idx"));
    }

    #[test]
    fn includes_add_document_version_migration_definition() {
        let migration = MIGRATIONS
            .iter()
            .find(|migration| migration.version == 6)
            .expect("version 6 migration definition should exist");

        assert_eq!(migration.description, "add_document_version");
        assert_eq!(
            migration.down_sql,
            "ALTER TABLE documents DROP COLUMN IF EXISTS version;"
        );

        let sql = fs::read_to_string("migrations/0006_add_document_version.sql")
            .expect("migration 0006 should exist");
        assert!(sql.contains("ADD COLUMN IF NOT EXISTS version bigint NOT NULL DEFAULT 1"));
    }

    #[test]
    fn migration_versions_are_contiguous_and_match_count() {
        for (index, migration) in MIGRATIONS.iter().enumerate() {
            assert_eq!(
                migration.version,
                (index as i64) + 1,
                "migration versions must be contiguous starting at 1"
            );
        }
    }

    #[test]
    fn migration_0018_add_note_chunk_embedding_provenance_exists_and_adds_columns() {
        let sql = fs::read_to_string("migrations/0018_add_note_chunk_embedding_provenance.sql")
            .expect("migration 0018 should exist");
        assert!(
            sql.contains("embedding_provider"),
            "migration 0018 must add embedding_provider column"
        );
        assert!(
            sql.contains("embedding_model"),
            "migration 0018 must add embedding_model column"
        );
        assert!(
            sql.contains("embedding_dimensions"),
            "migration 0018 must add embedding_dimensions column"
        );
        assert!(
            sql.contains("note_chunks_embedding_provenance_idx"),
            "migration 0018 must add provenance index"
        );
    }
}
