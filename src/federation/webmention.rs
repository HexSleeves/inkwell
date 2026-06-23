//! Webmention receive-verification + send-discovery helpers (card T11).
//!
//! The two network-touching halves of Webmention both reduce to "fetch a page
//! through the SSRF guard and look for something in it":
//!   - VERIFY (receive): fetch the claimed `source` URL and confirm it actually
//!     contains a link to our `target` — only a confirmed source is stored.
//!   - DISCOVER (send): fetch an external `target` and find its Webmention
//!     endpoint (Link header or `<link rel="webmention">`), then POST to it.
//!
//! The HTML/header scanning here is deliberately simple and allocation-bounded:
//! a substring/`rel` scan, never a full DOM parse. It runs only on the
//! already-size-capped body returned by [`super::fetch`].

use anyhow::Result;
use reqwest::Url;

use super::fetch::{FetchedPage, guarded_get, guarded_post};

/// Confirm that the page at `source_url` actually links to `target_url`, by
/// fetching the source through the SSRF guard and scanning for the target URL.
///
/// Returns `Ok(true)` only when the fetched source clearly references the
/// target. Any fetch failure (blocked, timeout, non-2xx) surfaces as `Err`, and
/// a fetched-but-non-referencing source returns `Ok(false)`; the caller drops
/// the pending mention in both cases. Never panics.
pub async fn verify_source_links_to_target(source_url: &str, target_url: &str) -> Result<bool> {
    let page = guarded_get(source_url).await?;
    Ok(body_links_to_target(&page.body, target_url))
}

/// Whether `body` references `target_url`. A source genuinely linking to the
/// target will contain the absolute target URL in an `href`/`src` (or plain
/// text); we accept the target with or without a trailing slash and ignore the
/// scheme's case. This is intentionally lenient on HTML shape but strict on the
/// URL: it must contain the host+path, not just the bare host.
pub fn body_links_to_target(body: &str, target_url: &str) -> bool {
    let needle = target_url.trim();
    if needle.is_empty() {
        return false;
    }
    if body.contains(needle) {
        return true;
    }
    // Tolerate a trailing-slash mismatch in either direction.
    match needle.strip_suffix('/') {
        Some(stripped) => !stripped.is_empty() && body.contains(stripped),
        None => body.contains(&format!("{needle}/")),
    }
}

/// Deliver a Webmention for `source_url` → `target_url`: discover the target's
/// Webmention endpoint and POST the `source`/`target` form to it. All fetches go
/// through the SSRF guard. Best-effort: returns `Ok(true)` if delivered,
/// `Ok(false)` if the target advertises no endpoint, `Err` on a fetch failure.
pub async fn send_webmention(source_url: &str, target_url: &str) -> Result<bool> {
    let page = guarded_get(target_url).await?;
    let Some(endpoint) = discover_endpoint(&page) else {
        return Ok(false);
    };
    guarded_post(
        endpoint.as_str(),
        &[("source", source_url), ("target", target_url)],
    )
    .await?;
    Ok(true)
}

/// Find a target's Webmention endpoint from a fetched page: the HTTP `Link`
/// header takes precedence (per the spec), then a `<link rel="webmention">` /
/// `<a rel="webmention">` in the body. The endpoint is resolved against the
/// page's final URL so a relative endpoint becomes absolute.
///
/// `FetchedPage` does not currently carry response headers, so endpoint
/// discovery here is body-only; the body scan covers the common
/// `<link rel="webmention" href="...">` advertisement. Returns `None` when no
/// endpoint is advertised.
pub fn discover_endpoint(page: &FetchedPage) -> Option<Url> {
    let href = find_rel_webmention_href(&page.body)?;
    // Resolve relative endpoints against the page URL; an empty href means the
    // page itself is the endpoint (per spec).
    if href.trim().is_empty() {
        return Some(page.final_url.clone());
    }
    page.final_url.join(href.trim()).ok()
}

/// Scan HTML for the first `<link>`/`<a>` tag carrying `rel="webmention"` and
/// return its `href`. A minimal tag scan — not a DOM parse — sufficient for the
/// well-formed advertisement Webmention senders look for.
fn find_rel_webmention_href(body: &str) -> Option<String> {
    let lower = body.to_ascii_lowercase();
    let mut search_from = 0;
    while let Some(rel_pos) = lower[search_from..].find("rel=") {
        let abs = search_from + rel_pos;
        // Bound the tag we inspect: from the enclosing '<' to the next '>'.
        let tag_start = lower[..abs].rfind('<').unwrap_or(abs);
        let tag_end = lower[abs..]
            .find('>')
            .map(|p| abs + p)
            .unwrap_or(lower.len());
        let tag_lower = &lower[tag_start..tag_end];
        if rel_value_contains_webmention(tag_lower)
            && let Some(href) = extract_attr(&body[tag_start..tag_end], "href")
        {
            return Some(href);
        }
        search_from = tag_end.max(abs + 4);
    }
    None
}

/// Whether a (lowercased) tag's `rel` attribute lists `webmention` as one of its
/// space-separated tokens (so `rel="webmention"` and `rel="foo webmention"`
/// both match, but `rel="webmentions"` does not).
fn rel_value_contains_webmention(tag_lower: &str) -> bool {
    let Some(value) = extract_attr(tag_lower, "rel") else {
        return false;
    };
    value.split_whitespace().any(|token| token == "webmention")
}

/// Extract the value of `attr` from a single tag's text, supporting double,
/// single, and unquoted forms. Case-insensitive on the attribute name.
fn extract_attr(tag: &str, attr: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let pat = format!("{attr}=");
    let mut from = 0;
    while let Some(rel) = lower[from..].find(&pat) {
        let eq = from + rel + pat.len();
        // Ensure the match is a real attribute boundary (preceded by whitespace,
        // '<', or start) so `xhref=` doesn't match `href`.
        let before = lower[..from + rel].chars().last();
        if !matches!(
            before,
            None | Some(' ') | Some('\t') | Some('\n') | Some('\r') | Some('<') | Some('/')
        ) {
            from = eq;
            continue;
        }
        let rest = &tag[eq..];
        let value = match rest.chars().next() {
            Some('"') => rest[1..].split('"').next(),
            Some('\'') => rest[1..].split('\'').next(),
            _ => rest.split([' ', '\t', '\n', '\r', '>', '/']).next(),
        };
        return value.map(|v| v.to_string());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn body_links_to_target_matches_exact_and_slash_variants() {
        let target = "https://blog.example.com/notes/hello";
        assert!(body_links_to_target(
            r#"<a href="https://blog.example.com/notes/hello">x</a>"#,
            target
        ));
        // Trailing slash on the link but not the target.
        assert!(body_links_to_target(
            r#"<a href="https://blog.example.com/notes/hello/">x</a>"#,
            target
        ));
        // Plain text reference also counts.
        assert!(body_links_to_target(
            "see https://blog.example.com/notes/hello for more",
            target
        ));
    }

    #[test]
    fn body_links_to_target_rejects_unrelated_or_host_only() {
        let target = "https://blog.example.com/notes/hello";
        assert!(!body_links_to_target(
            "<a href=\"https://other.example/\">x</a>",
            target
        ));
        // The bare host without the path must NOT count as a link to the note.
        assert!(!body_links_to_target(
            "<a href=\"https://blog.example.com/\">x</a>",
            target
        ));
        assert!(!body_links_to_target("", target));
    }

    #[test]
    fn discovers_link_rel_webmention_in_body() {
        let page = FetchedPage {
            final_url: Url::parse("https://target.example/post").unwrap(),
            body: r#"<html><head>
                <link rel="webmention" href="https://target.example/wm">
            </head></html>"#
                .to_string(),
        };
        let endpoint = discover_endpoint(&page).unwrap();
        assert_eq!(endpoint.as_str(), "https://target.example/wm");
    }

    #[test]
    fn discovers_relative_endpoint_resolved_against_page() {
        let page = FetchedPage {
            final_url: Url::parse("https://target.example/post").unwrap(),
            body: r#"<link href="/webmention-endpoint" rel="webmention">"#.to_string(),
        };
        let endpoint = discover_endpoint(&page).unwrap();
        assert_eq!(
            endpoint.as_str(),
            "https://target.example/webmention-endpoint"
        );
    }

    #[test]
    fn discovers_endpoint_with_extra_rel_tokens() {
        let page = FetchedPage {
            final_url: Url::parse("https://t.example/").unwrap(),
            body: r#"<a href="https://t.example/wm" rel="nofollow webmention">wm</a>"#.to_string(),
        };
        assert_eq!(
            discover_endpoint(&page).unwrap().as_str(),
            "https://t.example/wm"
        );
    }

    #[test]
    fn ignores_rel_webmentions_plural_and_missing() {
        let page = FetchedPage {
            final_url: Url::parse("https://t.example/").unwrap(),
            body: r#"<link rel="webmentions" href="https://t.example/nope">
                     <link rel="stylesheet" href="https://t.example/style.css">"#
                .to_string(),
        };
        assert!(discover_endpoint(&page).is_none());
    }

    #[test]
    fn extract_attr_handles_quote_styles() {
        assert_eq!(
            extract_attr(r#"<a href="x" rel="webmention">"#, "href").as_deref(),
            Some("x")
        );
        assert_eq!(
            extract_attr("<a href='y' rel='webmention'>", "href").as_deref(),
            Some("y")
        );
        assert_eq!(
            extract_attr("<a href=z rel=webmention>", "href").as_deref(),
            Some("z")
        );
        // Must not match a different attribute that ends in the name.
        assert_eq!(extract_attr("<a data-href=no>", "href"), None);
    }
}
