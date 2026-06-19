use crate::domain::document::{
    Document, DocumentPatch, DocumentStatus, ListByTagOptions, ListOptions, NewDocument,
    SearchOptions, StatusFilter, TagCount,
};
use sqlx::{PgPool, Postgres, QueryBuilder};
use uuid::Uuid;

const UNIQUE_VIOLATION: &str = "23505";

#[derive(Debug, thiserror::Error)]
pub enum DbError {
    #[error("A document with slug \"{slug}\" already exists.")]
    DuplicateSlug { slug: String },
    #[error(transparent)]
    Sqlx(#[from] sqlx::Error),
}

pub async fn create_document(pool: &PgPool, input: NewDocument) -> Result<Document, DbError> {
    let result = sqlx::query_as::<Postgres, Document>(
        r#"
        INSERT INTO documents (slug, title, body_markdown, rendered_html, status, tags)
        VALUES ($1, $2, $3, $4, COALESCE($5, 'draft'), $6)
        RETURNING id, slug, title, body_markdown, rendered_html, status, tags, created_at, updated_at
        "#,
    )
    .bind(&input.slug)
    .bind(&input.title)
    .bind(&input.body_markdown)
    .bind(&input.rendered_html)
    .bind(input.status.map(|status| status.as_str().to_string()))
    .bind(&input.tags)
    .fetch_one(pool)
    .await;

    map_duplicate_slug(result, &input.slug)
}

pub async fn get_document_by_slug(
    pool: &PgPool,
    slug: &str,
    filter: StatusFilter,
) -> Result<Option<Document>, sqlx::Error> {
    match filter.status {
        Some(status) => {
            sqlx::query_as::<Postgres, Document>(
                r#"
                SELECT id, slug, title, body_markdown, rendered_html, status, tags, created_at, updated_at
                FROM documents
                WHERE slug = $1 AND status = $2
                "#,
            )
            .bind(slug)
            .bind(status.as_str())
            .fetch_optional(pool)
            .await
        }
        None => {
            sqlx::query_as::<Postgres, Document>(
                r#"
                SELECT id, slug, title, body_markdown, rendered_html, status, tags, created_at, updated_at
                FROM documents
                WHERE slug = $1
                "#,
            )
            .bind(slug)
            .fetch_optional(pool)
            .await
        }
    }
}

pub async fn get_document_by_id(pool: &PgPool, id: Uuid) -> Result<Option<Document>, sqlx::Error> {
    sqlx::query_as::<Postgres, Document>(
        r#"
        SELECT id, slug, title, body_markdown, rendered_html, status, tags, created_at, updated_at
        FROM documents
        WHERE id = $1
        "#,
    )
    .bind(id)
    .fetch_optional(pool)
    .await
}

pub async fn list_documents(
    pool: &PgPool,
    options: ListOptions,
) -> Result<Vec<Document>, sqlx::Error> {
    let mut builder = QueryBuilder::<Postgres>::new(
        "SELECT id, slug, title, body_markdown, rendered_html, status, tags, created_at, updated_at FROM documents",
    );
    if let Some(status) = options.status {
        builder.push(" WHERE status = ").push_bind(status.as_str());
    }
    builder.push(" ORDER BY created_at DESC, id DESC");
    if let Some(limit) = options.limit {
        builder.push(" LIMIT ").push_bind(limit as i64);
    }
    if let Some(offset) = options.offset {
        builder.push(" OFFSET ").push_bind(offset as i64);
    }
    builder.build_query_as().fetch_all(pool).await
}

pub async fn count_documents(pool: &PgPool, filter: StatusFilter) -> Result<i64, sqlx::Error> {
    match filter.status {
        Some(status) => {
            sqlx::query_scalar::<Postgres, i64>(
                "SELECT count(*)::bigint FROM documents WHERE status = $1",
            )
            .bind(status.as_str())
            .fetch_one(pool)
            .await
        }
        None => {
            sqlx::query_scalar::<Postgres, i64>("SELECT count(*)::bigint FROM documents")
                .fetch_one(pool)
                .await
        }
    }
}

pub async fn update_document_by_slug(
    pool: &PgPool,
    slug: &str,
    patch: DocumentPatch,
) -> Result<Option<Document>, DbError> {
    let result = sqlx::query_as::<Postgres, Document>(
        r#"
        UPDATE documents
        SET title = COALESCE($2, title),
            body_markdown = COALESCE($3, body_markdown),
            rendered_html = COALESCE($4, rendered_html),
            tags = COALESCE($5, tags),
            updated_at = now()
        WHERE slug = $1
        RETURNING id, slug, title, body_markdown, rendered_html, status, tags, created_at, updated_at
        "#,
    )
    .bind(slug)
    .bind(&patch.title)
    .bind(&patch.body_markdown)
    .bind(&patch.rendered_html)
    .bind(&patch.tags)
    .fetch_optional(pool)
    .await;

    map_optional_duplicate_slug(result, slug)
}

pub async fn set_document_status(
    pool: &PgPool,
    slug: &str,
    status: DocumentStatus,
) -> Result<Option<Document>, sqlx::Error> {
    sqlx::query_as::<Postgres, Document>(
        r#"
        UPDATE documents
        SET status = $2
        WHERE slug = $1
        RETURNING id, slug, title, body_markdown, rendered_html, status, tags, created_at, updated_at
        "#,
    )
    .bind(slug)
    .bind(status.as_str())
    .fetch_optional(pool)
    .await
}

pub async fn delete_document_by_slug(pool: &PgPool, slug: &str) -> Result<bool, sqlx::Error> {
    let result = sqlx::query("DELETE FROM documents WHERE slug = $1")
        .bind(slug)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
}

pub async fn list_documents_by_tag(
    pool: &PgPool,
    tag: &str,
    options: ListByTagOptions,
) -> Result<Vec<Document>, sqlx::Error> {
    let mut builder = QueryBuilder::<Postgres>::new(
        "SELECT id, slug, title, body_markdown, rendered_html, status, tags, created_at, updated_at FROM documents WHERE ",
    );
    builder.push_bind(tag).push(" = ANY(tags)");
    if let Some(status) = options.status {
        builder.push(" AND status = ").push_bind(status.as_str());
    }
    builder.push(" ORDER BY created_at DESC, id DESC");
    if let Some(limit) = options.limit {
        builder.push(" LIMIT ").push_bind(limit as i64);
    }
    if let Some(offset) = options.offset {
        builder.push(" OFFSET ").push_bind(offset as i64);
    }
    builder.build_query_as().fetch_all(pool).await
}

pub async fn count_documents_by_tag(
    pool: &PgPool,
    tag: &str,
    filter: StatusFilter,
) -> Result<i64, sqlx::Error> {
    let mut builder =
        QueryBuilder::<Postgres>::new("SELECT count(*)::bigint FROM documents WHERE ");
    builder.push_bind(tag).push(" = ANY(tags)");
    if let Some(status) = filter.status {
        builder.push(" AND status = ").push_bind(status.as_str());
    }
    builder.build_query_scalar().fetch_one(pool).await
}

pub async fn list_published_tags(pool: &PgPool) -> Result<Vec<TagCount>, sqlx::Error> {
    sqlx::query_as::<Postgres, TagCount>(
        r#"
        SELECT tag, count(*)::bigint AS count
        FROM documents
        CROSS JOIN LATERAL unnest(tags) AS tag
        WHERE status = 'published'
        GROUP BY tag
        ORDER BY count DESC, tag ASC
        "#,
    )
    .fetch_all(pool)
    .await
}

pub async fn search_published_documents(
    pool: &PgPool,
    query: &str,
    options: SearchOptions,
) -> Result<Vec<Document>, sqlx::Error> {
    let pattern = format!("%{}%", escape_like_pattern(query));
    let mut builder = QueryBuilder::<Postgres>::new(
        "SELECT id, slug, title, body_markdown, rendered_html, status, tags, created_at, updated_at
         FROM documents
         WHERE status = 'published'
         AND (title ILIKE ",
    );
    builder
        .push_bind(&pattern)
        .push(" ESCAPE '\\' OR body_markdown ILIKE ")
        .push_bind(&pattern)
        .push(" ESCAPE '\\')")
        .push(" ORDER BY (CASE WHEN title ILIKE ")
        .push_bind(&pattern)
        .push(" ESCAPE '\\' THEN 0 ELSE 1 END), created_at DESC, id DESC");
    if let Some(limit) = options.limit {
        builder.push(" LIMIT ").push_bind(limit as i64);
    }
    if let Some(offset) = options.offset {
        builder.push(" OFFSET ").push_bind(offset as i64);
    }
    builder.build_query_as().fetch_all(pool).await
}

pub async fn count_search_published_documents(
    pool: &PgPool,
    query: &str,
) -> Result<i64, sqlx::Error> {
    let pattern = format!("%{}%", escape_like_pattern(query));
    sqlx::query_scalar::<Postgres, i64>(
        "SELECT count(*)::bigint FROM documents WHERE status = 'published' AND (title ILIKE $1 ESCAPE '\\' OR body_markdown ILIKE $1 ESCAPE '\\')",
    )
    .bind(pattern)
    .fetch_one(pool)
    .await
}

fn escape_like_pattern(term: &str) -> String {
    term.chars()
        .flat_map(|ch| match ch {
            '\\' => vec!['\\', '\\'],
            '%' => vec!['\\', '%'],
            '_' => vec!['\\', '_'],
            other => vec![other],
        })
        .collect()
}

fn map_duplicate_slug(
    result: Result<Document, sqlx::Error>,
    slug: &str,
) -> Result<Document, DbError> {
    match result {
        Ok(document) => Ok(document),
        Err(error) if is_unique_violation(&error) => Err(DbError::DuplicateSlug {
            slug: slug.to_string(),
        }),
        Err(error) => Err(DbError::Sqlx(error)),
    }
}

fn map_optional_duplicate_slug(
    result: Result<Option<Document>, sqlx::Error>,
    slug: &str,
) -> Result<Option<Document>, DbError> {
    match result {
        Ok(document) => Ok(document),
        Err(error) if is_unique_violation(&error) => Err(DbError::DuplicateSlug {
            slug: slug.to_string(),
        }),
        Err(error) => Err(DbError::Sqlx(error)),
    }
}

fn is_unique_violation(error: &sqlx::Error) -> bool {
    matches!(
        error,
        sqlx::Error::Database(database_error) if database_error.code().as_deref() == Some(UNIQUE_VIOLATION)
    )
}
