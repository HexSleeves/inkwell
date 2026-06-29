# Plan 027: Browser media-upload UI (file-picker + drag-drop)

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan in
> `plans/README.md` — unless a reviewer dispatched you and told you they
> maintain the index.
>
> **Drift check (run first)**: `git diff --stat ed97b6e..HEAD -- src/http/media.rs src/http/router.rs src/http/auth_session.rs src/views/login.rs src/views/mod.rs`
> If any of those changed since this plan was written, compare the "Current
> state" excerpts against the live code before proceeding; on a mismatch, treat
> it as a STOP condition.

## Status

- **Priority**: P3
- **Effort**: M
- **Risk**: MED (new browser surface + inline JS under a strict CSP)
- **Depends on**: browser login UI (shipped — `GET /login`, session-cookie auth) and the media API (`POST /media`). Both are on `main` as of the stamp commit.
- **Category**: direction / dx (frontend)
- **Planned at**: commit `ed97b6e`, 2026-06-28

## Why this matters

The media API (`POST /media` + `GET /media/{id}`) and the browser login flow (`GET /login`, session cookie) both shipped, but there is no browser UI to upload an image — authors must use `curl --data-binary`, the `inkwell author upload` CLI, or MCP. This plan adds a small same-origin page where a logged-in author drag-drops or picks an image, it uploads via the existing API riding the **session cookie** (no key pasting), and the page hands back the `/media/{id}` URL plus a ready-to-paste `![](…)` Markdown snippet.

The hard question the old stub flagged ("auth in the browser") is already solved: `authenticate()` resolves the `inkwell_session` cookie to a `Principal` when `INKWELL_BROWSER_LOGIN=true` and no `x-api-key` header is present (`src/http/auth.rs`). So `POST /media` accepts the session cookie directly. This plan only adds a page + a tiny inline uploader script.

## Current state

**`POST /media`** (`src/http/media.rs`) — the API this UI drives, UNCHANGED by this plan:
- Auth: `require_principal` → requires the `write` scope (403 otherwise; 401 if unauthenticated).
- Body: **raw image bytes**; the `Content-Type` header names the MIME type.
- Allowlist: `image/png`, `image/jpeg`, `image/gif`, `image/webp` (SVG excluded). 400 on other types.
- Size cap: `MAX_MEDIA_BYTES = 5 * 1024 * 1024` (5 MiB); 413 if exceeded.
- Success: `201` with JSON `{ "id": "<uuid>", "url": "/media/<uuid>" }`.
- Errors use the standard envelope `{ "error": { "message": "…" } }` (`src/error.rs`).

**Session-cookie auth** (`src/http/auth.rs`): `authenticate()` honors the `inkwell_session` cookie (only when `INKWELL_BROWSER_LOGIN=true` and no `x-api-key`). A same-origin `fetch('/media', { credentials: 'same-origin' })` from the page sends the cookie. The cookie is `SameSite=Strict`, so a same-origin POST includes it and cross-site requests cannot — this is the CSRF defense; **no separate CSRF token is needed** (same posture as the login flow).

**The exemplar to copy — `src/views/login.rs`** `render_login_page(site, csp_nonce, logged_in)`: it renders through the shared `render_page` chrome and wires its form with a single **inline `<script nonce="{csp_nonce}">`** (the strict CSP is `script-src 'self' 'nonce-…'`, so inline JS must carry the per-request nonce). Mirror this structure exactly. Imports it uses:
```rust
use super::layout::{HeadMeta, SiteMeta, escape_html, render_page};
```
The nonce attribute pattern (note `escape_html` on the nonce):
```rust
let nonce_attr = csp_nonce
    .map(|value| format!(r#" nonce="{}""#, escape_html(value)))
    .unwrap_or_default();
```

**The page handler exemplar — `login_page`** (`src/http/auth_session.rs`):
```rust
pub async fn login_page(
    State(state): State<AppState>,
    Extension(csp_nonce): Extension<CspNonce>,
    headers: HeaderMap,
) -> Response {
    let site = SiteMeta::from_config(&state.config);
    let logged_in = extract_session_cookie(&headers).is_some();
    Html(render_login_page(&site, Some(csp_nonce.as_str()), logged_in)).into_response()
}
```
`CspNonce` is `crate::http::security_headers::CspNonce`; `extract_session_cookie` is `crate::http::auth_session::extract_session_cookie` (`pub(crate)`).

**Router flag-gating** (`src/http/router.rs`, the `if browser_login` block near the end):
```rust
if browser_login {
    router = router
        .route("/login", get(auth_session::login_page))
        .route("/auth/login", any(auth_session::login))
        .route("/auth/logout", any(auth_session::logout));
}
```

**`src/views/mod.rs`** currently declares: `archive, document, index, layout, login, search, tags` — add `media`.

## Commands you will need

| Purpose   | Command                                                       | Expected on success |
|-----------|--------------------------------------------------------------|---------------------|
| Fmt       | `cargo fmt --all -- --check`                                 | exit 0              |
| Lint      | `cargo clippy --all-targets --all-features --locked -- -D warnings` | exit 0       |
| Typecheck | `cargo check --all-targets`                                 | exit 0              |
| View unit tests | `cargo nextest run --lib views::media`                | all pass            |
| Flag tests (DB) | `DATABASE_URL=… cargo nextest run --test browser_login` | all pass (or skip without a DB) |

> The fresh worktree has no Postgres. `tests/browser_login.rs` is DB-backed and **skips** without `DATABASE_URL`; CI's `test-integration` job validates it on the PR. The view unit tests (`views::media`) need no DB and must pass locally.

## Scope

**In scope** (only these files):
- `src/views/media.rs` (NEW) — `render_media_upload_page(site, csp_nonce, logged_in)` + inline `#[cfg(test)]` tests.
- `src/views/mod.rs` — add `pub mod media;`.
- `src/http/media.rs` — add the `media_new_page` GET handler (page only; do NOT touch `media_upload`/`media_serve`/`upload`/`serve`).
- `src/http/router.rs` — register `GET /media/new` inside the existing `if browser_login { … }` block.
- `tests/browser_login.rs` — add flag-on (200) and flag-off (404) cases for `/media/new`.

**Out of scope** (do NOT touch):
- The media API logic (`upload`, `serve`, `media_upload`, `media_serve`, allowlist, size cap) — the UI consumes it as-is.
- The auth layer (`src/http/auth.rs`, `auth_session.rs` session logic) — already resolves the cookie.
- Image processing / resize / thumbnails, a media gallery / listing, delete UI — separate cards.
- Any change to the CSP or `security_headers.rs`.

## Git workflow

- Branch: `advisor/027-media-upload-ui`
- Commit: `feat(http): browser media-upload UI at /media/new` (valid conventional type — `feat`)
- Do NOT push or open a PR unless the operator instructed it.

## Steps

### Step 1: Add the view module `src/views/media.rs`

Create `src/views/media.rs`, mirroring `src/views/login.rs`. Signature:
```rust
use super::layout::{HeadMeta, SiteMeta, escape_html, render_page};

/// Render the media-upload page through the shared layout. When `logged_in` is
/// false, show a prompt to sign in (link to `/login`); when true, show the
/// drag-drop / file-picker uploader. The inline uploader script carries the CSP
/// nonce so it survives the strict `script-src 'self' 'nonce-…'` policy.
pub fn render_media_upload_page(site: &SiteMeta<'_>, csp_nonce: Option<&str>, logged_in: bool) -> String {
```

Body when `logged_in` is false:
```html
<h1>Upload media</h1>
<p>You must <a href="/login">sign in</a> to upload images.</p>
```

Body when `logged_in` is true (a drop-zone + file input + status + result area):
```html
<h1>Upload media</h1>
<form id="upload-form" class="upload">
  <label for="file">Choose an image (PNG, JPEG, GIF, or WebP, ≤ 5 MiB)</label>
  <input type="file" id="file" name="file" accept="image/png,image/jpeg,image/gif,image/webp" required />
  <div id="dropzone" class="dropzone">Drop an image here, or use the picker above.</div>
  <button type="submit">Upload</button>
</form>
<p id="status" role="status" aria-live="polite"></p>
<div id="result" hidden>
  <label for="url">URL</label>
  <input id="url" type="text" readonly />
  <button id="copy-url" type="button">Copy URL</button>
  <label for="markdown">Markdown</label>
  <input id="markdown" type="text" readonly />
  <button id="copy-md" type="button">Copy Markdown</button>
</div>
```

The inline script (build with the same `nonce_attr` pattern as `login.rs`). Client-side it mirrors the server's guards (allowlist + 5 MiB), then uploads the **raw file bytes** with the file's MIME as `Content-Type`, riding the session cookie:
```rust
let nonce_attr = csp_nonce
    .map(|value| format!(r#" nonce="{}""#, escape_html(value)))
    .unwrap_or_default();

let script = format!(
    r#"<script{nonce}>
(function () {{
  var form = document.getElementById('upload-form');
  if (!form) return;
  var fileInput = document.getElementById('file');
  var dropzone = document.getElementById('dropzone');
  var status = document.getElementById('status');
  var result = document.getElementById('result');
  var urlField = document.getElementById('url');
  var mdField = document.getElementById('markdown');
  var ALLOWED = ['image/png', 'image/jpeg', 'image/gif', 'image/webp'];
  var MAX_BYTES = 5 * 1024 * 1024;

  if (dropzone) {{
    ['dragover', 'dragenter'].forEach(function (e) {{
      dropzone.addEventListener(e, function (ev) {{ ev.preventDefault(); dropzone.classList.add('over'); }});
    }});
    ['dragleave', 'drop'].forEach(function (e) {{
      dropzone.addEventListener(e, function () {{ dropzone.classList.remove('over'); }});
    }});
    dropzone.addEventListener('drop', function (ev) {{
      ev.preventDefault();
      if (ev.dataTransfer && ev.dataTransfer.files && ev.dataTransfer.files.length) {{
        fileInput.files = ev.dataTransfer.files;
      }}
    }});
  }}

  function upload(file) {{
    status.textContent = '';
    result.hidden = true;
    if (!file) {{ status.textContent = 'Choose a file first.'; return; }}
    if (ALLOWED.indexOf(file.type) === -1) {{ status.textContent = 'Unsupported type. Use PNG, JPEG, GIF, or WebP.'; return; }}
    if (file.size > MAX_BYTES) {{ status.textContent = 'File too large (max 5 MiB).'; return; }}
    status.textContent = 'Uploading…';
    fetch('/media', {{
      method: 'POST',
      credentials: 'same-origin',
      headers: {{ 'content-type': file.type }},
      body: file
    }})
      .then(function (response) {{
        return response.json().then(function (data) {{ return {{ ok: response.ok, data: data }}; }});
      }})
      .then(function (r) {{
        if (r.ok && r.data && r.data.url) {{
          status.textContent = 'Uploaded.';
          urlField.value = r.data.url;
          mdField.value = '![](' + r.data.url + ')';
          result.hidden = false;
        }} else {{
          var msg = (r.data && r.data.error && r.data.error.message) ? r.data.error.message : 'Upload failed.';
          status.textContent = msg;
        }}
      }})
      .catch(function () {{ status.textContent = 'Upload failed. Please try again.'; }});
  }}

  form.addEventListener('submit', function (event) {{
    event.preventDefault();
    upload(fileInput.files && fileInput.files[0]);
  }});

  function copyFrom(id) {{
    var el = document.getElementById(id);
    if (el && navigator.clipboard) {{ navigator.clipboard.writeText(el.value); }}
  }}
  var cu = document.getElementById('copy-url');
  var cm = document.getElementById('copy-md');
  if (cu) cu.addEventListener('click', function () {{ copyFrom('url'); }});
  if (cm) cm.addEventListener('click', function () {{ copyFrom('markdown'); }});
}})();
</script>"#,
    nonce = nonce_attr,
);
```
No values are interpolated into the script body, so nothing inside needs escaping; only the (escaped) nonce is interpolated onto the tag. Compose `main` and render with `render_page` exactly like `login.rs`:
```rust
let main = format!("{body}\n{script}");
render_page(
    site,
    HeadMeta {
        title: &format!("Upload media — {}", site.name),
        description: None,
        canonical_url: format!("{}/media/new", site.base_url),
        og_type: "website",
        json_ld: None,
        csp_nonce,
    },
    &main,
)
```
Confirm `HeadMeta`'s exact field set against `src/views/login.rs` (copy it verbatim; do not invent fields).

**Verify**: `cargo check --all-targets` → exit 0

### Step 2: Declare the module

In `src/views/mod.rs` add `pub mod media;` (keep the list alphabetical: after `login`).

**Verify**: `cargo check --all-targets` → exit 0

### Step 3: Add the page handler in `src/http/media.rs`

Add (do not modify the existing upload/serve handlers). Mirror `login_page`:
```rust
use axum::extract::Extension;
use axum::response::Html;
use crate::http::security_headers::CspNonce;
use crate::http::auth_session::extract_session_cookie;
use crate::views::layout::SiteMeta;
use crate::views::media::render_media_upload_page;

/// `GET /media/new` — the browser upload page. Flag-gated in the router (only
/// registered when `INKWELL_BROWSER_LOGIN=true`). The page just chooses which UI
/// to show from cookie presence; the actual upload is auth-enforced by `POST /media`.
pub async fn media_new_page(
    State(state): State<AppState>,
    Extension(csp_nonce): Extension<CspNonce>,
    headers: HeaderMap,
) -> Response {
    let site = SiteMeta::from_config(&state.config);
    let logged_in = extract_session_cookie(&headers).is_some();
    Html(render_media_upload_page(&site, Some(csp_nonce.as_str()), logged_in)).into_response()
}
```
Adjust imports to avoid duplicates (the file already imports `State`, `HeaderMap`, `Response`, `IntoResponse`). If an import is already present, don't re-add it.

**Verify**: `cargo check --all-targets` → exit 0

### Step 4: Register the route (flag-gated)

In `src/http/router.rs`, inside the existing `if browser_login { … }` block, add the page route alongside `/login`:
```rust
        .route("/media/new", get(media::media_new_page))
```
`/media/new` is a static segment, so axum's router matches it ahead of the dynamic `/media/{id}` serve route — no collision. Keep it inside the flag block so the upload UI only exists when browser sessions are enabled (the only time cookie auth works).

**Verify**: `cargo check --all-targets` → exit 0

### Step 5: Tests

1. **View unit tests** in `src/views/media.rs` `#[cfg(test)]` (model on `login.rs` tests):
   - logged-out page contains `href="/login"` and does NOT contain `id="upload-form"`.
   - logged-in page contains `id="upload-form"`, `id="file"`, `accept="image/png,image/jpeg,image/gif,image/webp"`, targets `/media` in the script, and the inline script carries the nonce: `html.contains(r#"<script nonce="abc123">"#)`.
   - a hostile nonce (`"><x`) is HTML-escaped on the tag (assert it does not break out; assert `&quot;&gt;&lt;x` appears).
   - `None` nonce emits a bare `<script>`.
2. **Router flag tests** in `tests/browser_login.rs` (use the existing `browser_login_router(pool)` helper for flag-on, and the flag-off router for 404):
   - flag ON: `GET /media/new` → `StatusCode::OK`, body contains `Upload media`.
   - flag OFF: `GET /media/new` → `StatusCode::BAD_REQUEST` (400). When the page route is NOT registered, `/media/new` is matched by the dynamic `/media/{id}` serve route, whose `Path<Uuid>` extractor rejects `"new"` with 400 — NOT a 404. (Verified against the live router; `/media` + `/media/{id}` are the only `/media*` routes when the flag is off.) Assert 400 and add a one-line comment explaining the fall-through. This is acceptable: the feature is simply absent.

**Verify**: `cargo nextest run --lib views::media` → all pass; (with DB) `cargo nextest run --test browser_login` → all pass.

### Step 6: Full gate sweep

**Verify**: `cargo fmt --all -- --check` → exit 0; `cargo clippy --all-targets --all-features --locked -- -D warnings` → exit 0; `cargo check --all-targets` → exit 0.

## Test plan

- `src/views/media.rs` unit tests (no DB): logged-out vs logged-in markup, script target `/media`, nonce present + escaped, bare-tag fallback. Pattern source: the `#[cfg(test)]` block in `src/views/login.rs`.
- `tests/browser_login.rs`: `/media/new` is 200 with the flag on and **400 with it off** (the unregistered path falls through to `/media/{id}`'s Uuid extractor) — proves the flag-gating, mirroring the existing `/login` / `/auth/*` flag tests in that file.
- Manual acceptance (operator, post-merge, flag on): sign in at `/login`, open `/media/new`, drop a ≤5 MiB PNG → see the `/media/{id}` URL + `![](…)` snippet + working copy buttons; an oversized or wrong-type file shows the client-side guard message; an unauthenticated POST (no/expired cookie) surfaces the server 401/403 message.

## Done criteria

Machine-checkable; ALL must hold:
- [ ] `cargo fmt --all -- --check` exits 0
- [ ] `cargo clippy --all-targets --all-features --locked -- -D warnings` exits 0
- [ ] `cargo check --all-targets` exits 0
- [ ] `cargo nextest run --lib views::media` passes (≥4 new view tests)
- [ ] `grep -q "pub mod media;" src/views/mod.rs`
- [ ] `grep -q "/media/new" src/http/router.rs` AND it is inside the `if browser_login` block
- [ ] `git diff --name-only ed97b6e..HEAD` lists ONLY: `src/views/media.rs`, `src/views/mod.rs`, `src/http/media.rs`, `src/http/router.rs`, `tests/browser_login.rs`
- [ ] The media API functions (`upload`, `serve`, `media_upload`, `media_serve`) are unchanged (`git diff ed97b6e..HEAD -- src/http/media.rs` shows only the added `media_new_page` + its imports)
- [ ] `plans/README.md` status row updated

## STOP conditions

Stop and report (do not improvise) if:
- The CSP in `src/http/security_headers.rs` is NOT `script-src 'self' 'nonce-…'` (the inline-script approach assumes it). If `script-src` lacks the nonce, the upload script will be blocked — report instead of weakening the CSP.
- `POST /media` no longer accepts the session cookie (e.g. `authenticate` changed so cookies aren't resolved for it) — the UI would 401 every upload. Verify `src/http/auth.rs` still resolves `inkwell_session`; if not, STOP.
- `render_page` / `HeadMeta` / `SiteMeta` signatures differ from what `src/views/login.rs` uses (drift) — match the live `login.rs`, and if it has materially changed, STOP and report.
- Adding `/media/new` makes axum reject the router (route conflict with `/media/{id}`) — report the exact error; do not rename the serve route.

## Maintenance notes

- The page is intentionally gated behind `INKWELL_BROWSER_LOGIN`: cookie auth (the only browser credential) only exists when that flag is on. If the flag graduates to default-on (see `docs/spikes/001-browser-login-ui.md` / ADR 0010), this page graduates with it automatically.
- If the media allowlist or `MAX_MEDIA_BYTES` ever change in `src/http/media.rs`, update the client-side `ALLOWED` / `MAX_BYTES` mirror in `src/views/media.rs` to match (they are a UX convenience; the server remains the source of truth and still rejects mismatches).
- Future follow-ups deliberately deferred: a media gallery/list page, delete UI, and image resizing/thumbnails. Each is its own card.
- A reviewer should scrutinize: the inline script carries the nonce (not blocked by CSP), the route is inside the flag block, and no media-API logic changed.
