/**
 * Migration runner.
 *
 * Applies pending {@link Migration}s in ascending id order and records each in
 * the `schema_migrations` ledger so re-runs are idempotent. Rollback runs the
 * reverse SQL for the most recently applied migrations.
 *
 * Migrations are applied sequentially and each is recorded only after its `up`
 * SQL succeeds, so a failed run is safely resumable: already-applied migrations
 * stay applied and the failing one is retried on the next run. Pass a single
 * pooled client (not a multi-connection pool) if you need a migration's
 * statements to share one session.
 */

import type { Queryable } from './pool.js';
import { MIGRATIONS, type Migration } from './migrations.js';

/** Create the ledger table that tracks which migrations have been applied. */
export async function ensureMigrationsTable(db: Queryable): Promise<void> {
  await db.query(`
    CREATE TABLE IF NOT EXISTS schema_migrations (
      id text PRIMARY KEY,
      name text NOT NULL,
      applied_at timestamptz NOT NULL DEFAULT now()
    );
  `);
}

/** Ids of migrations already recorded as applied, in ascending order. */
export async function appliedMigrationIds(db: Queryable): Promise<string[]> {
  await ensureMigrationsTable(db);
  const result = await db.query<{ id: string }>(`SELECT id FROM schema_migrations ORDER BY id ASC`);
  return result.rows.map((row) => row.id);
}

function sortedById(migrations: readonly Migration[]): Migration[] {
  return [...migrations].sort((a, b) => a.id.localeCompare(b.id));
}

/**
 * Apply all pending migrations. Returns the ids applied during this run (empty
 * when the database is already up to date).
 */
export async function migrate(
  db: Queryable,
  migrations: readonly Migration[] = MIGRATIONS,
): Promise<string[]> {
  const done = new Set(await appliedMigrationIds(db));
  const applied: string[] = [];
  for (const migration of sortedById(migrations)) {
    if (done.has(migration.id)) continue;
    await db.query(migration.up);
    await db.query(`INSERT INTO schema_migrations (id, name) VALUES ($1, $2)`, [
      migration.id,
      migration.name,
    ]);
    applied.push(migration.id);
  }
  return applied;
}

export interface RollbackOptions {
  /** How many of the most recent migrations to roll back. Defaults to 1. */
  steps?: number;
}

/**
 * Roll back the most recently applied migrations (newest first). Returns the
 * ids reverted during this run.
 */
export async function rollback(
  db: Queryable,
  options: RollbackOptions = {},
  migrations: readonly Migration[] = MIGRATIONS,
): Promise<string[]> {
  const steps = Math.max(0, options.steps ?? 1);
  const byId = new Map(migrations.map((migration) => [migration.id, migration]));
  const newestFirst = (await appliedMigrationIds(db)).reverse();
  const reverted: string[] = [];
  for (const id of newestFirst.slice(0, steps)) {
    const migration = byId.get(id);
    if (!migration) {
      throw new Error(`Cannot roll back migration ${id}: no matching definition found.`);
    }
    await db.query(migration.down);
    await db.query(`DELETE FROM schema_migrations WHERE id = $1`, [id]);
    reverted.push(id);
  }
  return reverted;
}
