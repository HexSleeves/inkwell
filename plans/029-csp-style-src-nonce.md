# Plan 029: Remove style-src 'unsafe-inline' by externalizing the site stylesheet

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `plans/README.md`.
>
> **Drift check (run first)**: `git diff --stat 0819727..HEAD -- src/http/security_headers.rs src/views/layout.rs src/http/assets.rs src/http/router.rs`
> If any in-scope file changed, compare the "Current state" excerpts against the
> live code; on a mismatch treat it as a STOP condition.

## Status

- **Priority**: P2
- **Effort**: M
- **Risk**: MED
- **Depends on**: plans/028-security-headers-hardening.md (same CSP string in `security_headers.rs`)
- **Category**: security
- **Planned at**: commit `0819727`, 2026-06-26

## Why this matters

The CSP contains `style-src 'self' 'unsafe-inline'`. `'unsafe-inline'` trusts any `<style>` block on the page — a CSS-injection / attribute-oracle vector if user-controlled content ever reaches a `<style>` context. The `script-src` directive is already nonce-locked; `style-src` should be equally strict.

**Key finding from the live code**: the ONLY inline style anywhere is a single static block in the shared layout — `<style>{STYLES}</style>` at `src/views/layout.rs:416`, where `STYLES` is a compile-time `const` (`layout.rs:101`) that is byte-identical on every page. There is no per-page inline style.

The simplest correct fix is therefore **not** to thread a nonce through every page handler/view (that would touch 7–8 files because only the document view currently receives the CSP nonce). Instead, **externalize the static stylesheet**: serve `STYLES` from a same-origin route `GET /assets/site.css` and replace the inline `<style>` with a `<link rel="stylesheet">`. Then `style-src 'self'` covers it with **no** `'unsafe-inline'`, no nonce threading, and no hash to keep in sync. This mirrors how the font is already served (`src/http/assets.rs` — its module doc explicitly notes same-origin assets load "under the strict `default-src 'self'` CSP"). Bonus: the CSS becomes browser-cacheable across pages.

## Current state

**`src/views/layout.rs:101`** — the style constant:
```rust
const STYLES: &str = r#"
... (large CSS block) ...
"#;
```

**`src/views/layout.rs:414-416`** — the inline `<style>` in the `<head>` (rendered on EVERY page):
```rust
    <link rel="preload" href="/assets/fonts/nunito.woff2" as="font" type="font/woff2" crossorigin />
    <style>{}</style>{}
```
The first `{}` is filled by `STYLES`, the second `{}` by `extra_css` (the optional `INKWELL_CUSTOM_CSS_URL` `<link>`). See the `format!` args at `layout.rs:435-437` (`STYLES, extra_css, ...`).

**`src/http/assets.rs`** — the exemplar pattern to copy (font handler):
```rust
pub async fn nunito_font() -> Response {
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "font/woff2"),
            (header::CACHE_CONTROL, "public, max-age=31536000, immutable"),
        ],
        NUNITO_WOFF2,
    ).into_response()
}
```

**`src/http/router.rs:95`** — the font route (add the CSS route next to it):
```rust
.route("/assets/fonts/nunito.woff2", get(assets::nunito_font))
```

**`src/http/security_headers.rs:54`** — CSP (AFTER plan 028 lands, so `img-src` already has colons):
```rust
"default-src 'self'; object-src 'none'; base-uri 'self'; frame-ancestors 'none'; \
 img-src 'self' http: https:; style-src 'self' 'unsafe-inline'; \
 script-src 'self' 'nonce-{}'"
```

## Commands you will need

| Purpose    | Command                                              | Expected on success          |
|------------|------------------------------------------------------|------------------------------|
| Find style tags | `grep -rn '<style' src/views/`                  | exactly one hit: `layout.rs` |
| Typecheck  | `cargo check --all-targets`                          | exit 0                       |
| Tests      | `cargo nextest run --test security_headers_contract --test view_layout_contract` | all pass |
| All tests  | `cargo nextest run`                                  | all pass                     |
| Lint       | `cargo clippy --all-targets -- -D warnings`          | exit 0                       |

## Scope

**In scope**:
- `src/views/layout.rs` — make `STYLES` reachable by `assets.rs` (e.g. `pub(crate) const STYLES`), and replace the inline `<style>{}</style>` with `<link rel="stylesheet" href="/assets/site.css" />`
- `src/http/assets.rs` — add a `site_css()` handler serving `STYLES` as `text/css`
- `src/http/router.rs` — register `GET /assets/site.css`
- `src/http/security_headers.rs` — change `style-src 'self' 'unsafe-inline'` to `style-src 'self'`
- `tests/security_headers_contract.rs`, `tests/view_layout_contract.rs` — update/extend assertions

**Out of scope**:
- `script-src` nonce logic — already correct; do not touch
- The `INKWELL_CUSTOM_CSS_URL` `<link>` (`extra_css`) — it is already an external `<link>` covered by `style-src 'self'` only if same-origin; leaving the operator-supplied URL as-is is fine (a cross-origin custom CSS URL would need a separate `style-src` host allowance — note it, do not solve it here)
- Any `src/rendering/` change — Ammonia already strips user `style` attributes

## Git workflow

- Branch: `advisor/029-externalize-stylesheet`
- Commit: `fix(http): externalize site CSS to /assets/site.css and drop style-src 'unsafe-inline'` (use a valid conventional-commit type — `security(...)` is rejected by the repo's Semantic-PR check)

## Steps

### Step 1: Confirm there is exactly one inline `<style>`

Run `grep -rn '<style' src/views/`. Expect a single hit in `layout.rs` (the one at line 416). If there are more, STOP and report — this plan assumes a single static style block.

**Verify**: one hit, in `layout.rs`.

### Step 2: Expose STYLES and add the site_css handler

1. In `src/views/layout.rs`, change `const STYLES: &str` to `pub(crate) const STYLES: &str` so `assets.rs` can serve it.
2. In `src/http/assets.rs`, add a handler mirroring `nunito_font`:
```rust
/// `GET /assets/site.css` — serve the site stylesheet same-origin so the page
/// needs no inline <style> and the CSP can drop 'unsafe-inline' from style-src.
pub async fn site_css() -> Response {
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "text/css; charset=utf-8"),
            // STYLES can change between releases, so do NOT mark immutable; a
            // modest cache is enough (it is re-fetched at most hourly).
            (header::CACHE_CONTROL, "public, max-age=3600"),
        ],
        crate::views::layout::STYLES,
    ).into_response()
}
```

**Verify**: `cargo check --all-targets` → exit 0

### Step 3: Register the route

In `src/http/router.rs`, add next to the font route (line ~95):
```rust
.route("/assets/site.css", get(assets::site_css))
```

**Verify**: `cargo check --all-targets` → exit 0

### Step 4: Replace the inline `<style>` with a `<link>`

In `src/views/layout.rs`, change the head markup at line 416 from:
```rust
    <style>{}</style>{}
```
to:
```rust
    <link rel="stylesheet" href="/assets/site.css" />{}
```
and remove `STYLES` from the `format!` argument list (the `format!` at lines ~435-437) — the first `{}` is gone, so drop the `STYLES,` argument; keep `extra_css` as the remaining trailing `{}`. Re-check the `format!` placeholder count matches the argument count after the edit (a mismatch is a compile error, which is the safety net).

**Verify**: `cargo check --all-targets` → exit 0; `grep -rn '<style' src/views/` → no hits.

### Step 5: Drop 'unsafe-inline' from the CSP

In `src/http/security_headers.rs`, change `style-src 'self' 'unsafe-inline'` to `style-src 'self'` in the CSP format string. Leave everything else (including the `script-src` nonce) unchanged.

**Verify**: `cargo check --all-targets` → exit 0; `grep -n "unsafe-inline" src/http/security_headers.rs` → no hits.

### Step 6: Update tests

1. `tests/security_headers_contract.rs` — change the `style-src` CSP assertion to `csp.contains("style-src 'self'")` AND add an explicit `assert!(!csp.contains("unsafe-inline"))`. (Just asserting `contains("style-src 'self'")` is NOT enough — it still matches the old `style-src 'self' 'unsafe-inline'`; the negative assertion is what guards the regression.)
2. `tests/view_layout_contract.rs` — if any test asserts the page contains `<style>`, change it to assert the page contains `<link rel="stylesheet" href="/assets/site.css"` and does NOT contain `<style>`.
3. Route-registration test — **must exercise the REAL router**, not an ad-hoc one. Note: `tests/security_headers_contract.rs` builds an ad-hoc `Router::new().route(...)` and `tests/view_layout_contract.rs` calls render fns directly — neither registers `/assets/*`, so a path typo in `router.rs` would pass tests there. Instead add the route test to **`tests/http_caching.rs`**, which already has a private `router_with_unreachable_database()` helper that calls the real `build_router` (DB-less). The `site_css` handler needs no DB, so it returns 200 even against an unreachable DB. Assert `GET /assets/site.css` → 200, `content-type: text/css; charset=utf-8`, non-empty body. (Model it on the existing tests in that file.)

**Verify**: `cargo nextest run --test security_headers_contract --test view_layout_contract --test http_caching` → all pass (these three are non-DB / DB-less-router suites)

## Test plan

- CSP no longer contains `'unsafe-inline'` in `style-src` (contract test updated).
- `GET /assets/site.css` → 200, `text/css`, non-empty.
- Rendered pages contain `<link rel="stylesheet" href="/assets/site.css">` and no `<style>` block.

## Done criteria

- [ ] `cargo check --all-targets` exits 0
- [ ] `cargo clippy --all-targets -- -D warnings` exits 0
- [ ] `cargo fmt --check` exits 0
- [ ] `cargo nextest run` exits 0
- [ ] `grep -rn '<style' src/views/` → no matches
- [ ] `grep -n "unsafe-inline" src/http/security_headers.rs` → no matches
- [ ] CSP test asserts `!csp.contains("unsafe-inline")` (negative guard), not just `contains("style-src 'self'")`
- [ ] `GET /assets/site.css` is tested through the REAL router (in `tests/http_caching.rs` via `build_router`), returns 200 + `text/css`
- [ ] `plans/README.md` status row updated

## STOP conditions

- `grep -rn '<style' src/views/` finds more than one inline style block, or a style block that is NOT a single static const. The externalize approach assumes one static block; report and stop.
- Removing `STYLES` from the `format!` argument list leaves a placeholder/argument count mismatch you cannot resolve (re-count `{}` vs args). Report.
- A page visibly loses styling after the change (the `<link>` href is wrong or the route isn't registered). Re-check Steps 3-4.

## Maintenance notes

- `STYLES` now ships via `GET /assets/site.css`. Editing `STYLES` requires no CSP change (that was the whole point of avoiding a hash). The 1-hour cache means a style change can take up to an hour to appear for a returning visitor; bump to a content-hashed URL later if instant invalidation is needed.
- Any FUTURE inline `<style>` added to a view would be blocked by `style-src 'self'`. The rule: site styles go in `STYLES` (served via the route); never add an inline `<style>` tag. Add this to the `context/conventions.md` verify checklist.
- A cross-origin `INKWELL_CUSTOM_CSS_URL` would be blocked by `style-src 'self'`. If custom external CSS must be supported, add that host to `style-src` from config — a separate, deliberate change.
