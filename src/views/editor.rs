//! Server-rendered authoring web UI (CYP-42).
//!
//! A minimal, JS-driven editor layered over the existing `/documents` JSON API.
//! Like the login page (ADR 0010) these routes are registered **only when
//! `INKWELL_BROWSER_LOGIN=true`**; with the flag off they do not exist and the
//! public build carries none of this surface.
//!
//! # How it talks to the API
//! Each page is a thin HTML shell plus one nonce'd inline `<script>` that drives
//! the existing JSON endpoints with `fetch` (same-origin, so the `inkwell_session`
//! cookie is sent automatically — no token handling in the page):
//! - list:        `GET  /documents?status=all`
//! - create:      `POST /documents`
//! - load/edit:   `GET  /documents/{slug}` then `PATCH /documents/{slug}`
//! - publish:     `POST /documents/{slug}/publish` / `…/unpublish`
//!
//! The strict CSP from `security_headers` (`script-src 'self' 'nonce-…'`) blocks
//! any inline script without the per-request nonce, so every `<script>` carries
//! `nonce="{csp_nonce}"`. No external JS is loaded. The slug for the edit page is
//! passed through a `data-slug` attribute (HTML-escaped), never interpolated into
//! the script body, so there is no script-injection surface.
//!
//! # Preview
//! The preview pane is populated from the API's `renderedHtml` field — the exact
//! HTML the public page path renders — so a saved draft previews identically to
//! how it will appear once published. Preview refreshes on load and after each
//! save (it reflects the last *saved* body, hence "live-ish").

use super::layout::{HeadMeta, SiteMeta, escape_html, render_page};

/// Shared nonce attribute helper: `" nonce=\"…\""` or empty when absent.
fn nonce_attr(csp_nonce: Option<&str>) -> String {
    csp_nonce
        .map(|value| format!(r#" nonce="{}""#, escape_html(value)))
        .unwrap_or_default()
}

/// Placeholder the page templates carry where [`media_insert_markup`] is spliced
/// in. Keeps the surrounding HTML a plain literal (no `format!` escaping of the
/// form markup) while the widget stays defined in exactly one place.
const MEDIA_INSERT_SLOT: &str = "MEDIA_INSERT_SLOT";

/// Image-insert controls shared by the new and edit pages (CYP-45).
///
/// Markup only; [`media_insert_script`] drives it. Sits directly above the body
/// textarea so the picker, the drop target, and the text it edits are together.
fn media_insert_markup() -> &'static str {
    r#"<div class="media-insert">
              <label for="media-file">Insert image <span class="hint">(pick, drag onto the body, or paste)</span></label>
              <input type="file" id="media-file" accept="image/png,image/jpeg,image/gif,image/webp" />
              <p id="media-status" class="hint" role="status" aria-live="polite"></p>
            </div>"#
}

/// Uploader for the editor pages: sends the image to `POST /media` and inserts a
/// markdown reference at the caret in `#body`.
///
/// Three entry points, all landing in the same upload path: the file picker, a
/// drop onto the textarea, and a paste of image data. `max_bytes` mirrors the
/// server's `INKWELL_MEDIA_MAX_BYTES` so an oversized file is reported before it
/// is uploaded — the server still enforces the real cap (413).
fn media_insert_script(csp_nonce: Option<&str>, max_bytes: usize) -> String {
    format!(
        r#"<script{nonce}>
(function () {{
  var input = document.getElementById('media-file');
  var body = document.getElementById('body');
  var status = document.getElementById('media-status');
  if (!input || !body) return;
  var ALLOWED = ['image/png', 'image/jpeg', 'image/gif', 'image/webp'];
  var MAX_BYTES = {max_bytes};

  // Markdown link text: strip the extension and the characters that would break
  // out of the `![alt](url)` construct.
  function altFor(file) {{
    var name = (file.name || 'image').replace(/\.[^.]+$/, '');
    return name.replace(/[\[\]()!\r\n]/g, ' ').trim() || 'image';
  }}

  function insertAtCursor(text) {{
    var start = body.selectionStart;
    var end = body.selectionEnd;
    if (start == null || end == null) {{ start = end = body.value.length; }}
    body.value = body.value.slice(0, start) + text + body.value.slice(end);
    var caret = start + text.length;
    body.selectionStart = caret;
    body.selectionEnd = caret;
    body.focus();
  }}

  function upload(file) {{
    if (!file) return;
    if (ALLOWED.indexOf(file.type) === -1) {{
      status.textContent = 'Unsupported type. Use PNG, JPEG, GIF, or WebP.';
      return;
    }}
    if (file.size > MAX_BYTES) {{
      status.textContent = 'Image too large (max ' + Math.floor(MAX_BYTES / 1048576) + ' MiB).';
      return;
    }}
    status.textContent = 'Uploading ' + (file.name || 'image') + '…';
    fetch('/media', {{
      method: 'POST',
      credentials: 'same-origin',
      headers: {{ 'content-type': file.type }},
      body: file
    }})
      .then(function (r) {{
        if (r.status === 401) {{ window.location.assign('/login'); return null; }}
        return r.json().then(function (data) {{ return {{ ok: r.ok, data: data }}; }});
      }})
      .then(function (result) {{
        if (!result) return;
        if (result.ok && result.data && result.data.url) {{
          insertAtCursor('![' + altFor(file) + '](' + result.data.url + ')');
          status.textContent = 'Image inserted. Save to keep it.';
          return;
        }}
        var message = result.data && result.data.error && result.data.error.message;
        status.textContent = message || 'Upload failed.';
      }})
      .catch(function () {{ status.textContent = 'Upload failed. Please try again.'; }});
  }}

  input.addEventListener('change', function () {{
    upload(input.files && input.files[0]);
    // Clear so picking the same file twice fires `change` again.
    input.value = '';
  }});

  ['dragover', 'dragenter'].forEach(function (name) {{
    body.addEventListener(name, function (event) {{
      if (event.dataTransfer) {{ event.preventDefault(); body.classList.add('drop-target'); }}
    }});
  }});
  ['dragleave', 'drop'].forEach(function (name) {{
    body.addEventListener(name, function () {{ body.classList.remove('drop-target'); }});
  }});
  body.addEventListener('drop', function (event) {{
    var files = event.dataTransfer && event.dataTransfer.files;
    if (files && files.length) {{ event.preventDefault(); upload(files[0]); }}
  }});

  body.addEventListener('paste', function (event) {{
    var items = event.clipboardData && event.clipboardData.items;
    if (!items) return;
    for (var i = 0; i < items.length; i++) {{
      if (items[i].kind === 'file' && ALLOWED.indexOf(items[i].type) !== -1) {{
        var file = items[i].getAsFile();
        if (file) {{ event.preventDefault(); upload(file); }}
        return;
      }}
    }}
  }});
}})();
</script>"#,
        nonce = nonce_attr(csp_nonce),
        max_bytes = max_bytes,
    )
}

/// Render the document list page (`GET /editor`).
///
/// The table is filled in by the inline script from `GET /documents?status=all`;
/// the server ships only the chrome and an empty `<tbody>` plus a status region.
pub fn render_editor_list(site: &SiteMeta<'_>, csp_nonce: Option<&str>) -> String {
    let body = r#"<h1>Your documents</h1>
        <p class="editor-actions">
          <a class="btn" href="/editor/new">New document</a>
          <button id="logout" type="button" class="btn btn-secondary">Log out</button>
        </p>
        <p id="status" role="status" aria-live="polite"></p>
        <table class="doc-list">
          <thead>
            <tr><th>Title</th><th>Status</th><th>Updated</th><th>Actions</th></tr>
          </thead>
          <tbody id="doc-rows"></tbody>
        </table>
        <p id="empty" class="empty" hidden>No documents yet. Create your first one.</p>"#;

    let script = format!(
        r#"<script{nonce}>
(function () {{
  var rows = document.getElementById('doc-rows');
  var empty = document.getElementById('empty');
  var status = document.getElementById('status');
  var logout = document.getElementById('logout');

  function esc(s) {{
    var d = document.createElement('div');
    d.textContent = s == null ? '' : String(s);
    return d.innerHTML;
  }}

  fetch('/documents?status=all&limit=100', {{ headers: {{ accept: 'application/json' }} }})
    .then(function (r) {{
      if (r.status === 401) {{ window.location.assign('/login'); return null; }}
      if (!r.ok) throw new Error('list failed');
      return r.json();
    }})
    .then(function (data) {{
      if (!data) return;
      var docs = data.documents || [];
      if (docs.length === 0) {{ empty.hidden = false; return; }}
      docs.forEach(function (doc) {{
        var tr = document.createElement('tr');
        var updated = (doc.updatedAt || '').slice(0, 10);
        tr.innerHTML =
          '<td>' + esc(doc.title) + '</td>' +
          '<td><span class="badge badge-' + esc(doc.status) + '">' + esc(doc.status) + '</span></td>' +
          '<td>' + esc(updated) + '</td>' +
          '<td class="row-actions">' +
            '<a href="/editor/' + encodeURIComponent(doc.slug) + '">Edit</a>' +
            (doc.status === 'published'
              ? ' · <a href="/' + encodeURIComponent(doc.slug) + '">View</a>'
              : '') +
          '</td>';
        rows.appendChild(tr);
      }});
    }})
    .catch(function () {{ status.textContent = 'Could not load documents. Please retry.'; }});

  if (logout) {{
    logout.addEventListener('click', function () {{
      fetch('/auth/logout', {{ method: 'POST' }})
        .then(function () {{ window.location.assign('/login'); }})
        .catch(function () {{ window.location.reload(); }});
    }});
  }}
}})();
</script>"#,
        nonce = nonce_attr(csp_nonce),
    );

    let main = format!("{body}\n{script}");
    render_page(
        site,
        HeadMeta {
            title: &format!("Your documents — {}", site.name),
            description: None,
            canonical_url: format!("{}/editor", site.base_url),
            og_type: "website",
            json_ld: None,
            csp_nonce,
            nav_current: None,
            wide_layout: false,
        },
        &main,
    )
}

/// Render the "new document" page (`GET /editor/new`).
///
/// On submit the inline script `POST`s to `/documents` and, on `201`, redirects
/// to the new document's edit page (`/editor/{slug}`).
pub fn render_editor_new(
    site: &SiteMeta<'_>,
    csp_nonce: Option<&str>,
    media_max_bytes: usize,
) -> String {
    let body = r#"<h1>New document</h1>
        <p class="editor-actions"><a href="/editor">&larr; Back to documents</a></p>
        <form id="new-form" class="editor-form">
          <label for="title">Title</label>
          <input type="text" id="title" name="title" required maxlength="200" />

          <label for="slug">Slug <span class="hint">(optional — derived from the title)</span></label>
          <input type="text" id="slug" name="slug" autocapitalize="off" spellcheck="false"
                 pattern="[a-z0-9]+(?:-[a-z0-9]+)*" />

          <label for="tags">Tags <span class="hint">(comma-separated)</span></label>
          <input type="text" id="tags" name="tags" autocapitalize="off" />

          <label for="growth">Growth</label>
          <select id="growth" name="growth">
            <option value="seedling" selected>seedling</option>
            <option value="budding">budding</option>
            <option value="evergreen">evergreen</option>
          </select>

          MEDIA_INSERT_SLOT
          <label for="body">Body (Markdown)</label>
          <textarea id="body" name="body" rows="18"></textarea>

          <div class="editor-actions">
            <button type="submit" class="btn">Create</button>
          </div>
        </form>
        <p id="status" role="alert" aria-live="polite"></p>"#
        .replace(MEDIA_INSERT_SLOT, media_insert_markup());

    let script = format!(
        r#"<script{nonce}>
(function () {{
  var form = document.getElementById('new-form');
  var status = document.getElementById('status');
  if (!form) return;

  function parseTags(raw) {{
    return raw.split(',').map(function (t) {{ return t.trim(); }}).filter(Boolean);
  }}

  form.addEventListener('submit', function (event) {{
    event.preventDefault();
    status.textContent = '';
    var payload = {{
      title: document.getElementById('title').value,
      bodyMarkdown: document.getElementById('body').value,
      tags: parseTags(document.getElementById('tags').value),
      growth: document.getElementById('growth').value
    }};
    var slug = document.getElementById('slug').value.trim();
    if (slug) payload.slug = slug;

    fetch('/documents', {{
      method: 'POST',
      headers: {{ 'content-type': 'application/json' }},
      body: JSON.stringify(payload)
    }})
      .then(function (r) {{
        if (r.status === 401) {{ window.location.assign('/login'); return null; }}
        if (r.status === 201) return r.json();
        return r.json().then(function (e) {{ throw new Error(e && e.error ? e.error : 'Create failed.'); }});
      }})
      .then(function (doc) {{
        if (doc) window.location.assign('/editor/' + encodeURIComponent(doc.slug));
      }})
      .catch(function (err) {{ status.textContent = err.message || 'Create failed.'; }});
  }});
}})();
</script>"#,
        nonce = nonce_attr(csp_nonce),
    );

    let media_script = media_insert_script(csp_nonce, media_max_bytes);
    let main = format!("{body}\n{script}\n{media_script}");
    render_page(
        site,
        HeadMeta {
            title: &format!("New document — {}", site.name),
            description: None,
            canonical_url: format!("{}/editor/new", site.base_url),
            og_type: "website",
            json_ld: None,
            csp_nonce,
            nav_current: None,
            wide_layout: false,
        },
        &main,
    )
}

/// Render the edit page (`GET /editor/{slug}`).
///
/// The slug is passed through a `data-slug` attribute (HTML-escaped) and read by
/// the script via `dataset` — never interpolated into the script body. The page
/// loads the document with `GET /documents/{slug}`, saves edits with `PATCH`
/// (carrying `If-Match` for optimistic concurrency), and toggles publication via
/// the publish/unpublish endpoints. The preview pane shows the API's
/// `renderedHtml` (the public render).
pub fn render_editor_edit(
    site: &SiteMeta<'_>,
    csp_nonce: Option<&str>,
    slug: &str,
    media_max_bytes: usize,
) -> String {
    let body = format!(
        r##"<h1>Edit document</h1>
        <p class="editor-actions"><a href="/editor">&larr; Back to documents</a></p>
        <div id="editor" data-slug="{slug}" class="editor-grid">
          <form id="edit-form" class="editor-form">
            <div class="editor-statusline">
              Status: <span id="doc-status" class="badge">…</span>
              <a id="view-link" href="#" hidden>View public page</a>
            </div>

            <label for="title">Title</label>
            <input type="text" id="title" name="title" required maxlength="200" />

            <label for="slug">Slug</label>
            <input type="text" id="slug" name="slug" autocapitalize="off" spellcheck="false"
                   pattern="[a-z0-9]+(?:-[a-z0-9]+)*" />

            <label for="tags">Tags <span class="hint">(comma-separated)</span></label>
            <input type="text" id="tags" name="tags" autocapitalize="off" />

            <label for="growth">Growth</label>
            <select id="growth" name="growth">
              <option value="seedling">seedling</option>
              <option value="budding">budding</option>
              <option value="evergreen">evergreen</option>
            </select>

            {media_insert}
            <label for="body">Body (Markdown)</label>
            <textarea id="body" name="body" rows="20"></textarea>

            <div class="editor-actions">
              <button type="submit" class="btn">Save draft</button>
              <button type="button" id="publish-btn" class="btn btn-publish">Publish</button>
            </div>
          </form>
          <section class="preview" aria-label="Preview">
            <h2>Preview</h2>
            <div id="preview" class="preview-body"></div>
          </section>
        </div>
        <p id="status" role="alert" aria-live="polite"></p>"##,
        slug = escape_html(slug),
        media_insert = media_insert_markup(),
    );

    let script = format!(
        r#"<script{nonce}>
(function () {{
  var editor = document.getElementById('editor');
  var slug = editor ? editor.dataset.slug : '';
  var form = document.getElementById('edit-form');
  var status = document.getElementById('status');
  var docStatus = document.getElementById('doc-status');
  var viewLink = document.getElementById('view-link');
  var preview = document.getElementById('preview');
  var publishBtn = document.getElementById('publish-btn');
  var version = null;

  function parseTags(raw) {{
    return raw.split(',').map(function (t) {{ return t.trim(); }}).filter(Boolean);
  }}

  function apply(doc) {{
    version = doc.version;
    document.getElementById('title').value = doc.title || '';
    document.getElementById('slug').value = doc.slug || '';
    document.getElementById('tags').value = (doc.tags || []).join(', ');
    document.getElementById('growth').value = doc.growth || 'seedling';
    document.getElementById('body').value = doc.bodyMarkdown || '';
    preview.innerHTML = doc.renderedHtml || '';
    docStatus.textContent = doc.status;
    docStatus.className = 'badge badge-' + doc.status;
    if (doc.status === 'published') {{
      viewLink.hidden = false;
      viewLink.setAttribute('href', '/' + encodeURIComponent(doc.slug));
      publishBtn.textContent = 'Unpublish';
    }} else {{
      viewLink.hidden = true;
      publishBtn.textContent = 'Publish';
    }}
    // The slug may have changed on save (rename); keep the editor bound to it.
    if (doc.slug) slug = doc.slug;
  }}

  function load() {{
    fetch('/documents/' + encodeURIComponent(slug), {{ headers: {{ accept: 'application/json' }} }})
      .then(function (r) {{
        if (r.status === 401) {{ window.location.assign('/login'); return null; }}
        if (r.status === 404) {{ throw new Error('Document not found.'); }}
        if (!r.ok) throw new Error('Could not load the document.');
        return r.json();
      }})
      .then(function (doc) {{ if (doc) apply(doc); }})
      .catch(function (err) {{ status.textContent = err.message || 'Load failed.'; }});
  }}

  form.addEventListener('submit', function (event) {{
    event.preventDefault();
    status.textContent = '';
    var payload = {{
      title: document.getElementById('title').value,
      bodyMarkdown: document.getElementById('body').value,
      tags: parseTags(document.getElementById('tags').value),
      growth: document.getElementById('growth').value
    }};
    var newSlug = document.getElementById('slug').value.trim();
    if (newSlug && newSlug !== slug) payload.slug = newSlug;

    var headers = {{ 'content-type': 'application/json' }};
    if (version != null) headers['if-match'] = String(version);

    fetch('/documents/' + encodeURIComponent(slug), {{
      method: 'PATCH', headers: headers, body: JSON.stringify(payload)
    }})
      .then(function (r) {{
        if (r.status === 401) {{ window.location.assign('/login'); return null; }}
        if (r.status === 409) {{ throw new Error('This document changed elsewhere. Reload before saving.'); }}
        if (!r.ok) {{ return r.json().then(function (e) {{ throw new Error(e && e.error ? e.error : 'Save failed.'); }}); }}
        return r.json();
      }})
      .then(function (doc) {{
        if (!doc) return;
        apply(doc);
        status.textContent = 'Saved.';
        // If the slug changed, reflect it in the URL without a reload.
        if (window.history && window.history.replaceState) {{
          window.history.replaceState(null, '', '/editor/' + encodeURIComponent(doc.slug));
        }}
      }})
      .catch(function (err) {{ status.textContent = err.message || 'Save failed.'; }});
  }});

  publishBtn.addEventListener('click', function () {{
    status.textContent = '';
    var publishing = publishBtn.textContent === 'Publish';
    var action = publishing ? 'publish' : 'unpublish';
    fetch('/documents/' + encodeURIComponent(slug) + '/' + action, {{ method: 'POST' }})
      .then(function (r) {{
        if (r.status === 401) {{ window.location.assign('/login'); return null; }}
        if (r.status === 403) {{ throw new Error('Your session lacks the "publish" scope.'); }}
        if (!r.ok) throw new Error((publishing ? 'Publish' : 'Unpublish') + ' failed.');
        return r.json();
      }})
      .then(function (doc) {{
        if (!doc) return;
        apply(doc);
        status.textContent = publishing ? 'Published.' : 'Unpublished.';
      }})
      .catch(function (err) {{ status.textContent = err.message; }});
  }});

  load();
}})();
</script>"#,
        nonce = nonce_attr(csp_nonce),
    );

    let media_script = media_insert_script(csp_nonce, media_max_bytes);
    let main = format!("{body}\n{script}\n{media_script}");
    render_page(
        site,
        HeadMeta {
            title: &format!("Edit — {}", site.name),
            description: None,
            canonical_url: format!("{}/editor/{}", site.base_url, slug),
            og_type: "website",
            json_ld: None,
            csp_nonce,
            nav_current: None,
            wide_layout: false,
        },
        &main,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Stand-in for the configured `INKWELL_MEDIA_MAX_BYTES` in view tests.
    const MAX_BYTES: usize = 5 * 1024 * 1024;

    #[test]
    fn list_page_renders_table_and_fetches_documents() {
        let site = SiteMeta::defaults();
        let html = render_editor_list(&site, Some("abc123"));
        assert!(html.contains(r#"class="doc-list""#));
        assert!(html.contains(r#"id="doc-rows""#));
        // Pulls the full status set so drafts and published both show.
        assert!(html.contains("/documents?status=all"));
        assert!(html.contains("/editor/new"));
        // Inline script carries the nonce so the strict CSP allows it.
        assert!(html.contains(r#"<script nonce="abc123">"#));
    }

    #[test]
    fn new_page_posts_to_documents() {
        let site = SiteMeta::defaults();
        let html = render_editor_new(&site, Some("n"), MAX_BYTES);
        assert!(html.contains(r#"id="new-form""#));
        assert!(html.contains(r#"id="body""#));
        assert!(html.contains("POST"));
        assert!(html.contains("/documents"));
    }

    #[test]
    fn edit_page_embeds_slug_in_data_attribute_not_script() {
        let site = SiteMeta::defaults();
        let html = render_editor_edit(&site, Some("n"), "hello-world", MAX_BYTES);
        assert!(html.contains(r#"data-slug="hello-world""#));
        assert!(html.contains(r#"id="preview""#));
        assert!(html.contains(r#"id="publish-btn""#));
        // Save path carries If-Match for optimistic concurrency.
        assert!(html.contains("if-match"));
    }

    #[test]
    fn edit_page_escapes_a_hostile_slug_in_the_data_attribute() {
        let site = SiteMeta::defaults();
        let html = render_editor_edit(&site, Some("n"), r#""><script>x"#, MAX_BYTES);
        // The slug must never break out of the attribute into live markup.
        assert!(!html.contains(r#"data-slug=""><script>x""#));
        assert!(html.contains("&quot;&gt;&lt;script&gt;x"));
    }

    #[test]
    fn pages_render_without_a_nonce() {
        let site = SiteMeta::defaults();
        assert!(render_editor_list(&site, None).contains("<script>"));
        assert!(render_editor_new(&site, None, MAX_BYTES).contains("<script>"));
        assert!(render_editor_edit(&site, None, "x", MAX_BYTES).contains("<script>"));
    }

    #[test]
    fn authoring_pages_carry_the_image_uploader_wired_to_the_media_api() {
        let site = SiteMeta::defaults();
        for html in [
            render_editor_new(&site, Some("n"), MAX_BYTES),
            render_editor_edit(&site, Some("n"), "hello", MAX_BYTES),
        ] {
            // The picker, the drop target, and the paste path all exist…
            assert!(html.contains(r#"id="media-file""#));
            assert!(html.contains("accept=\"image/png,image/jpeg,image/gif,image/webp\""));
            assert!(html.contains("addEventListener('drop'"));
            assert!(html.contains("addEventListener('paste'"));
            // …they upload to the media API…
            assert!(html.contains("fetch('/media'"));
            // …and the result is inserted at the caret as a markdown image.
            assert!(html.contains("insertAtCursor('![' + altFor(file) + '](' + result.data.url"));
            assert!(html.contains("body.selectionStart"));
            // The slot placeholder must never survive into the rendered page.
            assert!(!html.contains(MEDIA_INSERT_SLOT));
        }
    }

    #[test]
    fn uploader_mirrors_the_configured_size_cap() {
        let site = SiteMeta::defaults();
        let html = render_editor_new(&site, Some("n"), 1_234_567);
        assert!(html.contains("var MAX_BYTES = 1234567;"));
    }

    #[test]
    fn uploader_script_carries_the_csp_nonce() {
        let site = SiteMeta::defaults();
        let html = render_editor_edit(&site, Some("abc123"), "hello", MAX_BYTES);
        // Two nonce'd scripts: the page's own and the uploader's.
        assert_eq!(html.matches(r#"<script nonce="abc123">"#).count(), 2);
    }
}
