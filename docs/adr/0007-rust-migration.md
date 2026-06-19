# ADR 0007: Rust Migration

Status: accepted

## Decision

Replace the Node runtime with a single Rust binary while preserving the HTTP contract, page routes, Atom feed, sitemap, search behavior, Compose workflow, and CI quality gate.

## Parity rules

- API JSON response shapes remain exact.
- Public HTML parity is semantic and security-focused.
- Search remains `ILIKE`-based during the runtime cutover.
- Docker Compose remains the primary local full-stack workflow.
