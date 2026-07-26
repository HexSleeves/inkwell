//! `inkwell backup` — write a restorable bundle for the whole deployment.

use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use flate2::Compression;
use flate2::write::GzEncoder;
use futures_util::TryStreamExt;
use sqlx::{AssertSqlSafe, PgPool};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use super::{
    BUNDLE_FORMAT, Manifest, Record, TABLES, TableSummary, encode_blob, quote_column_list,
    quote_ident, restorable_columns,
};
use crate::db::migrations;
use crate::media::MediaStore;

/// What a completed backup wrote, for the CLI to print and tests to assert on.
#[derive(Clone, Debug)]
pub struct BackupSummary {
    /// `None` when the bundle went to stdout.
    pub path: Option<PathBuf>,
    pub manifest: Manifest,
    pub rows_written: i64,
    /// Blobs actually written. Lower than `manifest.blobs` only when a `media`
    /// row's bytes were already missing from storage.
    pub blobs_written: i64,
}

/// Default bundle filename: sortable, collision-free per second, and obviously
/// an Inkwell bundle rather than a bare `.gz`.
pub fn default_bundle_name(now: OffsetDateTime) -> Result<String> {
    let stamp = now
        .format(&time::macros::format_description!(
            "[year][month][day]T[hour][minute][second]Z"
        ))
        .context("formatting backup timestamp")?;
    Ok(format!("inkwell-backup-{stamp}.inkwell.gz"))
}

/// Dump every table in [`TABLES`] to a gzipped JSON Lines bundle.
///
/// The dump runs inside one `REPEATABLE READ` transaction, so the manifest's row
/// counts, the rows themselves, and cross-table foreign keys all come from a
/// single consistent snapshot even if the server is serving writes.
///
/// Media blobs are read through `media_store`, so the bundle is identical
/// whichever backend the source deployment uses (ADR 0013).
///
/// `destination` of `None` (or `Some("-")`) streams to stdout so the bundle can
/// be piped straight into `gpg`, `aws s3 cp -`, or an ssh pipe.
pub async fn run(
    pool: &PgPool,
    media_store: &dyn MediaStore,
    destination: Option<PathBuf>,
) -> Result<BackupSummary> {
    let to_stdout = destination
        .as_deref()
        .is_none_or(|path| path == Path::new("-"));
    let path = if to_stdout { None } else { destination.clone() };

    let mut tx = pool.begin().await?;
    // Must be the first statement of the transaction. A single snapshot is the
    // whole point: without it a concurrent write could land a `links` row whose
    // `documents` parent is not in the dump.
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ")
        .execute(&mut *tx)
        .await?;

    let applied = migrations::status(&mut *tx).await?;
    let applied_migrations: Vec<i64> = applied.iter().map(|row| row.version).collect();
    let schema_version = applied_migrations.iter().copied().max().unwrap_or(0);

    // Column lists and counts first, so the manifest is complete before a single
    // row is written and a reader can validate the bundle from its first line.
    let mut columns_by_table = Vec::with_capacity(TABLES.len());
    let mut summaries = Vec::with_capacity(TABLES.len());
    for table in TABLES {
        let columns = restorable_columns(&mut *tx, table).await?;
        let quoted_table = quote_ident(table)?;
        let rows: i64 = sqlx::query_scalar(AssertSqlSafe(format!(
            "SELECT count(*) FROM public.{quoted_table}"
        )))
        .fetch_one(&mut *tx)
        .await?;
        summaries.push(TableSummary {
            name: table.to_string(),
            rows,
        });
        columns_by_table.push((table, columns, quoted_table));
    }

    // Distinct storage keys, from the same snapshot as the `media` rows that
    // reference them. Pre-0025 rows carry their bytes inline in `media.data` and
    // have a NULL key — they travel with the row, so they are not listed here.
    let blob_keys: Vec<String> = sqlx::query_scalar(
        "SELECT DISTINCT storage_key FROM media WHERE storage_key IS NOT NULL ORDER BY 1",
    )
    .fetch_all(&mut *tx)
    .await?;

    let manifest = Manifest {
        bundle_format: BUNDLE_FORMAT,
        inkwell_version: env!("CARGO_PKG_VERSION").to_string(),
        schema_version,
        applied_migrations,
        created_at: OffsetDateTime::now_utc()
            .format(&Rfc3339)
            .context("formatting manifest createdAt")?,
        media_backend: media_store.backend().to_string(),
        blobs: blob_keys.len() as i64,
        tables: summaries,
    };

    let sink: Box<dyn Write> = match &path {
        Some(path) => Box::new(BufWriter::new(
            std::fs::File::create(path)
                .with_context(|| format!("creating backup file {}", path.display()))?,
        )),
        None => Box::new(BufWriter::new(std::io::stdout())),
    };
    let mut writer = GzEncoder::new(sink, Compression::default());

    write_line(&mut writer, &Record::Manifest(manifest.clone()))?;

    let mut rows_written: i64 = 0;
    for (table, columns, quoted_table) in &columns_by_table {
        let column_list = quote_column_list(columns)?;
        // `to_jsonb` over a subquery that names only the restorable columns keeps
        // generated columns (documents.search_vector) out of the bundle entirely
        // instead of shipping a tsvector we would have to strip on the way back
        // in. `::text` avoids needing sqlx's `json` feature. `ORDER BY 1` makes
        // bundles byte-stable for the same snapshot, which makes them diffable.
        let sql = format!(
            "SELECT to_jsonb(r)::text FROM (SELECT {column_list} FROM public.{quoted_table} ORDER BY 1) r"
        );
        let mut stream = sqlx::query_scalar::<_, String>(AssertSqlSafe(sql)).fetch(&mut *tx);
        while let Some(json) = stream
            .try_next()
            .await
            .with_context(|| format!("reading rows from {table}"))?
        {
            let data: serde_json::Map<String, serde_json::Value> = serde_json::from_str(&json)
                .with_context(|| format!("parsing row json from {table}"))?;
            write_line(
                &mut writer,
                &Record::Row {
                    table: (*table).to_string(),
                    data,
                },
            )?;
            rows_written += 1;
        }
    }

    // Blobs last, after the `media` rows that name them. Read through the store
    // trait so a filesystem-backed and a Postgres-backed deployment produce the
    // same bundle.
    let mut blobs_written: i64 = 0;
    for key in &blob_keys {
        let Some(bytes) = media_store
            .get(key)
            .await
            .with_context(|| format!("reading media blob {key}"))?
        else {
            // A `media` row whose bytes are gone is a pre-existing integrity
            // problem, not something a backup should paper over or abort on:
            // refusing here would mean a deployment with one missing file could
            // never be backed up at all.
            tracing::warn!(
                storage_key = %key,
                "media blob is missing from storage; backing up the row without its bytes"
            );
            continue;
        };
        write_line(
            &mut writer,
            &Record::Blob {
                key: key.clone(),
                bytes: encode_blob(&bytes),
            },
        )?;
        blobs_written += 1;
    }

    // Read-only transaction; roll it back explicitly rather than leaving the
    // guard to do it silently on drop.
    tx.rollback().await?;

    writer
        .finish()
        .context("finishing gzip stream")?
        .flush()
        .context("flushing backup output")?;

    Ok(BackupSummary {
        path,
        manifest,
        rows_written,
        blobs_written,
    })
}

fn write_line<W: Write>(writer: &mut W, record: &Record) -> Result<()> {
    serde_json::to_writer(&mut *writer, record).context("serialising bundle record")?;
    writer.write_all(b"\n").context("writing bundle newline")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::datetime;

    #[test]
    fn default_bundle_name_is_sortable_and_extension_tagged() {
        let name = default_bundle_name(datetime!(2026-07-25 14:03:09 UTC)).unwrap();
        assert_eq!(name, "inkwell-backup-20260725T140309Z.inkwell.gz");
    }
}
