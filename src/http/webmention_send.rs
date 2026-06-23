//! Webmention SEND on publish (card T11, federation P3) — OPT-IN, default OFF.
//!
//! When `INKWELL_WEBMENTION_SEND=true`, publishing a note triggers a best-effort
//! attempt to send a Webmention to every external URL the note links out to:
//! discover each target's endpoint and POST `source`/`target` through the
//! SSRF-guarded fetch helper. When the flag is OFF (the default), [`maybe_send`]
//! returns immediately and the entire send path is inert — no URL extraction, no
//! task spawned, no network touched.
//!
//! Sending is fire-and-forget: it runs in a detached task so it never blocks or
//! fails the publish, and every per-target failure is logged and dropped.

use std::collections::HashSet;

use crate::federation::webmention as wm;
use crate::http::AppState;

/// Maximum external targets a single publish will attempt to notify. A small cap
/// bounds the fan-out of one publish exactly like every other surface.
const MAX_SEND_TARGETS: usize = 50;

/// Best-effort Webmention send for a freshly published note. **Inert** unless
/// `INKWELL_WEBMENTION_SEND` is enabled: with the flag off this returns before
/// touching anything. With the flag on, it derives the note's public URL, finds
/// the external links in its body, and spawns a detached task that notifies each
/// target through the SSRF guard.
///
/// Does nothing (and logs at debug) when there is no `INKWELL_SITE_URL` to build
/// a canonical `source` URL from, since a Webmention needs an absolute source.
pub fn maybe_send(state: &AppState, slug: &str, body_markdown: &str) {
    if !state.config.webmention_send {
        // Flag off ⇒ fully inert. No extraction, no task, no network.
        return;
    }
    let Some(site_url) = state.config.site_url.as_deref() else {
        tracing::debug!("webmention send enabled but INKWELL_SITE_URL is unset; skipping");
        return;
    };
    let Some(source_url) = note_public_url(site_url, slug) else {
        tracing::debug!(%slug, "could not build a public source URL; skipping webmention send");
        return;
    };

    let targets = external_links(body_markdown, site_url);
    if targets.is_empty() {
        return;
    }

    tokio::spawn(async move {
        for target in targets.into_iter().take(MAX_SEND_TARGETS) {
            match wm::send_webmention(&source_url, &target).await {
                Ok(true) => {
                    tracing::info!(%target, "sent webmention");
                }
                Ok(false) => {
                    tracing::debug!(%target, "target advertises no webmention endpoint");
                }
                Err(error) => {
                    tracing::info!(%error, %target, "webmention send failed; dropping");
                }
            }
        }
    });
}

/// Build the canonical public URL of a note from the site URL and slug. The
/// public note route is `/{slug}`, mirroring the router.
fn note_public_url(site_url: &str, slug: &str) -> Option<String> {
    let base = reqwest::Url::parse(site_url).ok()?;
    base.join(slug).ok().map(|u| u.to_string())
}

/// Extract distinct *external* http(s) URLs the note links out to. Scans markdown
/// inline/auto links and bare URLs; internal wikilinks (`[[...]]`) carry no
/// scheme and are ignored. URLs whose origin matches `site_url` are dropped so a
/// note never sends a Webmention to itself via an absolute internal link. The
/// result is deduplicated and order-stable.
fn external_links(markdown: &str, site_url: &str) -> Vec<String> {
    let site = reqwest::Url::parse(site_url).ok();
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for url in scan_http_urls(markdown) {
        if is_same_origin(&url, site.as_ref()) {
            continue;
        }
        if seen.insert(url.clone()) {
            out.push(url);
        }
    }
    out
}

/// Whether `url` shares an origin (host + effective port) with the local site, so
/// it should be excluded from outbound send targets. A URL that fails to parse is
/// treated as not-same-origin (the SSRF guard re-validates it before any fetch).
fn is_same_origin(url: &str, site: Option<&reqwest::Url>) -> bool {
    let Some(site) = site else {
        return false;
    };
    let Ok(parsed) = reqwest::Url::parse(url) else {
        return false;
    };
    parsed.host_str() == site.host_str()
        && parsed.port_or_known_default() == site.port_or_known_default()
}

/// Find every `http://` / `https://` URL substring in `text`, trimming common
/// trailing markdown/punctuation delimiters. Deliberately simple — the SSRF guard
/// re-validates each candidate before any fetch, so a slightly greedy match is
/// harmless (a bad candidate just fails validation and is dropped).
fn scan_http_urls(text: &str) -> Vec<String> {
    let mut urls = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < text.len() {
        let rest = &text[i..];
        let is_url = rest.starts_with("http://") || rest.starts_with("https://");
        if !is_url {
            // Advance one char (UTF-8 safe).
            i += utf8_len(bytes[i]);
            continue;
        }
        // Take until whitespace or a delimiter that cannot be part of a URL.
        let end = rest
            .find(|c: char| c.is_whitespace() || matches!(c, '"' | '<' | '>' | '|' | '\\' | '`'))
            .unwrap_or(rest.len());
        let mut url = &rest[..end];
        // Trim trailing punctuation that is almost always markup, not URL.
        url = url.trim_end_matches([')', ']', '}', '.', ',', ';', '!', '?', '\'']);
        if url.len() > "https://".len() {
            urls.push(url.to_string());
        }
        i += end.max(1);
    }
    urls
}

/// Byte length of the UTF-8 sequence starting with `first`.
fn utf8_len(first: u8) -> usize {
    match first {
        b if b < 0x80 => 1,
        b if b >> 5 == 0b110 => 2,
        b if b >> 4 == 0b1110 => 3,
        _ => 4,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // A site URL that does not match any link in these fixtures, so the
    // same-origin filter is a no-op for the external-extraction assertions.
    const OTHER_SITE: &str = "https://blog.example.org";

    #[test]
    fn extracts_inline_and_bare_external_links() {
        let md = "See [example](https://example.com/post) and https://other.org/page too.";
        let links = external_links(md, OTHER_SITE);
        assert!(links.contains(&"https://example.com/post".to_string()));
        assert!(links.contains(&"https://other.org/page".to_string()));
    }

    #[test]
    fn dedupes_and_ignores_wikilinks() {
        let md = "[[internal-note]] https://example.com/a https://example.com/a";
        let links = external_links(md, OTHER_SITE);
        assert_eq!(links, vec!["https://example.com/a".to_string()]);
    }

    #[test]
    fn trims_trailing_markdown_punctuation() {
        let md = "(see https://example.com/x). Also: https://example.com/y!";
        let links = external_links(md, OTHER_SITE);
        assert!(links.contains(&"https://example.com/x".to_string()));
        assert!(links.contains(&"https://example.com/y".to_string()));
    }

    #[test]
    fn excludes_same_origin_internal_links() {
        let md = "internal https://blog.example.com/other and external https://example.com/x";
        let links = external_links(md, "https://blog.example.com");
        assert_eq!(links, vec!["https://example.com/x".to_string()]);
    }

    #[test]
    fn note_public_url_joins_slug() {
        assert_eq!(
            note_public_url("https://blog.example.com", "hello").as_deref(),
            Some("https://blog.example.com/hello")
        );
        assert_eq!(
            note_public_url("https://blog.example.com/", "hello").as_deref(),
            Some("https://blog.example.com/hello")
        );
    }

    #[test]
    fn no_external_links_yields_empty() {
        assert!(external_links("just [[wikilinks]] and plain text", OTHER_SITE).is_empty());
    }
}
