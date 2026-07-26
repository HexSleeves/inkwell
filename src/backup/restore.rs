//! `inkwell restore` — load a bundle produced by [`super::create`].
//!
//! Two safety properties the tests pin down:
//!
//! 1. **Never silently clobber.** Restoring into a deployment that already holds
//!    data fails before the first insert unless `--overwrite` is passed.
//! 2. **All or nothing.** Every write happens in one transaction, so a failure
//!    part-way through (bad row, missing column, FK violation) leaves the target
//!    exactly as it was.

use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use flate2::read::GzDecoder;
use sqlx::{AssertSqlSafe, PgPool, Postgres, Transaction};

use super::{
    BOOTSTRAP_ADMIN_ID, BUNDLE_FORMAT, Manifest, Record, TABLES, decode_blob, quote_column_list,
    quote_ident, restorable_columns,
};
use crate::db::migrations;
use crate::media::MediaStore;

/// Rows per `INSERT`. Large enough that a 10k-note garden is a handful of round
/// trips, small enough that a media-heavy bundle does not build a 100 MB
/// parameter string.
const BATCH_ROWS: usize = 500;

#[derive(Clone, Copy, Debug, Default)]
pub struct RestoreOptions {
    /// Required to restore into a deployment that already holds data. Wipes the
    /// backed-up tables first.
    pub overwrite: bool,
}

#[derive(Clone, Debug)]
pub struct RestoreSummary {
    pub manifest: Manifest,
    pub rows_restored: i64,
    pub blobs_restored: i64,
    /// Blobs the target held before an `--overwrite` restore that the bundle does
    /// not contain, deleted after the commit so the previous deployment's images
    /// are not left readable on disk.
    pub blobs_removed: i64,
    /// Non-fatal schema drift between bundle and target, surfaced to the
    /// operator rather than swallowed.
    pub warnings: Vec<String>,
}

/// Restore `source` (a path, or `None`/`-` for stdin) into the database behind
/// `pool`, writing media blobs through `media_store`.
///
/// Migrations run first, so an empty deployment only needs `inkwell restore` —
/// not `db migrate && restore`.
///
/// Blob writes happen through the *target's* store, so a bundle taken from a
/// filesystem-backed deployment restores onto a Postgres-backed one unchanged.
/// They are content-addressed and therefore idempotent and additive, which is
/// what makes it safe to write them before the transaction commits: a failed
/// restore leaves extra unreferenced bytes, never wrong ones.
pub async fn run(
    pool: &PgPool,
    media_store: &dyn MediaStore,
    source: Option<PathBuf>,
    options: RestoreOptions,
) -> Result<RestoreSummary> {
    let reader = open_source(source.as_deref())?;
    let mut lines = BufReader::new(GzDecoder::new(reader)).lines();

    let first = lines
        .next()
        .transpose()
        .context("reading bundle manifest line")?
        .ok_or_else(|| anyhow!("bundle is empty: expected a manifest on the first line"))?;
    let manifest = match parse_record(&first)? {
        Record::Manifest(manifest) => manifest,
        Record::Row { .. } | Record::Blob { .. } => {
            bail!("bundle is malformed: the first line must be the manifest")
        }
    };

    // Refuse before touching the target: an unreadable-format or from-the-future
    // bundle must not leave a half-migrated database behind.
    check_compatibility(&manifest)?;

    migrations::migrate(pool)
        .await
        .context("migrating the restore target to the current schema")?;

    let mut tx = pool.begin().await?;

    // Captured before the wipe: after `--overwrite` these are the blobs the
    // previous deployment referenced, and any the bundle does not bring back are
    // deleted once the restore commits.
    let previous_keys: Vec<String> =
        sqlx::query_scalar("SELECT DISTINCT storage_key FROM media WHERE storage_key IS NOT NULL")
            .fetch_all(&mut *tx)
            .await?;

    if !options.overwrite {
        let occupied = non_empty_tables(&mut tx).await?;
        if !occupied.is_empty() {
            let listed = occupied
                .iter()
                .map(|(table, count)| format!("{table}={count}"))
                .collect::<Vec<_>>()
                .join(", ");
            // Nothing has been written yet, and the transaction rolls back on
            // drop, so the target is untouched.
            bail!(
                "refusing to restore: target deployment is not empty ({listed}). \
                 Nothing was changed. Re-run with --overwrite to replace this data."
            );
        }
    }

    // Unconditional, in both paths. Even an "empty" target is not row-empty:
    // `migrations::migrate` above seeded the bootstrap admin, and the bundle
    // carries its own copy of that same fixed-uuid row. Without the wipe, the
    // very first insert would collide on `authors_pkey`. The emptiness check has
    // already proven there is no user data to lose here.
    wipe(&mut tx).await?;

    let mut warnings = Vec::new();
    let mut rows_restored: i64 = 0;
    let mut blobs_restored: i64 = 0;
    let mut restored_keys = std::collections::HashSet::new();
    let mut batch = Batch::default();

    for line in lines {
        let line = line.context("reading bundle line")?;
        if line.trim().is_empty() {
            continue;
        }
        match parse_record(&line)? {
            Record::Manifest(_) => bail!("bundle is malformed: a second manifest line was found"),
            Record::Row { table, data } => {
                if !TABLES.contains(&table.as_str()) {
                    // A bundle from a newer Inkwell whose table we dropped, or
                    // one we no longer back up. Skip loudly; do not guess.
                    let warning = format!(
                        "skipped rows for unknown table `{table}` (not part of this version's backup set)"
                    );
                    if !warnings.contains(&warning) {
                        warnings.push(warning);
                    }
                    continue;
                }
                if batch.table.as_deref() != Some(table.as_str()) {
                    rows_restored += batch.flush(&mut tx).await?;
                    let (columns, mut column_warnings) =
                        resolve_columns(&mut tx, &table, &data).await?;
                    warnings.append(&mut column_warnings);
                    batch.start(table.clone(), columns);
                }
                batch.push(serde_json::Value::Object(data));
                if batch.rows.len() >= BATCH_ROWS {
                    rows_restored += batch.flush(&mut tx).await?;
                }
            }
            Record::Blob { key, bytes } => {
                // Any pending rows go in first so a blob failure cannot leave the
                // batch silently dropped.
                rows_restored += batch.flush(&mut tx).await?;
                let decoded = decode_blob(&key, &bytes)?;
                media_store
                    .put(&key, &decoded)
                    .await
                    .with_context(|| format!("writing media blob {key}"))?;
                restored_keys.insert(key);
                blobs_restored += 1;
            }
        }
    }
    rows_restored += batch.flush(&mut tx).await?;

    tx.commit().await?;

    // Only after the commit: if the restore had failed, these blobs would still
    // be the ones the (rolled-back) database referenced.
    let mut blobs_removed: i64 = 0;
    for key in previous_keys {
        if restored_keys.contains(&key) {
            continue;
        }
        match media_store.delete(&key).await {
            Ok(()) => blobs_removed += 1,
            Err(error) => warnings.push(format!(
                "could not delete superseded media blob {key}: {error}"
            )),
        }
    }

    Ok(RestoreSummary {
        manifest,
        rows_restored,
        blobs_restored,
        blobs_removed,
        warnings,
    })
}

/// Refuse bundles this binary cannot faithfully read.
///
/// The important case is a bundle from a *newer* deployment: its rows may carry
/// columns and invariants this binary has never seen, and inserting them would
/// either fail confusingly or silently drop data.
fn check_compatibility(manifest: &Manifest) -> Result<()> {
    if manifest.bundle_format > BUNDLE_FORMAT {
        bail!(
            "bundle format {} is newer than this binary supports (max {}). \
             Upgrade Inkwell (bundle written by version {}) and retry.",
            manifest.bundle_format,
            BUNDLE_FORMAT,
            manifest.inkwell_version
        );
    }

    let known = migrations::latest_known_schema_version();
    if manifest.schema_version > known {
        bail!(
            "bundle schema version {} is newer than this binary knows ({}). \
             The bundle was written by Inkwell {}; upgrade to that version or later before restoring. \
             Nothing was changed.",
            manifest.schema_version,
            known,
            manifest.inkwell_version
        );
    }
    Ok(())
}

/// Tables that hold data, ignoring the bootstrap admin author that migration
/// 0015 seeds into every deployment.
async fn non_empty_tables(tx: &mut Transaction<'_, Postgres>) -> Result<Vec<(String, i64)>> {
    let mut occupied = Vec::new();
    for table in TABLES {
        let quoted = quote_ident(table)?;
        let sql = if table == "authors" {
            format!("SELECT count(*) FROM public.{quoted} WHERE id <> '{BOOTSTRAP_ADMIN_ID}'::uuid")
        } else {
            format!("SELECT count(*) FROM public.{quoted}")
        };
        let count: i64 = sqlx::query_scalar(AssertSqlSafe(sql))
            .fetch_one(&mut **tx)
            .await?;
        if count > 0 {
            occupied.push((table.to_string(), count));
        }
    }
    Ok(occupied)
}

/// Empty every backed-up table. One `TRUNCATE` so foreign keys between them
/// never transiently break; `CASCADE` covers the referencing tables (all of
/// which are in the list anyway).
async fn wipe(tx: &mut Transaction<'_, Postgres>) -> Result<()> {
    let quoted = TABLES
        .iter()
        .map(|table| quote_ident(table).map(|ident| format!("public.{ident}")))
        .collect::<Result<Vec<_>>>()?
        .join(", ");
    sqlx::query(AssertSqlSafe(format!(
        "TRUNCATE TABLE {quoted} RESTART IDENTITY CASCADE"
    )))
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Columns to write for `table`: the intersection of what the target accepts and
/// what the bundle actually carries, plus warnings for each side of the
/// difference.
async fn resolve_columns(
    tx: &mut Transaction<'_, Postgres>,
    table: &str,
    sample: &serde_json::Map<String, serde_json::Value>,
) -> Result<(Vec<String>, Vec<String>)> {
    let target = restorable_columns(&mut **tx, table).await?;
    let mut warnings = Vec::new();

    let intersection: Vec<String> = target
        .iter()
        .filter(|column| sample.contains_key(column.as_str()))
        .cloned()
        .collect();

    let missing: Vec<&String> = target
        .iter()
        .filter(|column| !sample.contains_key(column.as_str()))
        .collect();
    if !missing.is_empty() {
        warnings.push(format!(
            "{table}: bundle predates column(s) {missing:?}; they will take their schema defaults"
        ));
    }

    let extra: Vec<&String> = sample
        .keys()
        .filter(|column| !target.contains(column))
        .collect();
    if !extra.is_empty() {
        warnings.push(format!(
            "{table}: bundle carries column(s) {extra:?} that no longer exist here; ignoring them"
        ));
    }

    if intersection.is_empty() {
        bail!("{table}: bundle rows share no columns with the target schema");
    }
    Ok((intersection, warnings))
}

/// Accumulates rows for one table so they insert in batches instead of one
/// statement per row.
#[derive(Default)]
struct Batch {
    table: Option<String>,
    columns: Vec<String>,
    rows: Vec<serde_json::Value>,
}

impl Batch {
    fn start(&mut self, table: String, columns: Vec<String>) {
        self.table = Some(table);
        self.columns = columns;
        self.rows.clear();
    }

    fn push(&mut self, row: serde_json::Value) {
        self.rows.push(row);
    }

    /// Insert and clear. Returns the number of rows written.
    async fn flush(&mut self, tx: &mut Transaction<'_, Postgres>) -> Result<i64> {
        let Some(table) = self.table.clone() else {
            return Ok(0);
        };
        if self.rows.is_empty() {
            return Ok(0);
        }

        let quoted_table = quote_ident(&table)?;
        let column_list = quote_column_list(&self.columns)?;
        // `jsonb_populate_recordset` uses the table's own row type, so every
        // column is parsed by its real input function — `bytea` hex, pgvector
        // `[…]` literals, `timestamptz`, arrays — without a per-type branch here.
        let sql = format!(
            "INSERT INTO public.{quoted_table} ({column_list}) \
             SELECT {column_list} FROM jsonb_populate_recordset(NULL::public.{quoted_table}, $1::jsonb)"
        );
        let payload = serde_json::Value::Array(std::mem::take(&mut self.rows));
        let rows = payload.as_array().map(Vec::len).unwrap_or(0) as i64;
        sqlx::query(AssertSqlSafe(sql))
            .bind(serde_json::to_string(&payload).context("serialising insert batch")?)
            .execute(&mut **tx)
            .await
            .with_context(|| format!("restoring rows into {table}"))?;
        Ok(rows)
    }
}

fn open_source(source: Option<&Path>) -> Result<Box<dyn Read>> {
    match source {
        None => Ok(Box::new(std::io::stdin())),
        Some(path) if path == Path::new("-") => Ok(Box::new(std::io::stdin())),
        Some(path) => {
            Ok(Box::new(std::fs::File::open(path).with_context(|| {
                format!("opening bundle {}", path.display())
            })?))
        }
    }
}

fn parse_record(line: &str) -> Result<Record> {
    serde_json::from_str(line).context("parsing bundle line as a JSON Lines record")
}

#[cfg(test)]
mod tests {
    use super::*;
    fn manifest(schema_version: i64, bundle_format: u32) -> Manifest {
        Manifest {
            bundle_format,
            inkwell_version: "9.9.9".to_string(),
            schema_version,
            applied_migrations: (1..=schema_version).collect(),
            created_at: "2026-07-25T00:00:00Z".to_string(),
            media_backend: "local".to_string(),
            blobs: 0,
            tables: Vec::new(),
        }
    }

    #[test]
    fn accepts_a_bundle_at_the_current_schema_version() {
        let current = migrations::latest_known_schema_version();
        check_compatibility(&manifest(current, BUNDLE_FORMAT)).expect("same version must restore");
    }

    #[test]
    fn accepts_an_older_bundle() {
        let older = migrations::latest_known_schema_version() - 1;
        check_compatibility(&manifest(older, BUNDLE_FORMAT)).expect("older bundle must restore");
    }

    #[test]
    fn refuses_a_bundle_from_a_newer_schema() {
        let newer = migrations::latest_known_schema_version() + 1;
        let error = check_compatibility(&manifest(newer, BUNDLE_FORMAT))
            .expect_err("newer schema must be refused");
        let message = error.to_string();
        assert!(
            message.contains(&newer.to_string()),
            "error must state the bundle's schema version: {message}"
        );
        assert!(
            message.contains("newer than this binary knows"),
            "error must name the mismatch: {message}"
        );
        assert!(
            message.contains("Nothing was changed"),
            "error must state that the target is untouched: {message}"
        );
    }

    #[test]
    fn refuses_a_newer_bundle_format() {
        let error = check_compatibility(&manifest(1, BUNDLE_FORMAT + 1))
            .expect_err("newer bundle format must be refused");
        assert!(error.to_string().contains("bundle format"));
    }
}
