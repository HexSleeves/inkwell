use std::collections::HashSet;

use crate::db::links::Backlink;
use crate::domain::document::{Document, GrowthStage, timestamp};
use crate::rendering::wikilink::render_snippet_with_links;

use super::layout::{
    HeadMeta, SITE_NAME, date_line, derive_excerpt, escape_html, json_ld_document,
    normalize_site_url, render_page, render_tag_chips, truncate_on_char_boundary,
};

/// Longest backlink context snippet (in bytes) shown before multibyte-safe
/// truncation. Long contexts are clipped so the sidebar stays compact.
const BACKLINK_SNIPPET_MAX: usize = 160;

pub fn render_document_page(
    document: &Document,
    backlinks: &[Backlink],
    snippet_links: &HashSet<String>,
    site_url: Option<&str>,
    csp_nonce: &str,
) -> String {
    let base = normalize_site_url(site_url);
    let url = format!("{}/{}", base, urlencoding::encode(&document.slug));
    let description = derive_excerpt(document.body_markdown(), 160);
    let created = timestamp::serialize_to_string(&document.created_at);
    let updated = timestamp::serialize_to_string(&document.updated_at);
    let updated_text = if document.updated_at != document.created_at {
        format!(" &middot; {}", date_line("Updated", document.updated_at))
    } else {
        String::new()
    };
    let main = format!(
        r#"<article>
          <h1>{}</h1>
          <div class="meta">{}{}{}</div>{}
{}
        </article>"#,
        escape_html(&document.title),
        date_line("Published", document.created_at),
        updated_text,
        render_growth_chip(document.growth),
        render_tag_chips(&document.tags),
        document.rendered_html()
    );
    let main = format!(
        "{}{}",
        main,
        render_backlinks_panel(backlinks, snippet_links)
    );
    render_page(
        HeadMeta {
            title: &format!("{} — {}", document.title, SITE_NAME),
            description: if description.is_empty() {
                None
            } else {
                Some(&description)
            },
            canonical_url: url.clone(),
            og_type: "article",
            json_ld: Some(json_ld_document(
                &document.title,
                if description.is_empty() {
                    None
                } else {
                    Some(&description)
                },
                &url,
                &created,
                &updated,
                &document.tags,
            )),
            csp_nonce: Some(csp_nonce),
        },
        &main,
    )
}

/// Render the digital-garden maturity chip (seedling/budding/evergreen) for the
/// note's `<div class="meta">` line. The stage token is a fixed, sanitized
/// vocabulary, so it is safe to interpolate into both the class and the label.
fn render_growth_chip(growth: GrowthStage) -> String {
    format!(
        r#" &middot; <span class="growth growth-{stage}">{stage}</span>"#,
        stage = growth.as_str(),
    )
}

/// Render the "Linked from" panel listing every note that links to this one.
/// Each entry links to `/{source_slug}` (slug URL-encoded, title escaped) with
/// its context snippet beneath, truncated multibyte-safely. Returns an empty
/// string when there are no backlinks so the caller emits no empty box.
fn render_backlinks_panel(backlinks: &[Backlink], snippet_links: &HashSet<String>) -> String {
    if backlinks.is_empty() {
        return String::new();
    }
    let items = backlinks
        .iter()
        .map(|backlink| {
            let snippet = backlink
                .context_snippet
                .as_deref()
                .map(str::trim)
                .filter(|snippet| !snippet.is_empty())
                .map(|snippet| {
                    let clipped = truncate_on_char_boundary(snippet, BACKLINK_SNIPPET_MAX);
                    let ellipsis = if clipped.len() < snippet.len() {
                        "…"
                    } else {
                        ""
                    };
                    // `[[wikilinks]]` inside the context become live links (or
                    // stubs); everything else is escaped by the snippet renderer.
                    format!(
                        r#"
            <p class="backlink-context">{}{}</p>"#,
                        render_snippet_with_links(clipped.trim_end(), snippet_links),
                        ellipsis
                    )
                })
                .unwrap_or_default();
            format!(
                r#"          <li>
            <a class="backlink" href="/{}">{}</a>{}
          </li>"#,
                urlencoding::encode(&backlink.source_slug),
                escape_html(&backlink.source_title),
                snippet
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        r#"
        <aside class="backlinks" aria-label="Linked from">
          <h2>Linked from</h2>
          <ul class="backlinks-list">
{}
          </ul>
        </aside>"#,
        items
    )
}

pub fn render_not_found_page(site_url: Option<&str>) -> String {
    let base = normalize_site_url(site_url);
    render_page(
        HeadMeta {
            title: &format!("Not found — {}", SITE_NAME),
            description: None,
            canonical_url: format!("{}/", base),
            og_type: "website",
            json_ld: None,
            csp_nonce: None,
        },
        r#"<h1>Not found</h1>
        <p>That page does not exist. <a href="/">Back to the index.</a></p>"#,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn backlink(slug: &str, title: &str, snippet: Option<&str>) -> Backlink {
        Backlink {
            source_slug: slug.to_string(),
            source_title: title.to_string(),
            context_snippet: snippet.map(str::to_string),
        }
    }

    #[test]
    fn empty_backlinks_omit_the_panel_entirely() {
        assert_eq!(render_backlinks_panel(&[], &HashSet::new()), "");
    }

    #[test]
    fn growth_chip_renders_the_stage_as_a_labelled_span() {
        let chip = render_growth_chip(GrowthStage::Budding);
        assert!(chip.contains(r#"class="growth growth-budding""#));
        assert!(chip.contains(">budding<"));
        // Seedling default also renders.
        assert!(render_growth_chip(GrowthStage::Seedling).contains("seedling"));
    }

    #[test]
    fn panel_links_to_each_source_with_escaped_title_and_encoded_slug() {
        let resolved: HashSet<String> = ["here".to_string()].into_iter().collect();
        let html = render_backlinks_panel(
            &[backlink("a b", "Title & <Stuff>", Some("see [[here]]"))],
            &resolved,
        );
        assert!(html.contains(r#"class="backlinks""#));
        assert!(html.contains("Linked from"));
        // Slug is URL-encoded in the href.
        assert!(
            html.contains(r#"href="/a%20b""#),
            "slug must be url-encoded"
        );
        // Title is HTML-escaped.
        assert!(html.contains("Title &amp; &lt;Stuff&gt;"));
        // The `[[here]]` in the context snippet becomes a live link with its
        // brackets preserved inside the anchor.
        assert!(html.contains(r#"<a href="/here">[[here]]</a>"#));
    }

    #[test]
    fn panel_snippet_wikilink_to_a_missing_target_is_a_stub() {
        let html = render_backlinks_panel(
            &[backlink("s", "S", Some("see [[ghost]]"))],
            &HashSet::new(),
        );
        assert!(html.contains(r#"<a class="stub" href="/ghost">[[ghost]]</a>"#));
    }

    #[test]
    fn long_snippet_is_truncated_on_a_char_boundary_with_ellipsis() {
        // A multibyte char (é = 2 bytes) straddling the cap must not panic and
        // must not appear half-sliced.
        let snippet = format!("{}é tail", "x".repeat(BACKLINK_SNIPPET_MAX));
        let html = render_backlinks_panel(&[backlink("s", "S", Some(&snippet))], &HashSet::new());
        assert!(
            html.contains('…'),
            "long snippet is truncated with an ellipsis"
        );
        assert!(!html.contains("tail"), "text past the cap is dropped");
    }

    #[test]
    fn blank_snippet_renders_link_without_a_context_paragraph() {
        let html = render_backlinks_panel(&[backlink("s", "S", Some("   "))], &HashSet::new());
        assert!(html.contains(r#"href="/s""#));
        assert!(
            !html.contains("backlink-context"),
            "a whitespace-only snippet emits no context paragraph"
        );
    }
}
