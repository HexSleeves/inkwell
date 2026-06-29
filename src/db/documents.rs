use crate::db::links::Visibility;
use crate::domain::document::{
    AdjacentDoc, ArchiveMonth, Document, DocumentPatch, DocumentStatus, DocumentSummary,
    ListByTagOptions, ListOptions, NewDocument, SearchOptions, StatusFilter, TagCount,
};
use sqlx::{AssertSqlSafe, PgPool, Postgres, QueryBuilder};
use uuid::Uuid;

const DOCUMENT_COLUMNS: &str = "id, slug, title, body_markdown, rendered_html, status, growth, tags, version, created_at, updated_at";
const DOCUMENT_SUMMARY_COLUMNS: &str = "id, slug, title, LEFT(body_markdown, 320) AS body_excerpt_source, tags, growth, status, created_at, updated_at";
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
        AssertSqlSafe(format!(
            r#"
        INSERT INTO documents (slug, title, body_markdown, rendered_html, status, growth, tags, owner_id)
        VALUES (
            $1, $2, $3, $4, COALESCE($5, 'draft'), COALESCE($6, 'seedling'), $7,
            -- Stamp the creating author; fall back to the bootstrap admin when no
            -- principal id is supplied, matching the column default (ADR 0009).
            COALESCE($8, '00000000-0000-0000-0000-000000000001'::uuid)
        )
        RETURNING {DOCUMENT_COLUMNS}
        "#
        )),
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
            sqlx::query_as::<Postgres, Document>(AssertSqlSafe(format!(
                r#"
                SELECT {DOCUMENT_COLUMNS}
                FROM documents
                WHERE slug = $1 AND status = $2
                "#
            )))
            .bind(slug)
            .bind(status.as_str())
            .fetch_optional(pool)
            .await
        }
        None => {
            sqlx::query_as::<Postgres, Document>(AssertSqlSafe(format!(
                r#"
                SELECT {DOCUMENT_COLUMNS}
                FROM documents
                WHERE slug = $1
                "#
            )))
            .bind(slug)
            .fetch_optional(pool)
            .await
        }
    }
}

pub async fn get_document_body_by_id(
    pool: &PgPool,
    id: Uuid,
) -> Result<Option<(String, String)>, sqlx::Error> {
    sqlx::query_as::<Postgres, (String, String)>(
        "SELECT slug, body_markdown FROM documents WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
}

pub async fn get_embed_target_by_slug(
    pool: &PgPool,
    slug: &str,
    visibility: Visibility,
) -> Result<Option<(DocumentStatus, String)>, sqlx::Error> {
    match visibility {
        Visibility::Public => {
            sqlx::query_as::<Postgres, (DocumentStatus, String)>(
                "SELECT status, body_markdown FROM documents \
                 WHERE slug = $1 AND status = 'published'",
            )
            .bind(slug)
            .fetch_optional(pool)
            .await
        }
        Visibility::Owner(owner_id) => {
            sqlx::query_as::<Postgres, (DocumentStatus, String)>(
                "SELECT status, body_markdown FROM documents \
                 WHERE slug = $1 AND (status = 'published' OR owner_id = $2)",
            )
            .bind(slug)
            .bind(owner_id)
            .fetch_optional(pool)
            .await
        }
        Visibility::All => {
            sqlx::query_as::<Postgres, (DocumentStatus, String)>(
                "SELECT status, body_markdown FROM documents WHERE slug = $1",
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
    let mut builder =
        QueryBuilder::<Postgres>::new(format!("SELECT {DOCUMENT_COLUMNS} FROM documents"));
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

pub async fn list_document_summaries(
    pool: &PgPool,
    options: ListOptions,
) -> Result<Vec<DocumentSummary>, sqlx::Error> {
    let mut builder =
        QueryBuilder::<Postgres>::new(format!("SELECT {DOCUMENT_SUMMARY_COLUMNS} FROM documents"));
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
    // Rename path (ADR 0011): a patch that carries a *different* slug records the
    // old slug as a 301 alias and changes `documents.slug` atomically (one
    // version bump), alongside any field updates. A `new_slug` equal to the
    // current slug is a no-op and falls through to the plain update below.
    if let Some(new_slug) = patch.new_slug.as_deref().filter(|s| *s != slug) {
        return match rename_and_update(pool, slug, new_slug, &patch, owner, None).await? {
            ConditionalUpdate::Updated(document) => Ok(Some(*document)),
            ConditionalUpdate::NotFound => Ok(None),
            // No expected version is supplied here, so a mismatch never arises.
            ConditionalUpdate::VersionMismatch { .. } => Ok(None),
        };
    }

    let result = sqlx::query_as::<Postgres, Document>(AssertSqlSafe(format!(
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
        RETURNING {DOCUMENT_COLUMNS}
        "#
    )))
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
    // Rename path (ADR 0011) under optimistic concurrency: the version is checked
    // against the FOR UPDATE-locked row inside the same transaction as the alias
    // bookkeeping and slug change. See `rename_and_update`.
    if let Some(new_slug) = patch.new_slug.as_deref().filter(|s| *s != slug) {
        return rename_and_update(pool, slug, new_slug, &patch, owner, Some(expected_version))
            .await;
    }

    let result = sqlx::query_as::<Postgres, Document>(AssertSqlSafe(format!(
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
        RETURNING {DOCUMENT_COLUMNS}
        "#
    )))
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

/// Rename a document's slug (ADR 0011), recording the old slug as a 301 alias,
/// together with any field patch — all in ONE transaction so the change is
/// atomic and owner-enforced.
///
/// Steps, under a `FOR UPDATE` lock on the target row:
///  1. Lock by `current_slug` + ownership; a non-owner or missing slug → `NotFound`.
///  2. If `expected_version` is given and differs from the locked version →
///     `VersionMismatch` (the locked read makes this race-free).
///  3. If `new_slug` is already a live document's slug → `DuplicateSlug` (409).
///  4. Upsert `current_slug -> id` into `slug_aliases`, and delete any alias
///     equal to `new_slug` (so renaming back to a retired slug can't loop).
///  5. Apply the slug change + COALESCE field patch with a single version bump.
async fn rename_and_update(
    pool: &PgPool,
    current_slug: &str,
    new_slug: &str,
    patch: &DocumentPatch,
    owner: Option<Uuid>,
    expected_version: Option<i64>,
) -> Result<ConditionalUpdate, DbError> {
    let mut tx = pool.begin().await?;

    let locked = sqlx::query_as::<Postgres, (Uuid, i64)>(
        "SELECT id, version FROM documents \
         WHERE slug = $1 AND ($2::uuid IS NULL OR owner_id = $2) FOR UPDATE",
    )
    .bind(current_slug)
    .bind(owner)
    .fetch_optional(&mut *tx)
    .await?;
    let Some((id, version)) = locked else {
        tx.rollback().await?;
        return Ok(ConditionalUpdate::NotFound);
    };

    if let Some(expected) = expected_version
        && version != expected
    {
        tx.rollback().await?;
        return Ok(ConditionalUpdate::VersionMismatch { current: version });
    }

    // Destination slug must be free among live documents.
    let taken = sqlx::query_scalar::<Postgres, i32>("SELECT 1 FROM documents WHERE slug = $1")
        .bind(new_slug)
        .fetch_optional(&mut *tx)
        .await?;
    if taken.is_some() {
        tx.rollback().await?;
        return Err(DbError::DuplicateSlug {
            slug: new_slug.to_string(),
        });
    }

    sqlx::query(
        "INSERT INTO slug_aliases (old_slug, document_id) VALUES ($1, $2) \
         ON CONFLICT (old_slug) DO UPDATE SET document_id = EXCLUDED.document_id, created_at = now()",
    )
    .bind(current_slug)
    .bind(id)
    .execute(&mut *tx)
    .await?;
    sqlx::query("DELETE FROM slug_aliases WHERE old_slug = $1")
        .bind(new_slug)
        .execute(&mut *tx)
        .await?;

    let updated = sqlx::query_as::<Postgres, Document>(AssertSqlSafe(format!(
        r#"
        UPDATE documents
        SET slug = $2,
            title = COALESCE($3, title),
            body_markdown = COALESCE($4, body_markdown),
            rendered_html = COALESCE($5, rendered_html),
            growth = COALESCE($6, growth),
            tags = COALESCE($7, tags),
            version = version + 1,
            updated_at = now()
        WHERE id = $1
        RETURNING {DOCUMENT_COLUMNS}
        "#
    )))
    .bind(id)
    .bind(new_slug)
    .bind(&patch.title)
    .bind(&patch.body_markdown)
    .bind(&patch.rendered_html)
    .bind(patch.growth.map(|growth| growth.as_str().to_string()))
    .bind(&patch.tags)
    .fetch_one(&mut *tx)
    .await;

    let document = match updated {
        Ok(document) => document,
        // A concurrent insert could still take `new_slug` between the check and
        // this write; the unique index surfaces it as a 409 rather than a 500.
        Err(error) if is_unique_violation(&error) => {
            tx.rollback().await?;
            return Err(DbError::DuplicateSlug {
                slug: new_slug.to_string(),
            });
        }
        Err(error) => {
            tx.rollback().await?;
            return Err(DbError::Sqlx(error));
        }
    };

    tx.commit().await?;
    Ok(ConditionalUpdate::Updated(Box::new(document)))
}

/// Resolve a retired slug to its document's CURRENT slug for a 301 redirect, but
/// only when that document is visible under `visibility`. An alias whose target
/// is a draft the caller cannot see resolves to `None` (no existence leak),
/// mirroring the document read predicate (ADR 0009 slice 3b).
pub async fn resolve_alias_target(
    pool: &PgPool,
    old_slug: &str,
    visibility: Visibility,
) -> Result<Option<String>, sqlx::Error> {
    let base = "SELECT d.slug FROM slug_aliases a JOIN documents d ON d.id = a.document_id \
                WHERE a.old_slug = $1";
    match visibility {
        Visibility::Public => {
            sqlx::query_scalar::<Postgres, String>(AssertSqlSafe(format!(
                "{base} AND d.status = 'published'"
            )))
            .bind(old_slug)
            .fetch_optional(pool)
            .await
        }
        Visibility::Owner(owner_id) => {
            sqlx::query_scalar::<Postgres, String>(AssertSqlSafe(format!(
                "{base} AND (d.status = 'published' OR d.owner_id = $2)"
            )))
            .bind(old_slug)
            .bind(owner_id)
            .fetch_optional(pool)
            .await
        }
        Visibility::All => {
            sqlx::query_scalar::<Postgres, String>(base)
                .bind(old_slug)
                .fetch_optional(pool)
                .await
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
    sqlx::query_as::<Postgres, Document>(AssertSqlSafe(format!(
        r#"
        UPDATE documents
        SET status = $2, version = version + 1, updated_at = now()
        WHERE slug = $1 AND ($3::uuid IS NULL OR owner_id = $3)
        RETURNING {DOCUMENT_COLUMNS}
        "#
    )))
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
    let mut builder =
        QueryBuilder::<Postgres>::new(format!("SELECT {DOCUMENT_COLUMNS} FROM documents WHERE "));
    builder
        .push("tags @> ARRAY[")
        .push_bind(tag)
        .push("]::text[]");
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

pub async fn list_documents_by_tag_summary(
    pool: &PgPool,
    tag: &str,
    options: ListByTagOptions,
) -> Result<Vec<DocumentSummary>, sqlx::Error> {
    let mut builder = QueryBuilder::<Postgres>::new(format!(
        "SELECT {DOCUMENT_SUMMARY_COLUMNS} FROM documents WHERE "
    ));
    builder
        .push("tags @> ARRAY[")
        .push_bind(tag)
        .push("]::text[]");
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
    builder
        .push("tags @> ARRAY[")
        .push_bind(tag)
        .push("]::text[]");
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
    let mut builder = QueryBuilder::<Postgres>::new(format!(
        "SELECT {DOCUMENT_COLUMNS}
         FROM documents
         WHERE search_vector @@ websearch_to_tsquery('english', ",
    ));
    builder.push_bind(query).push(")");
    builder.push(" AND ");
    visibility.push_where(&mut builder);
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

pub async fn search_documents_summary(
    pool: &PgPool,
    query: &str,
    visibility: Visibility,
    options: SearchOptions,
) -> Result<Vec<DocumentSummary>, sqlx::Error> {
    let mut builder = QueryBuilder::<Postgres>::new(format!(
        "SELECT {DOCUMENT_SUMMARY_COLUMNS}
         FROM documents
         WHERE search_vector @@ websearch_to_tsquery('english', ",
    ));
    builder.push_bind(query).push(")");
    builder.push(" AND ");
    visibility.push_where(&mut builder);
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
    builder.push(" AND ");
    visibility.push_where(&mut builder);
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
            sqlx::query_as::<Postgres, Document>(AssertSqlSafe(format!(
                r#"SELECT {DOCUMENT_COLUMNS}
                   FROM documents WHERE slug = $1 AND status = 'published'"#,
            )))
            .bind(slug)
            .fetch_optional(pool)
            .await
        }
        Visibility::Owner(owner_id) => {
            sqlx::query_as::<Postgres, Document>(AssertSqlSafe(format!(
                r#"SELECT {DOCUMENT_COLUMNS}
                   FROM documents
                   WHERE slug = $1 AND (status = 'published' OR owner_id = $2)"#,
            )))
            .bind(slug)
            .bind(owner_id)
            .fetch_optional(pool)
            .await
        }
        Visibility::All => {
            sqlx::query_as::<Postgres, Document>(AssertSqlSafe(format!(
                r#"SELECT {DOCUMENT_COLUMNS}
                   FROM documents WHERE slug = $1"#,
            )))
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
    let mut builder =
        QueryBuilder::<Postgres>::new(format!("SELECT {DOCUMENT_COLUMNS} FROM documents WHERE "));
    visibility.push_where(&mut builder);
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
    let mut builder =
        QueryBuilder::<Postgres>::new("SELECT count(*)::bigint FROM documents WHERE ");
    visibility.push_where(&mut builder);
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

/// Return year/month buckets of published documents ordered newest first.
/// Used by the archive index page to build the browsing hierarchy.
/// Timestamps are normalised to UTC before year/month extraction so the
/// buckets are stable regardless of the database session timezone.
pub async fn list_archive_months(pool: &PgPool) -> Result<Vec<ArchiveMonth>, sqlx::Error> {
    sqlx::query_as::<Postgres, ArchiveMonth>(
        r#"
        SELECT
            EXTRACT(YEAR  FROM created_at AT TIME ZONE 'UTC')::int AS year,
            EXTRACT(MONTH FROM created_at AT TIME ZONE 'UTC')::int AS month,
            count(*)::bigint                                        AS count
        FROM documents
        WHERE status = 'published'
        GROUP BY 1, 2
        ORDER BY 1 DESC, 2 DESC
        "#,
    )
    .fetch_all(pool)
    .await
}

/// Compute the half-open UTC range `[month_start, next_month_start)` for the
/// given calendar `year`/`month`.
///
/// Archive-by-month queries previously wrapped `created_at` in
/// `EXTRACT(... AT TIME ZONE 'UTC')`, which is not sargable and forced a
/// sequential scan. Comparing the raw `timestamptz` column against a precomputed
/// UTC range instead lets the planner range-scan
/// `documents_status_created_at_id_idx` (migrations/0004). Because both bounds
/// are absolute instants, the result set is identical to the old EXTRACT
/// predicate, including the December -> January year rollover.
///
/// Returns `None` when `month` is outside `1..=12` (or the date is otherwise
/// invalid), in which case callers treat the bucket as empty — matching the old
/// EXTRACT predicate, which simply matched zero rows for such inputs.
fn month_utc_range(year: i32, month: i32) -> Option<(time::OffsetDateTime, time::OffsetDateTime)> {
    let month_enum = u8::try_from(month)
        .ok()
        .and_then(|m| time::Month::try_from(m).ok())?;
    let start = time::Date::from_calendar_date(year, month_enum, 1)
        .ok()?
        .midnight()
        .assume_utc();
    // `Month::next` wraps December -> January; bump the year on that wrap.
    let next_year = if month_enum == time::Month::December {
        year + 1
    } else {
        year
    };
    let end = time::Date::from_calendar_date(next_year, month_enum.next(), 1)
        .ok()?
        .midnight()
        .assume_utc();
    Some((start, end))
}

pub async fn count_documents_by_month(
    pool: &PgPool,
    year: i32,
    month: i32,
) -> Result<i64, sqlx::Error> {
    let Some((start, end)) = month_utc_range(year, month) else {
        return Ok(0);
    };
    sqlx::query_scalar::<Postgres, i64>(
        r#"
        SELECT count(*)::bigint
        FROM documents
        WHERE status = 'published'
          AND created_at >= $1
          AND created_at < $2
        "#,
    )
    .bind(start)
    .bind(end)
    .fetch_one(pool)
    .await
}

pub async fn list_documents_by_month(
    pool: &PgPool,
    year: i32,
    month: i32,
    limit: u32,
    offset: u32,
) -> Result<Vec<Document>, sqlx::Error> {
    let Some((start, end)) = month_utc_range(year, month) else {
        return Ok(Vec::new());
    };
    sqlx::query_as::<Postgres, Document>(AssertSqlSafe(format!(
        r#"
        SELECT {DOCUMENT_COLUMNS}
        FROM documents
        WHERE status = 'published'
          AND created_at >= $1
          AND created_at < $2
        ORDER BY created_at DESC, id DESC
        LIMIT $3 OFFSET $4
        "#
    )))
    .bind(start)
    .bind(end)
    .bind(limit as i64)
    .bind(offset as i64)
    .fetch_all(pool)
    .await
}

pub async fn list_documents_by_month_summary(
    pool: &PgPool,
    year: i32,
    month: i32,
    limit: u32,
    offset: u32,
) -> Result<Vec<DocumentSummary>, sqlx::Error> {
    let Some((start, end)) = month_utc_range(year, month) else {
        return Ok(Vec::new());
    };
    sqlx::query_as::<Postgres, DocumentSummary>(AssertSqlSafe(format!(
        r#"
        SELECT {DOCUMENT_SUMMARY_COLUMNS}
        FROM documents
        WHERE status = 'published'
          AND created_at >= $1
          AND created_at < $2
        ORDER BY created_at DESC, id DESC
        LIMIT $3 OFFSET $4
        "#
    )))
    .bind(start)
    .bind(end)
    .bind(limit as i64)
    .bind(offset as i64)
    .fetch_all(pool)
    .await
}

/// Return the published document immediately before (older) and immediately
/// after (newer) the document identified by `id`/`created_at` in the default
/// listing order (`created_at DESC, id DESC`). Either neighbour may be `None`
/// when this is the oldest or newest published document.
///
/// Accepts the already-fetched `id` and `created_at` so the handler avoids a
/// redundant SELECT when the document row is already in memory.
pub async fn get_adjacent_documents(
    pool: &PgPool,
    id: uuid::Uuid,
    created_at: time::OffsetDateTime,
) -> Result<(Option<AdjacentDoc>, Option<AdjacentDoc>), sqlx::Error> {
    let prev = sqlx::query_as::<Postgres, AdjacentDoc>(
        r#"
        SELECT slug, title FROM documents
        WHERE status = 'published'
          AND (created_at < $1 OR (created_at = $1 AND id < $2))
        ORDER BY created_at DESC, id DESC
        LIMIT 1
        "#,
    )
    .bind(created_at)
    .bind(id)
    .fetch_optional(pool)
    .await?;

    let next = sqlx::query_as::<Postgres, AdjacentDoc>(
        r#"
        SELECT slug, title FROM documents
        WHERE status = 'published'
          AND (created_at > $1 OR (created_at = $1 AND id > $2))
        ORDER BY created_at ASC, id ASC
        LIMIT 1
        "#,
    )
    .bind(created_at)
    .bind(id)
    .fetch_optional(pool)
    .await?;

    Ok((prev, next))
}

fn is_unique_violation(error: &sqlx::Error) -> bool {
    matches!(
        error,
        sqlx::Error::Database(database_error) if database_error.code().as_deref() == Some(UNIQUE_VIOLATION)
    )
}
