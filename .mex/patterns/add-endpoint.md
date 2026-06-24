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
       // Derive visibility before any DB call (token-aware as of scoped-tokens
       // slice 2). `authenticate` resolves the shared key OR a live scoped token;
       // an anonymous request returns None without a DB hit.
       let visibility = if authenticate(&headers, &state.config, &state.pool).await.is_some() {
           Visibility::All
       } else {
           Visibility::Public
       };
       // ...
   }
   ```

3. **For write endpoints**, resolve the principal first (and audit with it):
   ```rust
   let principal = require_principal(&headers, &state.config, &state.pool).await?;
   // ... after a successful mutation:
   record_audit(&state, &principal, AuditAction::Create, Some(document.id), &slug).await;
   ```
   For **admin-only** surfaces, also gate on the scope:
   ```rust
   if !principal.has(Scope::Admin) {
       return Err(AppError::Forbidden("This action requires an admin token.".into()));
   }
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
- [ ] Write endpoint calls `require_principal` before any mutation (and audits with the resolved `Principal`)
- [ ] Read endpoint derives `Visibility` from `authenticate(...).await.is_some()`
- [ ] Admin-only surface additionally checks `principal.has(Scope::Admin)`
- [ ] Response struct has `#[serde(rename_all = "camelCase")]`
- [ ] Route registered in `build_router_with_providers`
- [ ] Module declared in `src/http/mod.rs` (if new file)
- [ ] `cargo clippy --all-targets --all-features -- -D warnings` passes

## Update Scaffold
- [ ] Update `.mex/ROUTER.md` "Current Project State" if what's working/not built has changed
- [ ] Update `context/architecture.md` if the new endpoint changes the system's public surface
