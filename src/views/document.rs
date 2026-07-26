use std::collections::HashSet;

use crate::db::links::Backlink;
use crate::domain::document::{AdjacentDoc, Document, GrowthStage, timestamp};
use crate::rendering::wikilink::render_snippet_with_links;
use crate::views::theme::slot;

use super::layout::{
    BADGE_ICONS, HeadMeta, SITE_NAME, SPROUT_ICON, SiteMeta, date_line, derive_excerpt,
    escape_html, json_ld_document, render_page, render_public_page, render_tag_chips,
    truncate_on_char_boundary,
};

/// Longest backlink context snippet (in bytes) shown before multibyte-safe
/// truncation. Long contexts are clipped so the sidebar stays compact.
const BACKLINK_SNIPPET_MAX: usize = 160;

pub fn render_document_page(
    document: &Document,
    backlinks: &[Backlink],
    snippet_links: &HashSet<String>,
    site: &SiteMeta<'_>,
    csp_nonce: &str,
    prev: Option<&AdjacentDoc>,
    next: Option<&AdjacentDoc>,
) -> String {
    let url = format!("{}/{}", site.base_url, urlencoding::encode(&document.slug));
    let description = derive_excerpt(document.body_markdown(), 160);
    let created = timestamp::serialize_to_string(&document.created_at);
    let updated = timestamp::serialize_to_string(&document.updated_at);
    let updated_text = if document.updated_at != document.created_at {
        format!(" &middot; {}", date_line("Updated", document.updated_at))
    } else {
        String::new()
    };
    let meta_line = format!(
        r#"<div class="meta">{}{}{}</div>"#,
        date_line("Published", document.created_at),
        updated_text,
        render_growth_chip(document.growth),
    );
    let tags = render_tag_chips(&document.tags);
    let backlinks_panel = render_backlinks_panel(backlinks, snippet_links);
    let doc_nav = render_adjacent_nav(prev, next);
    // `body_html` is the *sanitized* document body. A theme composes fragments;
    // it has no route to the unsanitized markdown or pre-sanitize HTML, so
    // `rendering::sanitize` stays the only gate on untrusted markup.
    let main = site
        .theme
        .and_then(|theme| {
            theme.render_slot(
                slot::DOCUMENT,
                &[
                    ("site_name", &escape_html(site.name)),
                    ("title", &escape_html(&document.title)),
                    ("slug", &escape_html(&document.slug)),
                    ("url", &escape_html(&url)),
                    ("body_html", document.rendered_html()),
                    ("meta_line", &meta_line),
                    ("published", &date_line("Published", document.created_at)),
                    ("updated", &date_line("Updated", document.updated_at)),
                    ("growth", &render_growth_badge(document.growth)),
                    ("tags", &tags),
                    ("backlinks", &backlinks_panel),
                    ("doc_nav", &doc_nav),
                ],
            )
        })
        .unwrap_or_else(|| {
            format!(
                r#"<article>
          <h1>{}</h1>
          {meta_line}{tags}
{}
        </article>{backlinks_panel}{doc_nav}"#,
                escape_html(&document.title),
                document.rendered_html(),
            )
        });
    let desc = if description.is_empty() {
        None
    } else {
        Some(description.as_str())
    };
    render_public_page(
        site,
        HeadMeta {
            title: &format!("{} — {}", document.title, site.name),
            description: desc,
            canonical_url: url.clone(),
            og_type: "article",
            json_ld: Some(json_ld_document(
                &document.title,
                desc,
                &url,
                &created,
                &updated,
                &document.tags,
                site.name,
                site.author,
            )),
            csp_nonce: Some(csp_nonce),
            nav_current: Some("notes"),
            wide_layout: false,
        },
        &main,
    )
}

/// Render previous/next document navigation below the note body. Either
/// neighbour may be absent (first or last published document). Returns an
/// empty string when both are absent so no empty box is emitted.
fn render_adjacent_nav(prev: Option<&AdjacentDoc>, next: Option<&AdjacentDoc>) -> String {
    if prev.is_none() && next.is_none() {
        return String::new();
    }
    let prev_html = match prev {
        Some(doc) => format!(
            r#"<a class="doc-nav-prev" rel="prev" href="/{}">&larr; {}</a>"#,
            urlencoding::encode(&doc.slug),
            escape_html(&doc.title),
        ),
        None => r#"<span class="spacer"></span>"#.to_string(),
    };
    let next_html = match next {
        Some(doc) => format!(
            r#"<a class="doc-nav-next" rel="next" href="/{}">{} &rarr;</a>"#,
            urlencoding::encode(&doc.slug),
            escape_html(&doc.title),
        ),
        None => r#"<span class="spacer"></span>"#.to_string(),
    };
    format!(
        r#"
        <nav class="doc-nav" aria-label="Previous and next document">
          {}
          {}
        </nav>"#,
        prev_html, next_html
    )
}

/// Render the digital-garden maturity chip (seedling/budding/evergreen) for the
/// note's `<div class="meta">` line. The stage token is a fixed, sanitized
/// vocabulary, so it is safe to interpolate into both the class and the label.
fn render_growth_chip(growth: GrowthStage) -> String {
    format!(" &middot; {}", render_growth_badge(growth))
}

/// The chip itself, without the leading meta-line separator. Themes get this
/// form via `{{ growth }}` so they can place it wherever they like.
fn render_growth_badge(growth: GrowthStage) -> String {
    format!(
        r#"<span class="growth growth-{stage}">{icon}{stage}</span>"#,
        stage = growth.as_str(),
        icon = SPROUT_ICON,
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
        .enumerate()
        .map(|(index, backlink)| {
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
            // Cycle a small set of plant glyphs across the card badges so the
            // panel reads as a little garden rather than a plain list.
            let badge = BADGE_ICONS[index % BADGE_ICONS.len()];
            format!(
                r#"          <li>
            <span class="backlink-badge" aria-hidden="true">{}</span>
            <div class="backlink-main">
              <a class="backlink" href="/{}">{}</a>{}
            </div>
          </li>"#,
                badge,
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
          <h2>{icon}Linked from</h2>
          <ul class="backlinks-list">
{items}
          </ul>
        </aside>"#,
        icon = SPROUT_ICON,
        items = items,
    )
}

pub fn render_not_found_page(site_url: Option<&str>) -> String {
    let site = SiteMeta {
        name: SITE_NAME,
        description: None,
        author: None,
        base_url: super::layout::normalize_site_url(site_url),
        custom_css_url: None,
        // The 404 page is reachable without server config in hand, so it always
        // renders with the built-in look.
        theme: None,
    };
    render_page(
        &site,
        HeadMeta {
            title: &format!("Not found — {}", site.name),
            description: None,
            canonical_url: format!("{}/", site.base_url),
            og_type: "website",
            json_ld: None,
            csp_nonce: None,
            nav_current: None,
            wide_layout: false,
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
