/**
 * Test-only Postgres double.
 *
 * Spins up an in-memory `pg-mem` database that satisfies the {@link Queryable}
 * contract, so the migration runner and data-access layer can be exercised
 * without a live Postgres server. This file is excluded from the published
 * build (see `tsconfig.build.json`) and must never be imported by production
 * code.
 *
 * `pg-mem` implements only a subset of Postgres built-ins, so functions our
 * schema relies on (`gen_random_uuid()`) are registered here. `now()` is
 * supported natively. `noAstCoverageCheck` is enabled because pg-mem otherwise
 * throws on idempotent `CREATE TABLE IF NOT EXISTS` re-runs (it flags the
 * skipped constraint AST as "unread") — valid SQL that real Postgres accepts.
 */

import { randomUUID } from 'node:crypto';

import { DataType, newDb, type IMemoryDb } from 'pg-mem';

import type { Queryable } from './pool.js';

export interface MemoryDatabase {
  /** A `Queryable` backed by the in-memory database. */
  readonly db: Queryable;
  /** The underlying `pg-mem` instance, for advanced assertions if needed. */
  readonly mem: IMemoryDb;
}

/** Create a fresh, isolated in-memory database for a single test. */
export function createMemoryDatabase(): MemoryDatabase {
  const mem = newDb({ noAstCoverageCheck: true });
  mem.public.registerFunction({
    name: 'gen_random_uuid',
    returns: DataType.uuid,
    implementation: () => randomUUID(),
    impure: true,
  });
  const { Pool } = mem.adapters.createPg();
  const pool = new Pool();
  return { db: pool as Queryable, mem };
}
