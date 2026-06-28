# Spike 002: Write-Audit History API

## Decision

Expose the existing append-only `write_audit` trail through a read-only document
history endpoint:

```text
GET /documents/{slug}/history?limit=20&offset=0
```

The response is an event log, not a version diff or restore surface:

```json
{
  "slug": "example-note",
  "history": [
    {
      "action": "publish",
      "actorLabel": "shared-key",
      "at": "2026-06-28T12:00:00Z"
    }
  ]
}
```

Each item contains the recorded action, actor label, and event timestamp. Events
are ordered newest first.

## Auth And Visibility

History is visible to admins and to the document owner only.

This matches ADR 0009's ownership model: the bootstrap/shared admin principal can
inspect all document history, while a scoped author with `read` scope can inspect
only their own document history. Anonymous callers, tokens without `read`, and
other authors receive the same document-not-found response used by the document
API. The endpoint does not use the general `Visibility::Owner` predicate because
that predicate intentionally includes all published notes; history exposes actor
labels, so published-but-not-owned documents are still outside this surface.

## Lookup Strategy

The public API addresses documents by slug, but audit rows store both
`document_id` and a slug snapshot. The endpoint resolves the current slug to the
current document id under the admin-or-owner rule, then reads audit rows by
`document_id`.

This keeps the endpoint simple and index-friendly through
`write_audit_document_id_at_idx`. The tradeoff is that a request using an old
slug after a rename does not include history through the retired slug. The audit
rows still survive and keep their slug snapshots, so a future admin-only global
audit endpoint could expose rename-era or deleted-document history.

## Pagination

The endpoint accepts `limit` and `offset`.

- default `limit`: `DEFAULT_LIMIT` from `src/domain/document.rs`
- max `limit`: `MAX_LIMIT` from `src/domain/document.rs`
- `offset`: defaults to `0`
- invalid non-integer values return `400`
- `limit=0` returns `400`

## Deleted Documents

`write_audit.document_id` intentionally is not a foreign key, so audit rows
survive document deletion. `GET /documents/{slug}/history` still returns `404`
once the document no longer exists because the slug cannot resolve to a current
document id. A future admin-only audit search can expose deleted-document history
without changing this document-scoped route.

## Future Options

The existing audit table does not store body snapshots. If the write path later
records snapshots or diffs, this route can either grow richer event payloads or a
new restore-oriented endpoint can be added. That is separate from this spike.
