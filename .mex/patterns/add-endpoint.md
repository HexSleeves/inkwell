---
name: add-endpoint
description: Add a new HTTP route and handler to the inkwell API or page surface.
triggers:
  - "add endpoint"
  - "add route"
  - "new handler"
  - "new API"
  - "add page"
edges:
  - target: context/conventions.md
    condition: always — check naming, error handling, and visibility patterns
  - target: context/architecture.md
    condition: when the endpoint needs to understand how garden/db/ai layers connect
  - target: patterns/database-migration.md
    condition: when the endpoint requires a new column or table
last_updated: 2026-06-23
---

# Add Endpoint

## Context

All routes live in `src/http/router.rs`. Handlers live in `src/http/<surface>.rs`. The router uses `.route("/path", any(handler))` for multi-method endpoints and `.route("/path", get(handler))` for GET-only. `AppState` is injected via `State<AppState>` extractor.

## Steps

1. **Add the route** in `src/http/router.rs` → `build_router_with_providers`:
   ```rust
   .route("/documents/{slug}/my-feature", any(my_module::my_handler))
   ```

2. **Create or extend the handler file** in `src/http/my_module.rs` (or add to an existing file):
   ```rust
   pub async fn my_handler(
       State(state): State<AppState>,
       Path(slug): Path<String>,
       method: Method,
       headers: HeaderMap,
   ) -> Result<Response, AppError> {
       if method != Method::GET {
           return Err(AppError::MethodNotAllowed(vec!["GET"]));
       }
       // Derive visibility before any DB call
       let visibility = if is_authenticated(&headers, state.config.api_key.as_deref(), state.config.mcp_key.as_deref()) {
           Visibility::All
       } else {
           Visibility::Public
       };
       // ...
   }
   ```

3. **For write endpoints**, call `require_api_key` first:
   ```rust
   require_api_key(&headers, &state.config)?;
   ```

4. **Add the response type** (private to the handler file):
   ```rust
   #[derive(Serialize)]
   #[serde(rename_all = "camelCase")]
   struct MyFeatureResponse {
       slug: String,
       // ...
   }
   ```

5. **Register the module** in `src/http/mod.rs` if it's a new file:
   ```rust
   pub mod my_module;
   ```

6. **Add to router imports** in `src/http/router.rs`:
   ```rust
   use super::{ai, api, assets, feed, my_module, pages, search, security_headers, sitemap, webmention};
   ```

## Gotchas

- Use `any(handler)` not `get(handler).post(handler)` — every handler must method-dispatch internally and return `AppError::MethodNotAllowed` for unsupported methods. This is the existing pattern throughout `api.rs`.
- Every endpoint that reads documents must apply `Visibility` — never hardcode `status = 'published'` or skip the filter.
- Response types must NOT be `pub` — keep them private to the handler file; `Document` from `src/domain/` is the canonical type, re-wrap it in a local envelope.
- `AppState` is `Clone` (cheap Arc clones) — accept `State<AppState>` by value.
- Don't add business logic to handlers — orchestration belongs in `src/garden.rs`, DB queries in `src/db/`.

## Verify

- [ ] Method guard returns `AppError::MethodNotAllowed` for unsupported methods
- [ ] Write endpoint calls `require_api_key` before any mutation
- [ ] Read endpoint derives `Visibility` from `is_authenticated`
- [ ] Response struct has `#[serde(rename_all = "camelCase")]`
- [ ] Route registered in `build_router_with_providers`
- [ ] Module declared in `src/http/mod.rs` (if new file)
- [ ] `cargo clippy --all-targets --all-features -- -D warnings` passes

## Update Scaffold
- [ ] Update `.mex/ROUTER.md` "Current Project State" if what's working/not built has changed
- [ ] Update `context/architecture.md` if the new endpoint changes the system's public surface
