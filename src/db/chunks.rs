//! Embedding data access for the semantic layer (card T10, P3).
//!
//! Chunks live in `note_chunks` (migration 0009), one row per embedded slice of
//! a note's body. Embeddings are bound as the pgvector text literal and cast to
//! `::vector` in SQL (see [`crate::ai::vector_to_pg_text`]), so this layer needs
//! no version-coupled sqlx encoder for the `vector` type.
//!
//! Retrieval is visibility-filtered through the SAME [`Visibility`] predicate
//! every content-exposing surface uses, so a public caller never sees a draft
//! note in related-notes or ask-your-garden citations (the no-draft-leak
//! invariant). Distance is cosine (`<=>`).

use crate::ai::vector_to_pg_text;
use crate::db::links::Visibility;
use sqlx::{PgPool, Postgres};
use uuid::Uuid;

/// One chunk to persist for a note: its position and content alongside the
/// embedding vector produced for it.
#[derive(Clone, Debug)]
pub struct NewChunk {
    pub chunk_index: i32,
    pub content: String,
    pub embedding: Vec<f32>,
}

/// A related note surfaced by vector search: the published (or, for owners,
/// draft) note nearest the query note plus its best chunk distance.
#[derive(Clone, Debug, PartialEq)]
pub struct RelatedNote {
    pub slug: String,
    pub title: String,
    /// Cosine distance of the closest chunk (lower = more similar).
    pub distance: f64,
}

/// A retrieved chunk for ask-your-garden: the source note's slug/title and the
/// chunk text that matched, with its cosine distance to the query.
#[derive(Clone, Debug, PartialEq)]
pub struct RetrievedChunk {
    pub slug: String,
    pub title: String,
    pub content: String,
    pub distance: f64,
}

/// Replace every chunk of `note_id` with `chunks`, atomically. Mirrors
/// [`replace_source_edges`](crate::db::links::replace_source_edges): the old set
/// is deleted and the new set inserted in one transaction, so a re-embed never
/// leaves a partial mix of stale and fresh chunks. An empty `chunks` clears the
/// note's embeddings (e.g. a body that chunks to nothing).
///
/// `expected_version` guards against stale overwrites under concurrent updates:
/// indexing runs after the document write, so a slower OLDER update could
/// otherwise clobber embeddings produced by a newer one. The document row is
/// locked (`FOR UPDATE`) and its current `version` re-read inside the same
/// transaction; if it no longer matches `expected_version`, a newer write has
/// landed (and will run — or has run — its own indexing), so this replace is
/// skipped. Returns `true` if the chunks were written, `false` if skipped as
/// stale. A vanished note (deleted concurrently) is treated as stale.
pub async fn replace_note_chunks(
    pool: &PgPool,
    note_id: Uuid,
    expected_version: i64,
    chunks: &[NewChunk],
) -> Result<bool, sqlx::Error> {
    let mut tx = pool.begin().await?;
    let current_version: Option<i64> = sqlx::query_scalar::<Postgres, i64>(
        "SELECT version FROM documents WHERE id = $1 FOR UPDATE",
    )
    .bind(note_id)
    .fetch_optional(&mut *tx)
    .await?;
    if current_version != Some(expected_version) {
        // A newer revision (or a delete) won the race; leave its embeddings be.
        tx.rollback().await?;
        return Ok(false);
    }
    sqlx::query("DELETE FROM note_chunks WHERE note_id = $1")
        .bind(note_id)
        .execute(&mut *tx)
        .await?;
    for chunk in chunks {
        let embedding = vector_to_pg_text(&chunk.embedding);
        sqlx::query(
            "INSERT INTO note_chunks (note_id, chunk_index, content, embedding) \
             VALUES ($1, $2, $3, $4::vector)",
        )
        .bind(note_id)
        .bind(chunk.chunk_index)
        .bind(&chunk.content)
        .bind(&embedding)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(true)
}

/// Number of chunk rows stored for a note (used by tests and re-embed checks).
pub async fn count_note_chunks(pool: &PgPool, note_id: Uuid) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar::<Postgres, i64>(
        "SELECT count(*)::bigint FROM note_chunks WHERE note_id = $1",
    )
    .bind(note_id)
    .fetch_one(pool)
    .await
}

/// Notes nearest `query_embedding` by cosine distance over their chunk
/// embeddings, EXCLUDING the origin note and visibility-filtered so a public
/// caller never sees a draft (the no-draft-leak invariant). One row per note
/// (its closest chunk), ordered by ascending distance, capped at `limit`.
pub async fn related_notes(
    pool: &PgPool,
    exclude_note_id: Uuid,
    query_embedding: &[f32],
    visibility: Visibility,
    limit: i64,
) -> Result<Vec<RelatedNote>, sqlx::Error> {
    let embedding = vector_to_pg_text(query_embedding);
    // DISTINCT ON (note) keeps each note's single nearest chunk; the outer order
    // then ranks notes by that best distance. Both the inner DISTINCT ON and the
    // outer query order by distance so the chosen chunk is genuinely the closest.
    let rows: Vec<(String, String, f64)> = match visibility.status_filter() {
        Some(status) => {
            sqlx::query_as::<Postgres, (String, String, f64)>(
                r#"
                SELECT slug, title, distance FROM (
                    SELECT DISTINCT ON (documents.id)
                           documents.slug AS slug,
                           documents.title AS title,
                           (note_chunks.embedding <=> $1::vector) AS distance
                    FROM note_chunks
                    JOIN documents ON documents.id = note_chunks.note_id
                    WHERE documents.id <> $2
                      AND documents.status = $3
                    ORDER BY documents.id, distance
                ) AS nearest
                ORDER BY distance ASC, slug ASC
                LIMIT $4
                "#,
            )
            .bind(&embedding)
            .bind(exclude_note_id)
            .bind(status.as_str())
            .bind(limit)
            .fetch_all(pool)
            .await?
        }
        None => {
            sqlx::query_as::<Postgres, (String, String, f64)>(
                r#"
                SELECT slug, title, distance FROM (
                    SELECT DISTINCT ON (documents.id)
                           documents.slug AS slug,
                           documents.title AS title,
                           (note_chunks.embedding <=> $1::vector) AS distance
                    FROM note_chunks
                    JOIN documents ON documents.id = note_chunks.note_id
                    WHERE documents.id <> $2
                    ORDER BY documents.id, distance
                ) AS nearest
                ORDER BY distance ASC, slug ASC
                LIMIT $3
                "#,
            )
            .bind(&embedding)
            .bind(exclude_note_id)
            .bind(limit)
            .fetch_all(pool)
            .await?
        }
    };
    Ok(rows
        .into_iter()
        .map(|(slug, title, distance)| RelatedNote {
            slug,
            title,
            distance,
        })
        .collect())
}

/// Top-`limit` chunks nearest `query_embedding` by cosine distance for
/// ask-your-garden retrieval, visibility-filtered so a public caller's answer is
/// never grounded in (nor cites) a draft note. Returns the chunk content plus its
/// source note, ordered by ascending distance.
pub async fn search_chunks(
    pool: &PgPool,
    query_embedding: &[f32],
    visibility: Visibility,
    limit: i64,
) -> Result<Vec<RetrievedChunk>, sqlx::Error> {
    let embedding = vector_to_pg_text(query_embedding);
    let rows: Vec<(String, String, String, f64)> = match visibility.status_filter() {
        Some(status) => {
            sqlx::query_as::<Postgres, (String, String, String, f64)>(
                r#"
                SELECT documents.slug, documents.title, note_chunks.content,
                       (note_chunks.embedding <=> $1::vector) AS distance
                FROM note_chunks
                JOIN documents ON documents.id = note_chunks.note_id
                WHERE documents.status = $2
                ORDER BY distance ASC, documents.slug ASC, note_chunks.chunk_index ASC
                LIMIT $3
                "#,
            )
            .bind(&embedding)
            .bind(status.as_str())
            .bind(limit)
            .fetch_all(pool)
            .await?
        }
        None => {
            sqlx::query_as::<Postgres, (String, String, String, f64)>(
                r#"
                SELECT documents.slug, documents.title, note_chunks.content,
                       (note_chunks.embedding <=> $1::vector) AS distance
                FROM note_chunks
                JOIN documents ON documents.id = note_chunks.note_id
                ORDER BY distance ASC, documents.slug ASC, note_chunks.chunk_index ASC
                LIMIT $2
                "#,
            )
            .bind(&embedding)
            .bind(limit)
            .fetch_all(pool)
            .await?
        }
    };
    Ok(rows
        .into_iter()
        .map(|(slug, title, content, distance)| RetrievedChunk {
            slug,
            title,
            content,
            distance,
        })
        .collect())
}
