# v0.2 QA Matrix and Smoke-Test Checklist

This document defines the quality bar for the v0.2 release.  It is executable
by any maintainer who did not write the features.  Each section states which
checks are automated (name the cargo test), which are manual, the required
environment, and an explicit pass/fail evidence field.

---

## Environment Combinations

| Label | DATABASE_URL | INKWELL_API_KEY | VOYAGE_API_KEY | ANTHROPIC_API_KEY | INKWELL_SITE_URL | Notes |
|-------|-------------|-----------------|----------------|-------------------|-----------------|-------|
| **offline** | — | — | — | — | — | No DB; compile + unit tests only |
| **minimal** | set | set | — | — | optional | Core API + HTML; MockEmbedder |
| **ai-full** | set | set | set | set | set | Real embeddings + `/ask` synthesis |
| **compose** | set by Compose | set in `.env` | optional | optional | optional | One-command local stack |

> `INKWELL_WRITE_RATE_LIMIT` defaults to `60` req/min in all environments.
> `INKWELL_TRUST_FORWARDED_HEADERS` defaults to `false`; only set `true`
> behind a known reverse proxy (Railway sets this automatically).

---

## 1. Build

### 1a. Automated

| Test / command | Env | Pass criterion | Evidence field |
|----------------|-----|----------------|----------------|
| `cargo fmt --all -- --check` | offline | zero diff | [ ] |
| `cargo clippy --all-targets --all-features --locked -- -D warnings` | offline | zero warnings | [ ] |
| `cargo build --release --bin inkwell --locked` | offline | exit 0 | [ ] |
| CI `fmt` job | offline | green | [ ] |
| CI `clippy` job | offline | green | [ ] |
| CI `build-release` job | offline | green | [ ] |
| CI `docker` job | offline | green | [ ] |

### 1b. Manual smoke

| Step | Env | Pass criterion | Evidence field |
|------|-----|----------------|----------------|
| `docker compose up --build` | compose | all services healthy in < 60 s | [ ] |
| `./inkwell --version` (release binary) | offline | prints `inkwell 0.1.x` | [ ] |
| `./inkwell --help` | offline | usage block shown, no panic | [ ] |

---

## 2. Migrations

### 2a. Automated

| Test | Env | Pass criterion | Evidence field |
|------|-----|----------------|----------------|
| `scoped_tokens_slice1::foundation_schema_exists` | minimal | tables + columns present after migrate | [ ] |
| `scoped_tokens_slice4::owner_id_not_null_with_admin_default` | minimal | `documents.owner_id NOT NULL` constraint holds | [ ] |

Run with:
```
INKWELL_REQUIRE_DB_TESTS=1 cargo test --all --locked 2>&1 | grep -E "PASS|FAIL|test result"
```

### 2b. Manual smoke

| Step | Env | Pass criterion | Evidence field |
|------|-----|----------------|----------------|
| Fresh DB: `cargo run -- db migrate` | minimal | exits 0, 21 migrations applied | [ ] |
| Re-run migrate (idempotent) | minimal | exits 0, `0 migrations applied` | [ ] |
| pgvector check: `psql -c "SELECT 'ok' FROM pg_extension WHERE extname='vector'"` | minimal | returns `ok` | [ ] |

**Known gap:** no automated test for a failed/partial migration rollback path.

---

## 3. DB-backed Integration Tests

Run the full suite against a live DB:

```bash
INKWELL_REQUIRE_DB_TESTS=1 cargo test --all --locked
```

Without `DATABASE_URL` set this fails fast (enforced by
`db_requirements::require_db_tests_errors_without_database_url`).

### 3a. Seed

| Test | Pass criterion | Evidence field |
|------|----------------|----------------|
| `seed_idempotency::seed_creates_published_notes_with_backlinks` | seed creates ≥ 1 published note with backlinks | [ ] |
| `seed_idempotency::seed_is_idempotent_and_does_not_duplicate` | second `seed` run does not duplicate rows | [ ] |
| `scoped_tokens_slice1::bootstrap_admin_author_is_seeded` | admin author row exists after migrate | [ ] |
| `scoped_tokens_slice1::seed_and_backfill_are_idempotent` | backfill is safe to run twice | [ ] |

### 3b. Core API (CRUD + visibility)

| Test | Pass criterion | Evidence field |
|------|----------------|----------------|
| `api_contract::create_and_fetch_document` | POST → 201, GET → 200, body round-trips | [ ] |
| `api_contract::create_exposes_growth_in_the_envelope_defaulting_to_seedling` | `growthStage: "seedling"` in response | [ ] |
| `api_contract::graph_route_hides_drafts_from_public_callers` | unauthenticated `/graph` omits drafts | [ ] |
| `links_contract::backlinks_returns_each_linking_source_deduped_and_ordered` | backlinks correct + ordered | [ ] |
| `links_contract::backlinks_never_leak_a_draft_source_to_public_callers` | no draft slugs in public backlinks | [ ] |
| `links_contract::garden_graph_never_leaks_a_draft_node_or_edge_to_public` | draft nodes absent from public graph | [ ] |
| `links_contract::garden_graph_bounds_the_node_count` | node count ≤ cap | [ ] |
| `links_contract::updating_a_document_bumps_its_version` | version increments on PATCH | [ ] |
| `links_contract::documents_carry_a_version_defaulting_to_one` | new doc `version = 1` | [ ] |

### 3c. Auth / scoped tokens

| Test | Pass criterion | Evidence field |
|------|----------------|----------------|
| `scoped_tokens_slice1::mutations_emit_one_audit_row_each` | every write lands one audit row | [ ] |
| `scoped_tokens_slice3::write_scope_and_ownership_are_enforced` | wrong-owner write → 404; missing scope → 403 | [ ] |
| `scoped_tokens_slice3::publish_scope_and_ownership_are_enforced` | publish scope + ownership enforced atomically | [ ] |
| `scoped_tokens_slice3::read_scope_gates_writes_and_draft_visibility` | `read`-scope token sees own drafts, not others' | [ ] |
| `scoped_tokens_slice3::admin_bypasses_ownership` | admin key updates any doc | [ ] |
| `scoped_tokens_slice3b::*` (8 tests) | owner-aware visibility across all read surfaces | [ ] |
| `scoped_tokens_slice4::scoped_token_drives_the_mcp_client_surface` | MCP authenticates via scoped token | [ ] |
| `token_admin_ux::*` | token create/list/revoke admin UX | [ ] |

### 3d. Slug rename

| Test | Pass criterion | Evidence field |
|------|----------------|----------------|
| `slug_rename_contract::owner_rename_redirects_old_slug` | old slug returns 301 to new slug | [ ] |
| `slug_rename_contract::chained_renames_resolve_to_current_slug` | two renames chain correctly | [ ] |
| `slug_rename_contract::rename_to_existing_slug_conflicts` | 409 Conflict on collision | [ ] |
| `slug_rename_contract::non_owner_cannot_rename` | 404 for non-owner rename | [ ] |
| `slug_rename_contract::read_only_token_cannot_rename` | 403 for read-only token | [ ] |

---

## 4. Security Headers

### 4a. Automated (no DB required)

| Test | Pass criterion | Evidence field |
|------|----------------|----------------|
| `security_headers_contract::html_responses_include_csp_and_hardening_headers` | CSP present with `default-src 'self'`, `object-src 'none'`, `frame-ancestors 'none'`, nonce-gated `script-src`; `X-Content-Type-Options: nosniff`; `Referrer-Policy: strict-origin-when-cross-origin`; `Permissions-Policy` present | [ ] |
| `security_headers_contract::json_responses_keep_hardening_headers_without_csp` | no `Content-Security-Policy` on JSON; `X-Content-Type-Options: nosniff` present | [ ] |

Run with:
```
cargo test --test security_headers_contract
```

### 4b. Manual smoke

| Step | Env | Pass criterion | Evidence field |
|------|-----|----------------|----------------|
| `curl -sI http://localhost:3000/ \| grep -i "content-security-policy"` | compose | CSP header present | [ ] |
| `curl -sI http://localhost:3000/documents \| grep -i "x-content-type"` | compose | `nosniff` | [ ] |

---

## 5. Rate Limiting (CIL-128)

### 5a. Automated (DB-backed)

| Test | Pass criterion | Evidence field |
|------|----------------|----------------|
| `rate_limit_contract::write_burst_over_limit_returns_429_with_retry_after` | burst over limit → 429; `Retry-After` ≥ 1 s | [ ] |
| `rate_limit_contract::read_routes_and_public_site_are_not_rate_limited` | GET + public HTML never throttled, even after write bucket exhausted | [ ] |
| `rate_limit_contract::invalid_api_keys_cannot_bypass_the_limiter` | distinct invalid keys share IP bucket; 429 reached | [ ] |
| `rate_limit_contract::anonymous_callers_are_keyed_by_peer_ip` | different peer IPs get independent buckets | [ ] |

Run with:
```
INKWELL_REQUIRE_DB_TESTS=1 cargo test --test rate_limit_contract
```

### 5b. Manual smoke

| Step | Env | Pass criterion | Evidence field |
|------|-----|----------------|----------------|
| Send 65 POSTs in < 1 min to `/documents` with valid key | compose (default limit 60) | 6th+ request returns `429`; `Retry-After` header present | [ ] |
| `GET /documents` after write bucket exhausted | compose | `200 OK`, no `429` | [ ] |
| Set `INKWELL_WRITE_RATE_LIMIT=0` and send a write | compose | `201 Created` (limiter disabled) | [ ] |

---

## 6. Correlation IDs (CIL-125)

### 6a. Automated (DB-backed)

| Test | Pass criterion | Evidence field |
|------|----------------|----------------|
| `request_id_contract::generates_request_id_when_none_supplied` | response carries `X-Request-Id`; value is a valid UUID | [ ] |
| `request_id_contract::echoes_well_formed_inbound_request_id` | well-formed inbound id echoed unchanged | [ ] |
| `request_id_contract::malformed_inbound_request_id_is_replaced` | malformed id replaced with a valid UUID | [ ] |
| `request_id_contract::error_body_carries_request_id_matching_the_header` | `error.requestId` in 401 body matches `X-Request-Id` header | [ ] |

Run with:
```
INKWELL_REQUIRE_DB_TESTS=1 cargo test --test request_id_contract
```

### 6b. Manual smoke

| Step | Env | Pass criterion | Evidence field |
|------|-----|----------------|----------------|
| `curl -si http://localhost:3000/health` | compose | `x-request-id` header in response | [ ] |
| `curl -si -H "X-Request-Id: my-trace-42" http://localhost:3000/health` | compose | response echoes `x-request-id: my-trace-42` | [ ] |
| Send unauthenticated POST; inspect JSON body | compose | `error.requestId` field present and non-empty | [ ] |

---

## 7. Public Pages (HTML)

### 7a. Automated (no DB required)

| Test | Pass criterion | Evidence field |
|------|----------------|----------------|
| `view_layout_contract::render_page_emits_valid_html_attributes` | rendered HTML has no bare attribute values | [ ] |
| `view_layout_contract::render_page_omits_tailwind_browser_build` | CDN Tailwind runtime absent from HTML | [ ] |
| `view_layout_contract::render_page_allows_json_ld_with_csp_nonce_and_without_tailwind_runtime` | JSON-LD present; nonce applied | [ ] |
| `view_layout_contract::document_page_with_tags_has_no_literal_backslash_n` | no `\n` in rendered tag HTML | [ ] |
| `view_layout_contract::index_listing_with_tags_has_no_literal_backslash_n` | no `\n` in index listing | [ ] |

### 7b. Manual smoke

| Step | Env | Pass criterion | Evidence field |
|------|-----|----------------|----------------|
| `curl -si http://localhost:3000/` | compose | `200 OK`; HTML body contains `<title>` | [ ] |
| Visit a published document page in browser | compose | page renders; no raw Markdown visible | [ ] |
| Visit `/tags` index | compose | `200 OK`; at least one tag listed | [ ] |
| Visit a tag page (e.g. `/tags/test`) | compose | `200 OK` | [ ] |

---

## 8. Feed and Sitemap

### 8a. Automated

No dedicated automated test for feed/sitemap HTML output (known gap — see §13).

### 8b. Manual smoke

| Step | Env | Pass criterion | Evidence field |
|------|-----|----------------|----------------|
| `curl -si http://localhost:3000/feed.xml` | compose | `200 OK`; `Content-Type: application/atom+xml` or `application/rss+xml`; contains `<entry>` or `<item>` | [ ] |
| `curl -si http://localhost:3000/sitemap.xml` | compose | `200 OK`; `Content-Type: application/xml`; `<sitemapindex>` or `<urlset>` present | [ ] |
| `curl -si http://localhost:3000/sitemap-static.xml` | compose | `200 OK`; well-formed XML | [ ] |
| `curl -si "http://localhost:3000/sitemaps/documents/1"` | compose | `200 OK` or `404` (no documents yet) | [ ] |

---

## 9. API Writes

### 9a. Automated (DB-backed)

Covered by `api_contract`, `scoped_tokens_slice*`, and `links_contract` suites above.

### 9b. Manual smoke

| Step | Env | Pass criterion | Evidence field |
|------|-----|----------------|----------------|
| `curl -s -X POST http://localhost:3000/documents -H "Authorization: Bearer $INKWELL_API_KEY" -H "Content-Type: application/json" -d '{"title":"Smoke test","bodyMarkdown":"Hello **world**"}' \| jq .slug` | compose | slug printed (e.g. `"smoke-test"`) | [ ] |
| `curl -s http://localhost:3000/documents/smoke-test \| jq .status` | compose | `"draft"` | [ ] |
| `curl -s -X POST http://localhost:3000/documents/smoke-test/publish -H "Authorization: Bearer $INKWELL_API_KEY" \| jq .status` | compose | `"published"` | [ ] |
| `curl -si http://localhost:3000/smoke-test` | compose | `200 OK`; HTML body shows "Hello world" | [ ] |
| Unauthenticated write: omit `Authorization` header | compose | `401 Unauthorized` | [ ] |
| Write with revoked token | compose | `401 Unauthorized` | [ ] |

---

## 10. Search

### 10a. Automated (DB-backed, MockEmbedder)

| Test | Pass criterion | Evidence field |
|------|----------------|----------------|
| `ai_contract::fts_search_matches_body_and_ranks_title_first` | title match ranked above body match | [ ] |
| `ai_contract::fts_search_excludes_drafts_from_public_results` | drafts absent from unauthenticated `/search` | [ ] |
| `ai_contract::fts_search_tolerates_punctuation_in_query` | query with punctuation returns results, no 500 | [ ] |
| `ai_contract::related_returns_nearest_published_notes` | `/documents/{slug}/related` returns neighbours | [ ] |
| `ai_contract::related_hides_drafts_from_public_callers` | no drafts in related results | [ ] |
| `ai_contract::related_404s_for_unknown_or_draft_slug` | 404 for unknown slug | [ ] |
| `ai_contract::ask_known_answer_retrieves_and_cites_the_right_note` | MockLlm returns answer citing the right note | [ ] |
| `ai_contract::ask_reports_not_configured_without_anthropic_key` | graceful "AI not configured" with no key | [ ] |
| `ai_contract::ask_empty_query_is_a_bad_request` | 400 on empty query | [ ] |
| `ai_contract::ask_rejects_overlong_get_query_before_provider_work` | 400 on oversized GET `?q=` | [ ] |

Run with:
```
INKWELL_REQUIRE_DB_TESTS=1 cargo test --test ai_contract
```

### 10b. Manual smoke

| Step | Env | Pass criterion | Evidence field |
|------|-----|----------------|----------------|
| `curl "http://localhost:3000/search?q=hello"` | compose | `200 OK`; JSON array (may be empty on fresh DB) | [ ] |
| `curl "http://localhost:3000/ask?q=What+is+inkwell"` | compose | `200 OK`; `answer` field present (or "AI not configured") | [ ] |

---

## 11. Tags

### 11a. Automated

| Test | Pass criterion | Evidence field |
|------|----------------|----------------|
| `view_layout_contract::document_page_with_tags_has_no_literal_backslash_n` | tag list renders cleanly | [ ] |
| `view_layout_contract::tag_page_pager_has_no_literal_backslash_n` | tag pager renders cleanly | [ ] |

### 11b. Manual smoke

| Step | Env | Pass criterion | Evidence field |
|------|-----|----------------|----------------|
| Create a document with `"tags": ["qa","test"]` | compose | `tags` in response | [ ] |
| `curl http://localhost:3000/tags` | compose | `200 OK`; contains "qa" and "test" | [ ] |
| `curl http://localhost:3000/tags/qa` | compose | `200 OK`; document listed | [ ] |

---

## 12. HTTP Caching

### 12a. Automated (no DB required for most)

| Test | Pass criterion | Evidence field |
|------|----------------|----------------|
| `http_caching::cache_helper_emits_cache_headers_and_body_on_first_response` | `ETag` + `Cache-Control` on first GET | [ ] |
| `http_caching::cache_helper_returns_304_without_body_when_etag_matches` | `If-None-Match` returns `304 Not Modified` | [ ] |
| `http_caching::write_api_responses_do_not_emit_cache_headers` | no `ETag`/`Cache-Control` on POSTs | [ ] |

Run with:
```
cargo test --test http_caching
```

---

## 13. MCP Server

### 13a. Automated (DB-backed)

| Test | Pass criterion | Evidence field |
|------|----------------|----------------|
| `mcp_contract::mcp_round_trip_create_read_search_and_stale_update` | create → read → search → stale-update (409) round-trip via MCP client | [ ] |
| `scoped_tokens_slice4::scoped_token_drives_the_mcp_client_surface` | MCP authenticates with a scoped token, not the admin key | [ ] |

### 13b. Manual smoke

| Step | Env | Pass criterion | Evidence field |
|------|-----|----------------|----------------|
| `INKWELL_API_KEY=<scoped-token> inkwell mcp` (with server running) | minimal | MCP server starts over stdio without error | [ ] |
| Call `search_notes` via MCP client | minimal | returns results | [ ] |

---

## 14. Docker Compose (full stack)

### 14a. Manual smoke

| Step | Env | Pass criterion | Evidence field |
|------|-----|----------------|----------------|
| `cp .env.example .env && docker compose up --build` | compose | postgres healthy, migrations applied, seed runs, server listening on port 3000 | [ ] |
| `curl http://localhost:3000/health` | compose | `200 OK`; `{"ok":true}` | [ ] |
| `docker compose down -v && docker compose up --build` (clean slate) | compose | same as above; no leftover state | [ ] |

---

## 15. Author CLI

### 15a. Automated

| Test | Pass criterion | Evidence field |
|------|----------------|----------------|
| `author_flow::*` | CLI create/update/publish/list flow against a live API | [ ] |
| `clap_cli_contract::*` | CLI arg parsing, help text, error handling | [ ] |

### 15b. Manual smoke

| Step | Env | Pass criterion | Evidence field |
|------|-----|----------------|----------------|
| `inkwell author create --title "CLI test" --body "hello"` | compose | exits 0; note appears in `GET /documents` | [ ] |

---

## Known Gaps

The following areas lack automated coverage at v0.2.  They are manual-only
or deferred.

| Gap | Reason / Notes |
|-----|---------------|
| Feed/sitemap content tests | No `feed_contract` or `sitemap_contract` test file yet; covered only by manual smoke |
| Migration rollback / partial-failure path | Migrations are forward-only; no automated rollback test |
| Real Voyage AI embeddings | CI uses `MockEmbedder`; live embedding correctness not verified in CI |
| Real Anthropic `/ask` synthesis | CI uses `MockLlm`; live answer quality is manual |
| Webmention sending (`INKWELL_WEBMENTION_SEND=true`) | `webmention_contract` covers receiving; sending is manual |
| Browser login (`INKWELL_BROWSER_LOGIN=true`) | `browser_login` tests exist but the HTML login page is not yet built |
| Media upload UI | API covered by `media_contract`; upload UI is deferred |
| `INKWELL_TRUST_FORWARDED_HEADERS` integration | Rate-limit IP keying with real proxy headers is not tested in CI |
| Performance / load testing | No sustained load test exists |
