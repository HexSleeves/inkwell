/**
 * Inkwell — an open, API-first Markdown publishing platform.
 *
 * This is the package entry point. As the core grows it will export the
 * public API surface (document model, rendering pipeline, HTTP server).
 * For now it exposes a small, tested utility so the scaffold has something
 * real to build, lint, and test against.
 */

export const NAME = 'inkwell';
export const VERSION = '0.1.0';

export { renderMarkdown, renderDocumentHtml } from './rendering.js';
export { handleApiRequest, ApiError, type ApiRequest, type ApiResponse } from './api.js';
export { createServer, createRequestListener } from './server.js';
export {
  handlePageRequest,
  renderIndexPage,
  renderDocumentPage,
  renderNotFoundPage,
  escapeHtml,
  type PageRequest,
  type PageResponse,
} from './pages.js';
export { slugify } from './slug.js';
