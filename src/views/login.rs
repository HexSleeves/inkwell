//! Server-rendered browser login page (ADR 0010).
//!
//! Rendered by `GET /login`, which is registered **only when
//! `INKWELL_BROWSER_LOGIN=true`** (the same flag that gates the
//! `POST /auth/login` / `POST /auth/logout` endpoints). The page reuses the
//! shared [`render_page`] chrome so it inherits the site header, styles, and
//! head meta.
//!
//! # CSP-compatible submission
//! The login endpoint accepts a JSON body (not a form-encoded POST — the ADR's
//! CSRF defense), so the form is wired up by a tiny inline `<script>` that
//! `fetch`es `/auth/login` with `content-type: application/json`. The strict
//! policy from `security_headers` (`script-src 'self' 'nonce-…'`) blocks any
//! inline script without the per-request nonce, so the `<script>` carries
//! `nonce="{csp_nonce}"`. No external JS is loaded.

use super::layout::{HeadMeta, SiteMeta, escape_html, render_page};

/// Render the browser login page through the shared layout.
///
/// When `logged_in` is false, the body is a single-field login form (the author
/// token) plus an empty status element. When true, it is a short "signed in"
/// message and a log-out button. `csp_nonce` is threaded into both the inline
/// `<script>` (so it survives the strict CSP) and the shared head.
pub fn render_login_page(site: &SiteMeta<'_>, csp_nonce: Option<&str>, logged_in: bool) -> String {
    let body = if logged_in {
        r#"<h1>Your account</h1>
        <p>You are signed in.</p>
        <p><a class="btn" href="/editor">Open the editor</a></p>
        <button id="logout" type="button">Log out</button>
        <p id="status" role="status" aria-live="polite"></p>"#
            .to_string()
    } else {
        r#"<h1>Sign in</h1>
        <form id="login-form" class="login">
          <label for="token">Author token (ink_…)</label>
          <input
            type="password"
            id="token"
            name="token"
            autocomplete="current-password"
            autocapitalize="off"
            spellcheck="false"
            required
          />
          <button type="submit">Sign in</button>
        </form>
        <p id="status" role="alert" aria-live="polite"></p>"#
            .to_string()
    };

    let nonce_attr = csp_nonce
        .map(|value| format!(r#" nonce="{}""#, escape_html(value)))
        .unwrap_or_default();

    // Self-contained inline script: wires the login form (JSON fetch → redirect
    // on 200, show the error element otherwise) and the logout button. No values
    // are interpolated into the script body, so there is nothing to escape inside
    // it; the only interpolation is the (escaped) nonce on the tag itself.
    let script = format!(
        r#"<script{nonce}>
(function () {{
  var form = document.getElementById('login-form');
  var status = document.getElementById('status');
  var logout = document.getElementById('logout');
  if (form) {{
    form.addEventListener('submit', function (event) {{
      event.preventDefault();
      status.textContent = '';
      var token = document.getElementById('token').value;
      fetch('/auth/login', {{
        method: 'POST',
        headers: {{ 'content-type': 'application/json' }},
        body: JSON.stringify({{ token: token }})
      }})
        .then(function (response) {{
          if (response.status === 200) {{
            window.location.assign('/');
          }} else {{
            status.textContent = 'Login failed. Check your token and try again.';
          }}
        }})
        .catch(function () {{
          status.textContent = 'Login failed. Please try again.';
        }});
    }});
  }}
  if (logout) {{
    logout.addEventListener('click', function () {{
      fetch('/auth/logout', {{ method: 'POST' }})
        .then(function () {{
          window.location.assign('/login');
        }})
        .catch(function () {{
          window.location.reload();
        }});
    }});
  }}
}})();
</script>"#,
        nonce = nonce_attr,
    );

    let main = format!("{body}\n{script}");

    render_page(
        site,
        HeadMeta {
            title: &format!("Sign in — {}", site.name),
            description: None,
            canonical_url: format!("{}/login", site.base_url),
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

    #[test]
    fn logged_out_page_shows_the_token_form_and_json_target() {
        let site = SiteMeta::defaults();
        let html = render_login_page(&site, Some("abc123"), false);
        // The single token field and submit form are present.
        assert!(html.contains(r#"id="token""#));
        assert!(html.contains(r#"name="token""#));
        assert!(html.contains(r#"type="password""#));
        assert!(html.contains(r#"id="login-form""#));
        // An empty status/error element exists for the script to populate.
        assert!(html.contains(r#"id="status""#));
        // The script targets the JSON login endpoint.
        assert!(html.contains("/auth/login"));
        // The inline script carries the CSP nonce so it is not blocked.
        assert!(html.contains(r#"<script nonce="abc123">"#));
    }

    #[test]
    fn logged_in_page_shows_logout_not_the_form() {
        let site = SiteMeta::defaults();
        let html = render_login_page(&site, Some("nonce"), true);
        assert!(html.contains("You are signed in"));
        assert!(html.contains(r#"id="logout""#));
        assert!(html.contains("/auth/logout"));
        // The login form is not rendered when already signed in.
        assert!(!html.contains(r#"id="login-form""#));
    }

    #[test]
    fn nonce_is_html_escaped_on_the_script_tag() {
        let site = SiteMeta::defaults();
        let html = render_login_page(&site, Some(r#""><x"#), false);
        // A hostile nonce must be escaped, never break out of the attribute.
        assert!(!html.contains(r#"<script nonce=""><x">"#));
        assert!(html.contains("&quot;&gt;&lt;x"));
    }

    #[test]
    fn missing_nonce_emits_a_bare_script_tag() {
        let site = SiteMeta::defaults();
        let html = render_login_page(&site, None, false);
        assert!(html.contains("<script>"));
    }
}
