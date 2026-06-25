use crate::db::links::Visibility;
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
        INSERT INTO documents (slug, title, body_markdown, rendered_html, status, growth, tags, owner_id)
        VALUES (
            $1, $2, $3, $4, COALESCE($5, 'draft'), COALESCE($6, 'seedling'), $7,
            -- Stamp the creating author; fall back to the bootstrap admin when no
            -- principal id is supplied, matching the column default (ADR 0009).
            COALESCE($8, '00000000-0000-0000-0000-000000000001'::uuid)
        )
        RETURNING id, slug, title, body_markdown, rendered_html, status, growth, tags, version, created_at, updated_at
        "#,
    )
    .bind(&input.slug)
    .bind(&input.title)
    .bind(&input.body_markdown)
    .bind(&input.rendered_html)
    .bind(input.status.map(|status| status.as_str().to_string()))
    .bind(input.growth.map(|growth| growth.as_str().to_string()))
    .bind(&input.tags)
    .bind(input.owner_id)
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
                SELECT id, slug, title, body_markdown, rendered_html, status, growth, tags, version, created_at, updated_at
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
                SELECT id, slug, title, body_markdown, rendered_html, status, growth, tags, version, created_at, updated_at
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

pub async fn list_documents(
    pool: &PgPool,
    options: ListOptions,
) -> Result<Vec<Document>, sqlx::Error> {
    let mut builder = QueryBuilder::<Postgres>::new(
        "SELECT id, slug, title, body_markdown, rendered_html, status, growth, tags, version, created_at, updated_at FROM documents",
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

/// Update a document by slug. `owner` enforces ownership ATOMICALLY (ADR 0009
/// slice 3): `None` = admin (no constraint), `Some(id)` restricts the write to a
/// row owned by `id`. A non-owner (or missing slug) matches no row → `None` →
/// the handler's 404, with no separate check-then-write TOCTOU window.
pub async fn update_document_by_slug(
    pool: &PgPool,
    slug: &str,
    patch: DocumentPatch,
    owner: Option<Uuid>,
) -> Result<Option<Document>, DbError> {
    let result = sqlx::query_as::<Postgres, Document>(
        r#"
        UPDATE documents
        SET title = COALESCE($2, title),
            body_markdown = COALESCE($3, body_markdown),
            rendered_html = COALESCE($4, rendered_html),
            growth = COALESCE($5, growth),
            tags = COALESCE($6, tags),
            version = version + 1,
            updated_at = now()
        WHERE slug = $1 AND ($7::uuid IS NULL OR owner_id = $7)
        RETURNING id, slug, title, body_markdown, rendered_html, status, growth, tags, version, created_at, updated_at
        "#,
    )
    .bind(slug)
    .bind(&patch.title)
    .bind(&patch.body_markdown)
    .bind(&patch.rendered_html)
    .bind(patch.growth.map(|growth| growth.as_str().to_string()))
    .bind(&patch.tags)
    .bind(owner)
    .fetch_optional(pool)
    .await;

    map_optional_duplicate_slug(result, slug)
}

/// Outcome of a version-checked (`If-Match`) update.
///
/// The handler maps these to HTTP status codes: [`Updated`](Self::Updated) is a
/// `200`, [`NotFound`](Self::NotFound) a `404`, and
/// [`VersionMismatch`](Self::VersionMismatch) a `409 Conflict` so a stale write
/// surfaces cleanly to MCP clients instead of silently clobbering newer content.
pub enum ConditionalUpdate {
    Updated(Box<Document>),
    /// No row with this slug exists at all.
    NotFound,
    /// The row exists but its current `version` differs from the expected one.
    VersionMismatch {
        current: i64,
    },
}

/// Conditionally update a document, applying the patch only when the stored
/// `version` equals `expected_version`. The bump and the guard happen in the
/// same `UPDATE`, so two concurrent writers can never both win.
///
/// On a non-match the row is re-read to tell "no such slug" (→ 404) apart from
/// "someone edited it first" (→ 409); the probe is a plain read, so a row that
/// vanished between the update and the probe also reports `NotFound`.
pub async fn update_document_by_slug_if_version(
    pool: &PgPool,
    slug: &str,
    expected_version: i64,
    patch: DocumentPatch,
    owner: Option<Uuid>,
) -> Result<ConditionalUpdate, DbError> {
    let result = sqlx::query_as::<Postgres, Document>(
        r#"
        UPDATE documents
        SET title = COALESCE($3, title),
            body_markdown = COALESCE($4, body_markdown),
            rendered_html = COALESCE($5, rendered_html),
            growth = COALESCE($6, growth),
            tags = COALESCE($7, tags),
            version = version + 1,
            updated_at = now()
        WHERE slug = $1 AND version = $2 AND ($8::uuid IS NULL OR owner_id = $8)
        RETURNING id, slug, title, body_markdown, rendered_html, status, growth, tags, version, created_at, updated_at
        "#,
    )
    .bind(slug)
    .bind(expected_version)
    .bind(&patch.title)
    .bind(&patch.body_markdown)
    .bind(&patch.rendered_html)
    .bind(patch.growth.map(|growth| growth.as_str().to_string()))
    .bind(&patch.tags)
    .bind(owner)
    .fetch_optional(pool)
    .await;

    match map_optional_duplicate_slug(result, slug)? {
        Some(document) => Ok(ConditionalUpdate::Updated(Box::new(document))),
        None => {
            // The guarded UPDATE matched no row: the slug is gone, the version
            // moved, OR the caller doesn't own it. Probe at the SAME ownership
            // scope so a non-owner gets NotFound (404) rather than a
            // VersionMismatch (409) that would leak the row's existence.
            match sqlx::query_scalar::<Postgres, i64>(
                "SELECT version FROM documents WHERE slug = $1 AND ($2::uuid IS NULL OR owner_id = $2)",
            )
            .bind(slug)
            .bind(owner)
            .fetch_optional(pool)
            .await?
            {
                Some(current) => Ok(ConditionalUpdate::VersionMismatch { current }),
                None => Ok(ConditionalUpdate::NotFound),
            }
        }
    }
}

/// Set a document's status by slug. `owner` enforces ownership atomically (see
/// [`update_document_by_slug`]): `None` = admin, `Some(id)` restricts to a row
/// owned by `id`; a non-owner matches no row → `None` → 404.
pub async fn set_document_status(
    pool: &PgPool,
    slug: &str,
    status: DocumentStatus,
    owner: Option<Uuid>,
) -> Result<Option<Document>, sqlx::Error> {
    sqlx::query_as::<Postgres, Document>(
        r#"
        UPDATE documents
        SET status = $2, version = version + 1, updated_at = now()
        WHERE slug = $1 AND ($3::uuid IS NULL OR owner_id = $3)
        RETURNING id, slug, title, body_markdown, rendered_html, status, growth, tags, version, created_at, updated_at
        "#,
    )
    .bind(slug)
    .bind(status.as_str())
    .bind(owner)
    .fetch_optional(pool)
    .await
}

/// Overwrite a document's stored `rendered_html` without touching `version` or
/// `updated_at`. Used by the link-graph re-render fan-out: re-rendering a note
/// because a *linked* note changed is not a content edit, so it must not bump
/// the version or disturb cache/feed timestamps.
pub async fn set_rendered_html(
    pool: &PgPool,
    id: uuid::Uuid,
    rendered_html: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE documents SET rendered_html = $2 WHERE id = $1")
        .bind(id)
        .bind(rendered_html)
        .execute(pool)
        .await?;
    Ok(())
}

/// Delete a document by slug. `owner` enforces ownership atomically (see
/// [`update_document_by_slug`]): `None` = admin, `Some(id)` restricts to a row
/// owned by `id`; a non-owner deletes nothing → `false` → 404.
pub async fn delete_document_by_slug(
    pool: &PgPool,
    slug: &str,
    owner: Option<Uuid>,
) -> Result<bool, sqlx::Error> {
    let result = sqlx::query(
        "DELETE FROM documents WHERE slug = $1 AND ($2::uuid IS NULL OR owner_id = $2)",
    )
    .bind(slug)
    .bind(owner)
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
        "SELECT id, slug, title, body_markdown, rendered_html, status, growth, tags, version, created_at, updated_at FROM documents WHERE ",
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

pub async fn list_published_tags_page(
    pool: &PgPool,
    limit: u32,
    offset: u32,
) -> Result<Vec<TagCount>, sqlx::Error> {
    sqlx::query_as::<Postgres, TagCount>(
        r#"
        SELECT tag, count(*)::bigint AS count
        FROM documents
        CROSS JOIN LATERAL unnest(tags) AS tag
        WHERE status = 'published'
        GROUP BY tag
        ORDER BY count DESC, tag ASC
        LIMIT $1 OFFSET $2
        "#,
    )
    .bind(limit as i64)
    .bind(offset as i64)
    .fetch_all(pool)
    .await
}

pub async fn count_published_tags(pool: &PgPool) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar::<Postgres, i64>(
        r#"
        SELECT count(*)::bigint
        FROM (
            SELECT tag
            FROM documents
            CROSS JOIN LATERAL unnest(tags) AS tag
            WHERE status = 'published'
            GROUP BY tag
        ) AS published_tags
        "#,
    )
    .fetch_one(pool)
    .await
}

/// Full-text search over published documents (card T10, P3).
///
/// Uses Postgres FTS against the generated `search_vector` column (migration
/// 0008, title weighted 'A' above body 'B'), ranked by `ts_rank`. `websearch_to_tsquery`
/// parses the user's query the way a search box expects (bare words, quoted
/// phrases, `-exclusion`) and never errors on punctuation, so no manual escaping
/// is needed. The JSON + HTML surfaces and pagination are unchanged — only the
/// matching/ranking moved from ILIKE to FTS. Ties break on recency then id, so
/// ordering is deterministic.
pub async fn search_published_documents(
    pool: &PgPool,
    query: &str,
    options: SearchOptions,
) -> Result<Vec<Document>, sqlx::Error> {
    search_documents(pool, query, Visibility::Public, options).await
}

/// Visibility-aware full-text search: identical to
/// [`search_published_documents`] but applies the caller's
/// [`Visibility`] predicate (`Owner` sees own drafts + published; the public sees
/// only published; admin sees everything). Keeps the ask-your-garden FTS fallback
/// consistent with the vector path's visibility contract.
pub async fn search_documents(
    pool: &PgPool,
    query: &str,
    visibility: Visibility,
    options: SearchOptions,
) -> Result<Vec<Document>, sqlx::Error> {
    let mut builder = QueryBuilder::<Postgres>::new(
        "SELECT id, slug, title, body_markdown, rendered_html, status, growth, tags, version, created_at, updated_at
         FROM documents
         WHERE search_vector @@ websearch_to_tsquery('english', ",
    );
    builder.push_bind(query).push(")");
    match visibility {
        Visibility::Public => {
            builder.push(" AND status = 'published'");
        }
        Visibility::Owner(owner_id) => {
            builder
                .push(" AND (status = 'published' OR owner_id = ")
                .push_bind(owner_id)
                .push(")");
        }
        Visibility::All => {}
    }
    builder
        .push(" ORDER BY ts_rank(search_vector, websearch_to_tsquery('english', ")
        .push_bind(query)
        .push(")) DESC, created_at DESC, id DESC");
    if let Some(limit) = options.limit {
        builder.push(" LIMIT ").push_bind(limit as i64);
    }
    if let Some(offset) = options.offset {
        builder.push(" OFFSET ").push_bind(offset as i64);
    }
    builder.build_query_as().fetch_all(pool).await
}

/// Count FTS matches under the given [`Visibility`] predicate. Mirrors
/// [`search_documents`] so pagination totals stay consistent with the result set.
pub async fn count_search_documents(
    pool: &PgPool,
    query: &str,
    visibility: Visibility,
) -> Result<i64, sqlx::Error> {
    let mut builder = QueryBuilder::<Postgres>::new(
        "SELECT count(*)::bigint FROM documents \
         WHERE search_vector @@ websearch_to_tsquery('english', ",
    );
    builder.push_bind(query).push(")");
    match visibility {
        Visibility::Public => {
            builder.push(" AND status = 'published'");
        }
        Visibility::Owner(owner_id) => {
            builder
                .push(" AND (status = 'published' OR owner_id = ")
                .push_bind(owner_id)
                .push(")");
        }
        Visibility::All => {}
    }
    builder.build_query_scalar().fetch_one(pool).await
}

/// Visibility-aware single-document lookup by slug. The SQL predicate depends on
/// the caller's [`Visibility`]:
///   - [`Public`](Visibility::Public): `status = 'published'`
///   - [`Owner(id)`](Visibility::Owner): `status = 'published' OR owner_id = id`
///   - [`All`](Visibility::All): no restriction
///
/// Use this for every READ path that must enforce the no-draft-leak invariant.
/// Write paths (update, delete) that always need to see the row regardless of
/// status should continue to use [`get_document_by_slug`] with
/// [`StatusFilter::default()`].
pub async fn get_document_by_slug_vis(
    pool: &PgPool,
    slug: &str,
    visibility: Visibility,
) -> Result<Option<Document>, sqlx::Error> {
    match visibility {
        Visibility::Public => {
            sqlx::query_as::<Postgres, Document>(
                r#"SELECT id, slug, title, body_markdown, rendered_html, status, growth, tags,
                          version, created_at, updated_at
                   FROM documents WHERE slug = $1 AND status = 'published'"#,
            )
            .bind(slug)
            .fetch_optional(pool)
            .await
        }
        Visibility::Owner(owner_id) => {
            sqlx::query_as::<Postgres, Document>(
                r#"SELECT id, slug, title, body_markdown, rendered_html, status, growth, tags,
                          version, created_at, updated_at
                   FROM documents
                   WHERE slug = $1 AND (status = 'published' OR owner_id = $2)"#,
            )
            .bind(slug)
            .bind(owner_id)
            .fetch_optional(pool)
            .await
        }
        Visibility::All => {
            sqlx::query_as::<Postgres, Document>(
                r#"SELECT id, slug, title, body_markdown, rendered_html, status, growth, tags,
                          version, created_at, updated_at
                   FROM documents WHERE slug = $1"#,
            )
            .bind(slug)
            .fetch_optional(pool)
            .await
        }
    }
}

/// Visibility-aware document list. For Owner callers the base filter is
/// `(status='published' OR owner_id=$id)`; `extra_status` (from the `?status`
/// query param) further narrows the result set on top of that base:
///   - Admin+draft → own all drafts; Owner+draft → only own drafts.
///   - Admin+published / Owner+published → published only.
///   - `None` → use the visibility base without additional restriction.
pub async fn list_documents_vis(
    pool: &PgPool,
    visibility: Visibility,
    extra_status: Option<DocumentStatus>,
    limit: u32,
    offset: u32,
) -> Result<Vec<Document>, sqlx::Error> {
    let mut builder = QueryBuilder::<Postgres>::new(
        "SELECT id, slug, title, body_markdown, rendered_html, status, growth, tags, \
         version, created_at, updated_at FROM documents",
    );
    // Apply visibility base predicate.
    match visibility {
        Visibility::Public => {
            builder.push(" WHERE status = 'published'");
        }
        Visibility::Owner(owner_id) => {
            builder
                .push(" WHERE (status = 'published' OR owner_id = ")
                .push_bind(owner_id)
                .push(")");
        }
        Visibility::All => {
            if let Some(status) = extra_status {
                builder.push(" WHERE status = ").push_bind(status.as_str());
            }
            builder.push(" ORDER BY created_at DESC, id DESC");
            builder.push(" LIMIT ").push_bind(limit as i64);
            builder.push(" OFFSET ").push_bind(offset as i64);
            return builder.build_query_as().fetch_all(pool).await;
        }
    }
    // For Public and Owner: optionally AND a user-supplied status filter.
    if let Some(status) = extra_status {
        builder.push(" AND status = ").push_bind(status.as_str());
    }
    builder.push(" ORDER BY created_at DESC, id DESC");
    builder.push(" LIMIT ").push_bind(limit as i64);
    builder.push(" OFFSET ").push_bind(offset as i64);
    builder.build_query_as().fetch_all(pool).await
}

/// Visibility-aware document count. Mirrors [`list_documents_vis`] so
/// pagination totals are consistent.
pub async fn count_documents_vis(
    pool: &PgPool,
    visibility: Visibility,
    extra_status: Option<DocumentStatus>,
) -> Result<i64, sqlx::Error> {
    let mut builder = QueryBuilder::<Postgres>::new("SELECT count(*)::bigint FROM documents");
    match visibility {
        Visibility::Public => {
            builder.push(" WHERE status = 'published'");
        }
        Visibility::Owner(owner_id) => {
            builder
                .push(" WHERE (status = 'published' OR owner_id = ")
                .push_bind(owner_id)
                .push(")");
        }
        Visibility::All => {
            if let Some(status) = extra_status {
                builder.push(" WHERE status = ").push_bind(status.as_str());
            }
            return builder.build_query_scalar().fetch_one(pool).await;
        }
    }
    if let Some(status) = extra_status {
        builder.push(" AND status = ").push_bind(status.as_str());
    }
    builder.build_query_scalar().fetch_one(pool).await
}

pub async fn count_search_published_documents(
    pool: &PgPool,
    query: &str,
) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar::<Postgres, i64>(
        "SELECT count(*)::bigint FROM documents \
         WHERE status = 'published' \
         AND search_vector @@ websearch_to_tsquery('english', $1)",
    )
    .bind(query)
    .fetch_one(pool)
    .await
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
