import { defineConfig } from 'vitest/config';

export default defineConfig({
  test: {
    environment: 'node',
    include: ['src/**/*.test.ts'],
    coverage: {
      provider: 'v8',
      reporter: ['text', 'lcov'],
      include: ['src/**/*.ts'],
      exclude: ['src/**/*.test.ts'],
      // Regression gate: fail CI if coverage drops below these floors. Set with
      // headroom under current levels (~86% lines/stmts, ~80% branches) so the
      // thin process/CLI entrypoints (main.ts, db/cli.ts, db/pool.ts) don't make
      // the gate flaky, while still catching a real regression in the core.
      thresholds: {
        statements: 80,
        branches: 75,
        functions: 80,
        lines: 80,
      },
    },
  },
});
