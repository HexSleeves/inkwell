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
        },
        &main,
    )
}

/// Render the "new document" page (`GET /editor/new`).
///
/// On submit the inline script `POST`s to `/documents` and, on `201`, redirects
/// to the new document's edit page (`/editor/{slug}`).
pub fn render_editor_new(site: &SiteMeta<'_>, csp_nonce: Option<&str>) -> String {
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

          <label for="body">Body (Markdown)</label>
          <textarea id="body" name="body" rows="18"></textarea>

          <div class="editor-actions">
            <button type="submit" class="btn">Create</button>
          </div>
        </form>
        <p id="status" role="alert" aria-live="polite"></p>"#;

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

    let main = format!("{body}\n{script}");
    render_page(
        site,
        HeadMeta {
            title: &format!("New document — {}", site.name),
            description: None,
            canonical_url: format!("{}/editor/new", site.base_url),
            og_type: "website",
            json_ld: None,
            csp_nonce,
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
pub fn render_editor_edit(site: &SiteMeta<'_>, csp_nonce: Option<&str>, slug: &str) -> String {
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

    let main = format!("{body}\n{script}");
    render_page(
        site,
        HeadMeta {
            title: &format!("Edit — {}", site.name),
            description: None,
            canonical_url: format!("{}/editor/{}", site.base_url, slug),
            og_type: "website",
            json_ld: None,
            csp_nonce,
        },
        &main,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let html = render_editor_new(&site, Some("n"));
        assert!(html.contains(r#"id="new-form""#));
        assert!(html.contains(r#"id="body""#));
        assert!(html.contains("POST"));
        assert!(html.contains("/documents"));
    }

    #[test]
    fn edit_page_embeds_slug_in_data_attribute_not_script() {
        let site = SiteMeta::defaults();
        let html = render_editor_edit(&site, Some("n"), "hello-world");
        assert!(html.contains(r#"data-slug="hello-world""#));
        assert!(html.contains(r#"id="preview""#));
        assert!(html.contains(r#"id="publish-btn""#));
        // Save path carries If-Match for optimistic concurrency.
        assert!(html.contains("if-match"));
    }

    #[test]
    fn edit_page_escapes_a_hostile_slug_in_the_data_attribute() {
        let site = SiteMeta::defaults();
        let html = render_editor_edit(&site, Some("n"), r#""><script>x"#);
        // The slug must never break out of the attribute into live markup.
        assert!(!html.contains(r#"data-slug=""><script>x""#));
        assert!(html.contains("&quot;&gt;&lt;script&gt;x"));
    }

    #[test]
    fn pages_render_without_a_nonce() {
        let site = SiteMeta::defaults();
        assert!(render_editor_list(&site, None).contains("<script>"));
        assert!(render_editor_new(&site, None).contains("<script>"));
        assert!(render_editor_edit(&site, None, "x").contains("<script>"));
    }
}
