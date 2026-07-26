//! Tested backup + restore for the whole Inkwell deployment (CYP-49).
//!
//! # Why a logical, pure-Rust bundle instead of `pg_dump`
//!
//! `pg_dump`/`pg_restore` are excellent but they are *external binaries* whose
//! major version must match (or exceed) the server's. That makes them awkward to
//! depend on from a single static `inkwell` binary, impossible to exercise from
//! `cargo test` on a machine without matching client tools, and a silent failure
//! mode on hosts that ship an older `postgresql-client`. So `inkwell backup`
//! writes its own logical bundle using nothing but the pool we already have.
//! The `pg_dump` route is still documented in `docs/BACKUP-RESTORE.md` as the
//! physical-fidelity alternative.
//!
//! # Bundle format (version 1)
//!
//! A gzip stream of JSON Lines. The first line is the [`Manifest`]; every
//! subsequent line is one table row:
//!
//! ```text
//! {"kind":"manifest","bundleFormat":1,"inkwellVersion":"0.1.0","schemaVersion":24,...}
//! {"kind":"row","table":"authors","data":{"id":"…","name":"admin",…}}
//! {"kind":"row","table":"documents","data":{…}}
//! ```
//!
//! Rows are emitted in [`TABLES`] order, which is a topological order over the
//! foreign keys, so a restore can stream straight through without deferring
//! constraints.
//!
//! Row payloads come from Postgres' own `to_jsonb`, and are fed back through
//! `jsonb_populate_recordset`, so we never hand-write a per-table struct. Adding
//! a column in a migration extends the bundle automatically. Two consequences
//! worth knowing:
//!
//! - `bytea` round-trips as its `\x…` hex text form, so legacy pre-0025 inline
//!   `media.data` blobs come along with the row for free.
//! - Generated columns (`documents.search_vector`) are excluded from both dump
//!   and restore, because Postgres recomputes them on insert.
//!
//! # Media (CYP-45 / ADR 0013)
//!
//! Since CYP-45 a `media` row points at a content-addressed *storage key* and
//! the bytes live behind [`crate::media::MediaStore`] — on the local filesystem
//! under `INKWELL_MEDIA_DIR` (the default) or in the `media_blobs` table.
//!
//! The bundle therefore carries blobs as their own record kind, read through the
//! store trait rather than copied out of whichever place the source deployment
//! happens to keep them:
//!
//! ```text
//! {"kind":"blob","key":"3f/a9/3fa9…c1.png","bytes":"<base64>"}
//! ```
//!
//! Two things follow. A bundle is **backend-portable** — back up a
//! filesystem-backed deployment and restore it onto a Postgres-backed one (or
//! vice versa) and every `/media/{id}` URL still resolves. And `media_blobs` is
//! deliberately *not* in [`TABLES`]: it is one backend's private storage, not
//! logical data, so dumping it as a table would double the bundle for
//! Postgres-backed deployments and produce nothing restorable for
//! filesystem-backed ones.

pub mod create;
pub mod restore;

use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};
use sqlx::PgExecutor;

/// Bundle format version. Bumped only for changes an older reader could not
/// safely interpret; a reader refuses anything greater than the version it
/// knows.
pub const BUNDLE_FORMAT: u32 = 1;

/// Every table whose contents belong in a backup, in foreign-key-safe insert
/// order (dependencies first).
///
/// Two deliberate absences:
/// - `_sqlx_migrations` — the target deployment owns its own migration state, and
///   the bundle records the source's in [`Manifest::schema_version`].
/// - `media_blobs` — one storage backend's private bytes. Blobs travel as
///   [`Record::Blob`] records read through [`crate::media::MediaStore`] so a
///   bundle is portable across backends.
pub const TABLES: [&str; 11] = [
    "authors",
    "documents",
    "author_tokens",
    "links",
    "slug_aliases",
    "note_chunks",
    "webmentions",
    "write_audit",
    "media",
    "sessions",
    "preview_tokens",
];

/// The bootstrap admin author seeded by migration 0015 at a fixed uuid.
///
/// A freshly migrated deployment is *not* literally row-empty — it always holds
/// this one author. The emptiness check that guards restore therefore ignores
/// it, so "restore into an empty deployment" means what an operator expects.
pub const BOOTSTRAP_ADMIN_ID: &str = "00000000-0000-0000-0000-000000000001";

/// First line of a bundle: everything needed to decide whether this bundle can
/// be restored by the binary reading it.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    pub bundle_format: u32,
    /// `CARGO_PKG_VERSION` of the binary that produced the bundle.
    pub inkwell_version: String,
    /// Highest applied migration version on the source database.
    pub schema_version: i64,
    /// Every applied migration version, so a mismatch can be described exactly
    /// rather than as a single number.
    pub applied_migrations: Vec<i64>,
    /// RFC 3339, UTC.
    pub created_at: String,
    /// [`crate::media::MediaStore::backend`] of the source deployment. Recorded
    /// for the operator's benefit only — the bundle restores onto any backend.
    pub media_backend: String,
    /// Distinct media blobs carried in the bundle.
    pub blobs: i64,
    pub tables: Vec<TableSummary>,
}

/// Per-table row count captured inside the same snapshot transaction as the
/// rows themselves, so counts and contents always agree.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TableSummary {
    pub name: String,
    pub rows: i64,
}

impl Manifest {
    /// Row count recorded for `table`, or 0 when the bundle predates it.
    pub fn rows_for(&self, table: &str) -> i64 {
        self.tables
            .iter()
            .find(|summary| summary.name == table)
            .map(|summary| summary.rows)
            .unwrap_or(0)
    }
}

/// One JSON Lines record. Internally tagged so the reader can dispatch on
/// `kind` without a second parse.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Record {
    Manifest(Manifest),
    Row {
        table: String,
        data: serde_json::Map<String, serde_json::Value>,
    },
    /// One media blob, keyed by its content address. `bytes` is standard base64
    /// — 33% overhead against the raw blob, versus 100% for the `\x…` hex form a
    /// `bytea` column would have produced.
    Blob {
        key: String,
        bytes: String,
    },
}

/// Encode blob bytes for a [`Record::Blob`].
pub fn encode_blob(bytes: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// Decode a [`Record::Blob`] payload and verify it against the content address
/// in `key`.
///
/// The key *is* the SHA-256 of the bytes, so this is a free integrity check on
/// every blob in the bundle: truncation, bit rot, and tampering all surface here
/// instead of installing wrong image data.
pub fn decode_blob(key: &str, encoded: &str) -> Result<Vec<u8>> {
    use base64::Engine;

    let expected = crate::media::digest_in_storage_key(key)
        .ok_or_else(|| anyhow!("bundle contains a malformed media storage key: {key:?}"))?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|error| anyhow!("media blob {key} is not valid base64: {error}"))?;
    let actual = crate::media::checksum_hex(&bytes);
    if actual != expected {
        return Err(anyhow!(
            "media blob {key} failed its checksum: bundle bytes hash to {actual}. \
             The bundle is corrupt; nothing was changed."
        ));
    }
    Ok(bytes)
}

/// Columns of `table` that a restore may write: real, non-generated columns.
///
/// Generated columns (`documents.search_vector`) are excluded because Postgres
/// rejects an explicit value for them; identity columns are excluded because
/// they would need `OVERRIDING SYSTEM VALUE` (the schema has none today, so this
/// is future-proofing rather than dead code).
pub async fn restorable_columns<'e, E>(executor: E, table: &str) -> Result<Vec<String>>
where
    E: PgExecutor<'e>,
{
    let columns: Vec<String> = sqlx::query_scalar(
        r#"
        SELECT column_name
          FROM information_schema.columns
         WHERE table_schema = 'public'
           AND table_name = $1
           AND is_generated = 'NEVER'
           AND is_identity = 'NO'
         ORDER BY ordinal_position
        "#,
    )
    .bind(table)
    .fetch_all(executor)
    .await?;

    if columns.is_empty() {
        return Err(anyhow!(
            "table `{table}` does not exist (or has no columns) in the target database; run `inkwell db migrate` first"
        ));
    }
    Ok(columns)
}

/// Quote an identifier for interpolation into SQL.
///
/// Table names come from the [`TABLES`] constant and column names come from
/// `information_schema`, so neither is attacker-controlled — but this is SQL
/// built by string concatenation, so refuse anything that could break out of
/// the quotes rather than trusting that invariant to hold forever.
pub fn quote_ident(ident: &str) -> Result<String> {
    if ident.is_empty() || ident.contains('"') || ident.contains('\0') {
        return Err(anyhow!(
            "refusing to quote unsafe SQL identifier: {ident:?}"
        ));
    }
    Ok(format!("\"{ident}\""))
}

/// `"a", "b", "c"` — a quoted, comma-separated column list.
pub fn quote_column_list(columns: &[String]) -> Result<String> {
    let quoted = columns
        .iter()
        .map(|column| quote_ident(column))
        .collect::<Result<Vec<_>>>()?;
    Ok(quoted.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tables_list_orders_dependencies_before_dependents() {
        let position = |name: &str| {
            TABLES
                .iter()
                .position(|table| *table == name)
                .expect("table should be listed")
        };

        // documents.owner_id -> authors, and everything else hangs off one of
        // those two.
        assert!(position("authors") < position("documents"));
        for dependent in [
            "author_tokens",
            "links",
            "slug_aliases",
            "note_chunks",
            "webmentions",
            "write_audit",
            "media",
            "sessions",
            "preview_tokens",
        ] {
            assert!(
                position("authors") < position(dependent),
                "{dependent} references authors and must come after it"
            );
            assert!(
                position("documents") < position(dependent),
                "{dependent} references documents and must come after it"
            );
        }
    }

    #[test]
    fn quote_ident_rejects_quote_injection() {
        assert_eq!(quote_ident("documents").unwrap(), "\"documents\"");
        assert!(quote_ident("doc\"; DROP TABLE documents; --").is_err());
        assert!(quote_ident("").is_err());
    }

    #[test]
    fn media_blobs_is_not_dumped_as_a_table() {
        assert!(
            !TABLES.contains(&"media_blobs"),
            "media_blobs is one backend's private storage; blobs travel as Blob records"
        );
        assert!(
            TABLES.contains(&"media"),
            "media metadata rows are logical data and must be dumped"
        );
    }

    #[test]
    fn manifest_round_trips_as_a_tagged_record() {
        let manifest = Manifest {
            bundle_format: BUNDLE_FORMAT,
            inkwell_version: "0.1.0".to_string(),
            schema_version: 25,
            applied_migrations: vec![1, 25],
            created_at: "2026-07-25T00:00:00Z".to_string(),
            media_backend: "local".to_string(),
            blobs: 2,
            tables: vec![TableSummary {
                name: "documents".to_string(),
                rows: 3,
            }],
        };
        let line = serde_json::to_string(&Record::Manifest(manifest)).unwrap();
        assert!(line.starts_with(r#"{"kind":"manifest","bundleFormat":1"#));

        match serde_json::from_str::<Record>(&line).unwrap() {
            Record::Manifest(parsed) => {
                assert_eq!(parsed.schema_version, 25);
                assert_eq!(parsed.rows_for("documents"), 3);
                assert_eq!(parsed.rows_for("media"), 0);
            }
            other => panic!("expected a manifest record, got {other:?}"),
        }
    }

    #[test]
    fn blob_round_trips_and_verifies_its_content_address() {
        let bytes = b"not really a png, but hashable".to_vec();
        let checksum = crate::media::checksum_hex(&bytes);
        let key = crate::media::storage_key_for(&checksum, "image/png").expect("valid key");

        let encoded = encode_blob(&bytes);
        assert_eq!(decode_blob(&key, &encoded).unwrap(), bytes);

        // A blob whose bytes do not hash to its key is refused, not installed.
        let tampered = encode_blob(b"different bytes entirely");
        let error = decode_blob(&key, &tampered).expect_err("checksum mismatch must fail");
        assert!(error.to_string().contains("failed its checksum"));

        // A malformed key is refused before any decode work.
        assert!(decode_blob("../../etc/passwd", &encoded).is_err());
    }
}
