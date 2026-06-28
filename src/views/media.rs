//! Server-rendered browser media upload page.
//!
//! Rendered by `GET /media/new`, which is registered only when
//! `INKWELL_BROWSER_LOGIN=true`. The page reuses the shared [`render_page`]
//! chrome and carries the per-request CSP nonce on its inline uploader script.

use super::layout::{HeadMeta, SiteMeta, escape_html, render_page};

/// Render the media-upload page through the shared layout.
///
/// When `logged_in` is false, show a prompt to sign in. When true, show the
/// drag-drop / file-picker uploader. The actual upload is auth-enforced by
/// `POST /media`; this view only chooses which browser UI to render.
pub fn render_media_upload_page(
    site: &SiteMeta<'_>,
    csp_nonce: Option<&str>,
    logged_in: bool,
) -> String {
    let body = if logged_in {
        r#"<h1>Upload media</h1>
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
        </div>"#
            .to_string()
    } else {
        r#"<h1>Upload media</h1>
        <p>You must <a href="/login">sign in</a> to upload images.</p>"#
            .to_string()
    };

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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logged_out_page_prompts_for_login_without_upload_form() {
        let site = SiteMeta::defaults();
        let html = render_media_upload_page(&site, Some("abc123"), false);

        assert!(html.contains("Upload media"));
        assert!(html.contains(r#"href="/login""#));
        assert!(!html.contains(r#"id="upload-form""#));
    }

    #[test]
    fn logged_in_page_shows_upload_controls_and_script_target() {
        let site = SiteMeta::defaults();
        let html = render_media_upload_page(&site, Some("abc123"), true);

        assert!(html.contains(r#"id="upload-form""#));
        assert!(html.contains(r#"id="file""#));
        assert!(html.contains(r#"accept="image/png,image/jpeg,image/gif,image/webp""#));
        assert!(html.contains("fetch('/media'"));
        assert!(html.contains(r#"<script nonce="abc123">"#));
    }

    #[test]
    fn nonce_is_html_escaped_on_the_script_tag() {
        let site = SiteMeta::defaults();
        let html = render_media_upload_page(&site, Some(r#""><x"#), true);

        assert!(!html.contains(r#"<script nonce=""><x">"#));
        assert!(html.contains("&quot;&gt;&lt;x"));
    }

    #[test]
    fn missing_nonce_emits_a_bare_script_tag() {
        let site = SiteMeta::defaults();
        let html = render_media_upload_page(&site, None, true);

        assert!(html.contains("<script>"));
    }
}
