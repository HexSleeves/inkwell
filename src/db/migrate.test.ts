import { beforeEach, describe, expect, it } from 'vitest';

import { appliedMigrationIds, migrate, rollback } from './migrate.js';
import { MIGRATIONS } from './migrations.js';
import { createMemoryDatabase } from './test-helpers.js';
import type { Queryable } from './pool.js';

describe('migrate', () => {
  let db: Queryable;

  beforeEach(() => {
    db = createMemoryDatabase().db;
  });

  it('applies all migrations to a fresh database', async () => {
    const applied = await migrate(db);
    expect(applied).toEqual(MIGRATIONS.map((m) => m.id));
    expect(await appliedMigrationIds(db)).toEqual(['0001']);
  });

  it('records the migration name in the ledger', async () => {
    await migrate(db);
    const ledger = await db.query<{ id: string; name: string }>(
      `SELECT id, name FROM schema_migrations ORDER BY id`,
    );
    expect(ledger.rows).toEqual([{ id: '0001', name: 'create_documents' }]);
  });

  it('creates a usable documents table', async () => {
    await migrate(db);
    const inserted = await db.query<{ slug: string }>(
      `INSERT INTO documents (slug, title, body_markdown, rendered_html)
       VALUES ($1, $2, $3, $4) RETURNING slug`,
      ['hello-world', 'Hello World', '# Hi', '<h1>Hi</h1>'],
    );
    expect(inserted.rows[0]?.slug).toBe('hello-world');
  });

  it('is idempotent — a second run applies nothing', async () => {
    await migrate(db);
    const secondRun = await migrate(db);
    expect(secondRun).toEqual([]);
    expect(await appliedMigrationIds(db)).toEqual(['0001']);
  });

  it('rolls back the most recent migration', async () => {
    await migrate(db);
    const reverted = await rollback(db);
    expect(reverted).toEqual(['0001']);
    expect(await appliedMigrationIds(db)).toEqual([]);

    // The down SQL ran: the documents table is gone.
    await expect(db.query(`SELECT 1 FROM documents`)).rejects.toThrow();

    // NOTE: re-applying in the same instance is intentionally not asserted
    // here. pg-mem does not release the implicit `documents_pkey` index name on
    // DROP TABLE, so a re-CREATE throws — a pg-mem limitation, not Inkwell
    // behaviour. Clean application is covered by "applies all migrations to a
    // fresh database" above (each test gets a fresh database).
  });

  it('does nothing when there is nothing to roll back', async () => {
    expect(await rollback(db)).toEqual([]);
  });
});
