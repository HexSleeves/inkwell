# Plan 007: Add conditional GET caching to public HTML and XML

Executor instructions: Follow this after Plan 016 and coordinate with Plan 019. Run every verification command. If a STOP condition occurs, stop and report. When done, update this plan's row in plans/README.md.

Drift check: git diff --stat 8bcd1ea..HEAD -- src/http src/views tests Cargo.toml

## Status

- Priority: P3
- Effort: M
- Risk: MED
- Depends on: 016
- Category: performance
- Planned at: commit 8bcd1ea, 2026-06-19

## Why this matters

Public pages, feed, and sitemap are deterministic for a given document set but currently emit no ETag, Last-Modified, or Cache-Control. Every crawler hit and browser refresh does a full DB read and render.

## Current state

- src/http/pages.rs returns Html<String> with no cache headers.
- src/http/feed.rs and src/http/sitemap.rs return content type plus body only.
- src/http/search.rs renders HTML/JSON search without cache headers.
- Plan 016 should land first so this plan does not preserve swallowed DB errors.

## Commands

- cargo fmt --check
- cargo clippy --all-targets --all-features -- -D warnings
- cargo test --all

## Scope

In scope: src/http/pages.rs, src/http/feed.rs, src/http/sitemap.rs, src/http/search.rs only if caching search is included, a small helper module under src/http, tests.
Out of scope: service workers, background cache storage, body shape changes, authenticated mutation/API write caching.

## Steps

1. Add a shared cache helper that:
   - computes an ETag from response bytes and route key,
   - sets ETag and Cache-Control,
   - checks If-None-Match,
   - returns 304 Not Modified with no body when matched.

2. Apply it to public HTML and XML routes:
   - index, page, document, tags, tag pages
   - feed.xml
   - sitemap.xml and any sitemap part routes from Plan 019

3. Use conservative Cache-Control such as public, max-age=60, stale-while-revalidate=300 unless product requirements say otherwise.

4. Add tests:
   - first request returns body plus ETag,
   - second request with matching If-None-Match returns 304 and empty body,
   - content type remains correct,
   - write API responses are not cached.

5. Run verification.

## Done criteria

- Public HTML/XML read responses include ETag and Cache-Control.
- Matching If-None-Match returns 304.
- Authenticated mutation/API write responses do not gain caching accidentally.
- Verification commands pass.
- plans/README.md marks plan 007 DONE.

## STOP conditions

- Plan 016 has not landed.
- Plan 019 changes sitemap routing in a way that makes route keys ambiguous.
- Caching requires changing public response bodies.

