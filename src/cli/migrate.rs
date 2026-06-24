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

/// Re-embed all notes using the active embedder and replace stored chunks.
///
/// Lists all documents in pages of 100, calls `index_note` for each, and
/// prints a final summary. Never logs document bodies or secrets.
pub async fn db_reindex_embeddings(
    pool: &PgPool,
    embedder: &dyn crate::ai::Embedder,
) -> Result<()> {
    use crate::ai::index_note;
    use crate::domain::document::ListOptions;

    let page_size: u32 = 100;
    let mut offset: u32 = 0;
    let mut indexed: u64 = 0;
    let skipped: u64 = 0; // reserved for future use (stale-version skips)
    let mut failed: u64 = 0;

    println!(
        "Reindexing embeddings using provider={} model={}",
        embedder.provider(),
        embedder.model()
    );

    loop {
        let docs = crate::db::documents::list_documents(
            pool,
            ListOptions {
                limit: Some(page_size),
                offset: Some(offset),
                status: None,
            },
        )
        .await?;

        if docs.is_empty() {
            break;
        }

        let page_count = docs.len() as u32;
        for doc in docs {
            match index_note(pool, embedder, doc.id, doc.version, &doc.body_markdown).await {
                Ok(()) => {
                    indexed += 1;
                }
                Err(err) => {
                    // Log slug + error without touching the body. A rate limit or
                    // transient provider error should not abort the whole run.
                    tracing::warn!(slug = %doc.slug, error = %err, "reindex: failed to index note");
                    failed += 1;
                }
            }
        }

        if page_count < page_size {
            break;
        }
        offset += page_size;
    }

    println!("Reindex complete: indexed={indexed} skipped={skipped} failed={failed}");
    Ok(())
}
