/**
 * Public surface of the persistence layer.
 *
 * Import database primitives from `inkwell/db` (or relative `./db/index.js`)
 * rather than reaching into individual modules. Test-only helpers are
 * deliberately not re-exported here.
 */

export { createPool, type Queryable } from './pool.js';
export { MIGRATIONS, type Migration } from './migrations.js';
export {
  ensureMigrationsTable,
  appliedMigrationIds,
  migrate,
  rollback,
  type RollbackOptions,
} from './migrate.js';
export {
  createDocument,
  getDocumentBySlug,
  getDocumentById,
  listDocuments,
  countDocuments,
  updateDocumentBySlug,
  setDocumentStatus,
  deleteDocumentBySlug,
  asDocumentStatus,
  DOCUMENT_STATUSES,
  DuplicateSlugError,
  type Document,
  type DocumentStatus,
  type NewDocument,
  type DocumentPatch,
  type ListOptions,
  type StatusFilter,
} from './documents.js';
