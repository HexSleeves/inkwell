//! Webmention data access (card T11, federation P3).
//!
//! Received mentions live in the `webmentions` table (migration 0010), one row
//! per `(source_url, target_note_id)` pair. The lifecycle is:
//!   1. receive  → [`upsert_pending`] records the claim as `pending`;
//!   2. verify   → [`mark_verified`] flips it to `verified` once the async,
//!      SSRF-guarded fetch confirms the source really links to the target;
//!   3. drop     → [`delete_mention`] removes an unverifiable claim entirely, so
//!      an unverified mention is never surfaced.
//!
//! The read path ([`verified_mentions`]) is visibility-filtered exactly like
//! backlinks: a mention targeting a draft is invisible to the public, reusing the
//! centralized [`Visibility`] predicate rather than re-deriving the rule.

use crate::db::links::Visibility;
use sqlx::{PgPool, Postgres};
use uuid::Uuid;

/// A verified inbound mention of a note: the remote URL that links to it. Mirrors
/// the shape of [`Backlink`](crate::db::links::Backlink) for the read surfaces.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Mention {
    pub source_url: String,
}

/// Record a pending mention of `target_note_id` from `source_url`, or leave an
/// existing row's status untouched if the pair was already recorded. Returns the
/// row id so the verifier can target it precisely.
///
/// Re-submitting an already-`verified` mention must not silently downgrade it to
/// `pending` (that would briefly hide a real mention), so the upsert keeps the
/// existing status on conflict rather than resetting it.
pub async fn upsert_pending(
    pool: &PgPool,
    source_url: &str,
    target_note_id: Uuid,
) -> Result<Uuid, sqlx::Error> {
    sqlx::query_scalar::<Postgres, Uuid>(
        r#"
        INSERT INTO webmentions (source_url, target_note_id, status)
        VALUES ($1, $2, 'pending')
        ON CONFLICT (source_url, target_note_id)
        DO UPDATE SET source_url = EXCLUDED.source_url
        RETURNING id
        "#,
    )
    .bind(source_url)
    .bind(target_note_id)
    .fetch_one(pool)
    .await
}

/// Flip a recorded mention to `verified` after its source was confirmed to link
/// to the target. Idempotent: re-verifying an already-verified row is a no-op.
pub async fn mark_verified(pool: &PgPool, id: Uuid) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE webmentions SET status = 'verified' WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Delete a mention row — used when verification fails, so an unverifiable claim
/// is dropped entirely rather than lingering as `pending`.
pub async fn delete_mention(pool: &PgPool, id: Uuid) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM webmentions WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Verified mentions of `target_note_id`, visibility-filtered so a mention of a
/// note the caller cannot see is never exposed (the no-draft-leak invariant,
/// shared with [`backlinks`](crate::db::links::backlinks)). The target document's
/// status must satisfy the caller's [`Visibility`]: `Public` ⇒ the note must be
/// `published`; `All` ⇒ no restriction (owner scope). Ordered by recency then id
/// for deterministic output, and hard-capped so the payload stays bounded.
pub async fn verified_mentions(
    pool: &PgPool,
    target_note_id: Uuid,
    visibility: Visibility,
) -> Result<Vec<Mention>, sqlx::Error> {
    let rows: Vec<(String,)> = match visibility {
        Visibility::Public => {
            sqlx::query_as::<Postgres, (String,)>(
                r#"
                SELECT webmentions.source_url
                FROM webmentions
                JOIN documents ON documents.id = webmentions.target_note_id
                WHERE webmentions.target_note_id = $1
                  AND webmentions.status = 'verified'
                  AND documents.status = 'published'
                ORDER BY webmentions.created_at DESC, webmentions.id
                LIMIT $2
                "#,
            )
            .bind(target_note_id)
            .bind(MAX_MENTIONS)
            .fetch_all(pool)
            .await?
        }
        Visibility::Owner(owner_id) => {
            sqlx::query_as::<Postgres, (String,)>(
                r#"
                SELECT webmentions.source_url
                FROM webmentions
                JOIN documents ON documents.id = webmentions.target_note_id
                WHERE webmentions.target_note_id = $1
                  AND webmentions.status = 'verified'
                  AND (documents.status = 'published' OR documents.owner_id = $2)
                ORDER BY webmentions.created_at DESC, webmentions.id
                LIMIT $3
                "#,
            )
            .bind(target_note_id)
            .bind(owner_id)
            .bind(MAX_MENTIONS)
            .fetch_all(pool)
            .await?
        }
        Visibility::All => {
            sqlx::query_as::<Postgres, (String,)>(
                r#"
                SELECT webmentions.source_url
                FROM webmentions
                JOIN documents ON documents.id = webmentions.target_note_id
                WHERE webmentions.target_note_id = $1
                  AND webmentions.status = 'verified'
                ORDER BY webmentions.created_at DESC, webmentions.id
                LIMIT $2
                "#,
            )
            .bind(target_note_id)
            .bind(MAX_MENTIONS)
            .fetch_all(pool)
            .await?
        }
    };
    Ok(rows
        .into_iter()
        .map(|(source_url,)| Mention { source_url })
        .collect())
}

/// Hard cap on mentions returned for one note, so the surface stays bounded like
/// every other read path in the garden (graph nodes/edges, etc.).
const MAX_MENTIONS: i64 = 200;
