/**
 * Server entrypoint.
 *
 * Wires a real Postgres pool (resolved from `DATABASE_URL`) to the HTTP server
 * and listens on `PORT`. This is the runnable counterpart to the migration CLI
 * in `src/db/cli.ts`. Build first (`npm run build`), then:
 *
 *   node dist/main.js
 *
 * The repo exposes this as `npm start`. Configuration is read from the
 * environment:
 *
 *   DATABASE_URL   Postgres connection string (required)
 *   PORT           Port to listen on (default 3000)
 *   HOST           Address to bind (default 0.0.0.0)
 *
 * The wiring itself (routing, rendering, persistence) is covered by the
 * framework-free integration tests, so this module stays a thin bootstrap.
 */

import { createPool } from './db/pool.js';
import { createServer } from './server.js';

function parsePort(raw: string | undefined): number {
  if (raw === undefined || raw === '') return 3000;
  const port = Number.parseInt(raw, 10);
  if (!Number.isInteger(port) || port < 0 || port > 65535) {
    throw new Error(`Invalid PORT "${raw}": expected an integer between 0 and 65535.`);
  }
  return port;
}

function main(): void {
  const port = parsePort(process.env.PORT);
  const host = process.env.HOST ?? '0.0.0.0';

  // createPool throws loudly if DATABASE_URL is missing, so misconfiguration
  // fails at startup rather than on the first request.
  const pool = createPool();
  const server = createServer(pool);

  server.listen(port, host, () => {
    console.log(`Inkwell listening on http://${host}:${port}`);
  });

  // Drain connections cleanly on termination so deploys don't drop requests.
  const shutdown = (signal: string): void => {
    console.log(`Received ${signal}, shutting down.`);
    server.close(() => {
      void pool.end().finally(() => process.exit(0));
    });
  };
  process.on('SIGINT', () => shutdown('SIGINT'));
  process.on('SIGTERM', () => shutdown('SIGTERM'));
}

main();
