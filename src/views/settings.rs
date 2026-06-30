use crate::config::Config;

use super::layout::{HeadMeta, SiteMeta, escape_html, render_page};

/// Published-garden statistics shown on the Settings page. Each is optional so a
/// single failed count degrades to "—" instead of failing the whole page.
#[derive(Clone, Copy, Debug, Default)]
pub struct GardenStats {
    pub published: Option<i64>,
    pub tags: Option<i64>,
    pub links: Option<i64>,
}

/// The account panel's state, decided by the handler from `INKWELL_BROWSER_LOGIN`
/// and the request's resolved principal.
pub enum AccountPanel {
    /// Browser login is disabled (`INKWELL_BROWSER_LOGIN` off) — no panel shown.
    Disabled,
    /// Browser login is on, but the visitor is not signed in.
    Anonymous,
    /// Signed in: show the author label, granted scopes, and a logout control.
    SignedIn { label: String, scopes: Vec<String> },
}

/// Render the `/settings` "About this garden" page: read-only site identity,
/// enabled capabilities, and garden statistics, plus an optional account panel.
///
/// Everything here is derived from the operator's environment configuration
/// (read once at startup) — there is no editable state — so the page is purely
/// informational. Secrets (`api_key`, `database_url`, AI keys) are NEVER
/// rendered; capabilities are surfaced only as on/off booleans. The handler
/// returns this with `Cache-Control: no-store` because the account panel
/// reflects per-request auth state.
pub fn render_settings_page(
    site: &SiteMeta<'_>,
    config: &Config,
    stats: &GardenStats,
    version: &str,
    account: &AccountPanel,
    csp_nonce: &str,
) -> String {
    let description = site
        .description
        .unwrap_or("An open, API-first digital garden.");
    let author_row = match site.author {
        Some(author) => identity_row("Author", author),
        None => String::new(),
    };

    let identity = format!(
        r#"<section class="settings-section">
          <h2>Identity</h2>
          <dl class="settings-list">
            {title}
            {description}
            {author}
            {url}
          </dl>
        </section>"#,
        title = identity_row("Title", site.name),
        description = identity_row("Description", description),
        author = author_row,
        url = identity_row("Base URL", &site.base_url),
    );

    let capabilities = format!(
        r#"<section class="settings-section">
          <h2>Capabilities</h2>
          <ul class="capabilities">
            {ai}
            {embeddings}
            {webmention}
            {login}
            {rate}
          </ul>
        </section>"#,
        ai = capability_row(
            "AI answers (ask your garden)",
            config.anthropic_api_key.is_some()
        ),
        embeddings = capability_row(
            "Semantic search embeddings",
            config.voyage_api_key.is_some()
        ),
        webmention = capability_row("Webmention sending", config.webmention_send),
        login = capability_row("Browser login", config.browser_login),
        rate = capability_value_row(
            "Write rate limit",
            &if config.write_rate_limit == 0 {
                "disabled".to_string()
            } else {
                format!("{} req/min", config.write_rate_limit)
            },
        ),
    );

    let stats_html = format!(
        r#"<section class="settings-section">
          <h2>Garden</h2>
          <div class="settings-stats">
            {published}
            {tags}
            {links}
            {version}
          </div>
        </section>"#,
        published = stat_card("Published notes", stats.published),
        tags = stat_card("Tags", stats.tags),
        links = stat_card("Internal links", stats.links),
        version = stat_card_text("Version", version),
    );

    let account_html = render_account_panel(account, csp_nonce);

    let main = format!(
        r#"<div class="settings-page-header">
          <h1>Settings <span class="accent-dot">·</span> <span class="accent-title">About this garden</span></h1>
          <p class="settings-subtitle">Configuration is set by the operator's environment and read at startup.</p>
        </div>
        {identity}
        {capabilities}
        {stats_html}
        {account_html}"#
    );

    render_page(
        site,
        HeadMeta {
            title: &format!("Settings \u{2014} {}", site.name),
            description: Some(
                "About this garden: site configuration, capabilities, and statistics.",
            ),
            canonical_url: format!("{}/settings", site.base_url),
            og_type: "website",
            json_ld: None,
            csp_nonce: Some(csp_nonce),
            nav_current: Some("settings"),
            wide_layout: false,
        },
        &main,
    )
}

fn identity_row(label: &str, value: &str) -> String {
    format!(
        r#"<dt>{}</dt><dd>{}</dd>"#,
        escape_html(label),
        escape_html(value)
    )
}

fn capability_row(label: &str, enabled: bool) -> String {
    let (cls, text) = if enabled {
        ("cap-on", "On")
    } else {
        ("cap-off", "Off")
    };
    format!(
        r#"<li><span class="cap-label">{}</span><span class="cap-state {cls}">{text}</span></li>"#,
        escape_html(label)
    )
}

fn capability_value_row(label: &str, value: &str) -> String {
    format!(
        r#"<li><span class="cap-label">{}</span><span class="cap-state cap-value">{}</span></li>"#,
        escape_html(label),
        escape_html(value)
    )
}

fn stat_card(label: &str, value: Option<i64>) -> String {
    let value = value
        .map(|v| v.to_string())
        .unwrap_or_else(|| "\u{2014}".to_string());
    stat_card_text(label, &value)
}

fn stat_card_text(label: &str, value: &str) -> String {
    format!(
        r#"<div class="stat-card"><span class="stat-value">{}</span><span class="stat-label">{}</span></div>"#,
        escape_html(value),
        escape_html(label)
    )
}

fn render_account_panel(account: &AccountPanel, csp_nonce: &str) -> String {
    match account {
        AccountPanel::Disabled => String::new(),
        AccountPanel::Anonymous => r#"<section class="settings-section account-panel">
          <h2>Your account</h2>
          <p>You are not signed in.</p>
          <p><a class="btn" href="/login">Log in</a></p>
        </section>"#
            .to_string(),
        AccountPanel::SignedIn { label, scopes } => {
            let chips = if scopes.is_empty() {
                r#"<span class="account-noscope">no scopes</span>"#.to_string()
            } else {
                scopes
                    .iter()
                    .map(|scope| {
                        format!(r#"<span class="scope-chip">{}</span>"#, escape_html(scope))
                    })
                    .collect::<Vec<_>>()
                    .join("")
            };
            let nonce = escape_html(csp_nonce);
            format!(
                r#"<section class="settings-section account-panel">
          <h2>Your account</h2>
          <p>Signed in as <strong>{label}</strong>.</p>
          <p class="account-scopes">Scopes: {chips}</p>
          <button id="logout" type="button" class="btn">Log out</button>
          <p id="logout-status" role="status" aria-live="polite"></p>
        </section>
        <script nonce="{nonce}">
(function () {{
  var btn = document.getElementById('logout');
  var status = document.getElementById('logout-status');
  if (!btn) return;
  btn.addEventListener('click', function () {{
    btn.disabled = true;
    fetch('/auth/logout', {{ method: 'POST' }}).then(function (response) {{
      if (response.ok) {{
        window.location.assign('/settings');
        return;
      }}
      btn.disabled = false;
      if (status) status.textContent = 'Logout failed. Please try again.';
    }}).catch(function () {{
      btn.disabled = false;
      if (status) status.textContent = 'Logout failed. Please try again.';
    }});
  }});
}})();
</script>"#,
                label = escape_html(label),
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config(browser_login: bool, anthropic: bool) -> Config {
        Config {
            database_url: "postgres://user:secret-db-pass@localhost/x".to_string(),
            host: "127.0.0.1".to_string(),
            port: 3000,
            api_key: Some("super-secret-admin-key".to_string()),
            site_url: None,
            voyage_api_key: None,
            anthropic_api_key: anthropic.then(|| "sk-secret-anthropic".to_string()),
            llm_model: crate::config::DEFAULT_LLM_MODEL.to_string(),
            min_similarity: 0.0,
            webmention_send: false,
            browser_login,
            write_rate_limit: 60,
            trust_forwarded_headers: false,
            site_title: crate::config::DEFAULT_SITE_TITLE.to_string(),
            site_description: None,
            site_author: None,
            custom_css_url: None,
        }
    }

    fn stats() -> GardenStats {
        GardenStats {
            published: Some(42),
            tags: Some(13),
            links: Some(88),
        }
    }

    #[test]
    fn renders_identity_capabilities_and_stats() {
        let site = SiteMeta::defaults();
        let config = test_config(false, true);
        let html = render_settings_page(
            &site,
            &config,
            &stats(),
            "0.1.0",
            &AccountPanel::Disabled,
            "nonce123",
        );

        assert!(html.contains("About this garden"));
        assert!(html.contains("<dt>Title</dt>"));
        assert!(html.contains("AI answers (ask your garden)"));
        assert!(html.contains(r#"<span class="cap-state cap-on">On</span>"#));
        assert!(html.contains(r#"<span class="cap-state cap-off">Off</span>"#)); // browser login off
        assert!(html.contains("60 req/min"));
        assert!(html.contains("42"));
        assert!(html.contains("88"));
        assert!(html.contains("0.1.0"));
    }

    #[test]
    fn never_renders_secrets() {
        let site = SiteMeta::defaults();
        let config = test_config(false, true);
        let html = render_settings_page(
            &site,
            &config,
            &stats(),
            "0.1.0",
            &AccountPanel::Disabled,
            "nonce123",
        );

        assert!(!html.contains("super-secret-admin-key"));
        assert!(!html.contains("sk-secret-anthropic"));
        assert!(!html.contains("secret-db-pass"));
    }

    #[test]
    fn missing_stats_render_an_em_dash() {
        let site = SiteMeta::defaults();
        let config = test_config(false, false);
        let html = render_settings_page(
            &site,
            &config,
            &GardenStats::default(),
            "0.1.0",
            &AccountPanel::Disabled,
            "nonce123",
        );
        assert!(html.contains("\u{2014}"));
    }

    #[test]
    fn disabled_account_panel_is_omitted() {
        let site = SiteMeta::defaults();
        let config = test_config(false, false);
        let html = render_settings_page(
            &site,
            &config,
            &stats(),
            "0.1.0",
            &AccountPanel::Disabled,
            "nonce123",
        );
        assert!(!html.contains("account-panel"));
        assert!(!html.contains("Your account"));
    }

    #[test]
    fn anonymous_account_panel_shows_login_link() {
        let site = SiteMeta::defaults();
        let config = test_config(true, false);
        let html = render_settings_page(
            &site,
            &config,
            &stats(),
            "0.1.0",
            &AccountPanel::Anonymous,
            "nonce123",
        );
        assert!(html.contains("Your account"));
        assert!(html.contains(r#"href="/login""#));
        assert!(!html.contains(r#"id="logout""#));
    }

    #[test]
    fn signed_in_panel_shows_label_scopes_and_logout() {
        let site = SiteMeta::defaults();
        let config = test_config(true, false);
        let account = AccountPanel::SignedIn {
            label: "alice".to_string(),
            scopes: vec!["read".to_string(), "write".to_string()],
        };
        let html = render_settings_page(&site, &config, &stats(), "0.1.0", &account, "nonce123");

        assert!(html.contains("Signed in as <strong>alice</strong>"));
        assert!(html.contains(r#"<span class="scope-chip">read</span>"#));
        assert!(html.contains(r#"<span class="scope-chip">write</span>"#));
        assert!(html.contains(r#"<button id="logout""#));
        assert!(html.contains(r#"<script nonce="nonce123">"#));
        assert!(html.contains("/auth/logout"));
        // Logout redirect is gated on a successful response, not just fetch resolving.
        assert!(html.contains("response.ok"));
    }

    #[test]
    fn signed_in_label_is_html_escaped() {
        let site = SiteMeta::defaults();
        let config = test_config(true, false);
        let account = AccountPanel::SignedIn {
            label: "<script>alert(1)</script>".to_string(),
            scopes: vec![],
        };
        let html = render_settings_page(&site, &config, &stats(), "0.1.0", &account, "nonce123");
        assert!(html.contains("&lt;script&gt;alert(1)&lt;/script&gt;"));
        assert!(!html.contains("<script>alert(1)</script>"));
    }
}
