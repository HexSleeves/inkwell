/**
 * Migration CLI.
 *
 * Operates the migration runner against a real Postgres database resolved from
 * `DATABASE_URL`. Build first (`pnpm run build`), then:
 *
 *   node dist/db/cli.js migrate        # apply all pending migrations
 *   node dist/db/cli.js rollback [n]   # roll back the last n migrations (default 1)
 *   node dist/db/cli.js status         # list applied migration ids
 *
 * The repo exposes these as `pnpm run db:migrate` / `db:rollback` / `db:status`.
 */

import { appliedMigrationIds, migrate, rollback } from './migrate.js';
import { createPool } from './pool.js';

async function main(): Promise<void> {
  const command = process.argv[2] ?? 'migrate';
  const pool = createPool();
  try {
    switch (command) {
      case 'migrate': {
        const applied = await migrate(pool);
        console.log(applied.length ? `Applied: ${applied.join(', ')}` : 'Already up to date.');
        break;
      }
      case 'rollback': {
        const steps = Number.parseInt(process.argv[3] ?? '1', 10);
        const reverted = await rollback(pool, { steps });
        console.log(
          reverted.length ? `Rolled back: ${reverted.join(', ')}` : 'Nothing to roll back.',
        );
        break;
      }
      case 'status': {
        const ids = await appliedMigrationIds(pool);
        console.log(
          ids.length ? `Applied migrations: ${ids.join(', ')}` : 'No migrations applied.',
        );
        break;
      }
      default:
        console.error(`Unknown command "${command}". Use: migrate | rollback [steps] | status`);
        process.exitCode = 1;
    }
  } finally {
    await pool.end();
  }
}

main().catch((error: unknown) => {
  console.error(error);
  process.exitCode = 1;
});
