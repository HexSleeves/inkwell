/**
 * Postgres connection plumbing.
 *
 * The data-access layer and migration runner are written against the small
 * {@link Queryable} interface rather than a concrete `pg.Pool`. That keeps the
 * core decoupled from the driver: production code passes a real `pg` pool or
 * client, while tests pass an in-memory `pg-mem` adapter. Both satisfy the same
 * `query(text, params)` contract.
 */

import pg from 'pg';
import type { QueryResult, QueryResultRow } from 'pg';

const { Pool } = pg;

/**
 * Minimal subset of node-postgres's `Pool`/`PoolClient` that Inkwell depends on.
 * Anything exposing a compatible `query` method (a real pool, a checked-out
 * client, or a test double) can back the persistence layer.
 */
export interface Queryable {
  query<R extends QueryResultRow = QueryResultRow>(
    text: string,
    params?: readonly unknown[],
  ): Promise<QueryResult<R>>;
}

/**
 * Create a Postgres connection pool.
 *
 * Resolves the connection string from the explicit argument or the
 * `DATABASE_URL` environment variable. Throws if neither is provided so
 * misconfiguration fails loudly at startup rather than on first query.
 */
export function createPool(connectionString?: string): pg.Pool {
  const url = connectionString ?? process.env.DATABASE_URL;
  if (!url) {
    throw new Error('No Postgres connection string: pass one to createPool() or set DATABASE_URL.');
  }
  return new Pool({ connectionString: url });
}
