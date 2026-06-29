use std::collections::HashMap;
use std::f64::consts::PI;

use crate::domain::document::{DocumentSummary, TagCooccurrence, TagCount};

use super::layout::{
    HeadMeta, SiteMeta, escape_html, render_document_list, render_page, truncate_on_char_boundary,
};

/// Render the `/tags` overview as a two-panel page: an SVG force-style graph of
/// tags (sized by usage, linked by co-occurrence) on the left, and a filterable
/// sidebar list on the right. The layout is computed in Rust at render time — the
/// only client JS is a vanilla filter on the sidebar, carried under the request's
/// CSP nonce. An empty tag set degrades to a plain "no tags" message with no graph.
pub fn render_tag_index_page(
    tags: &[TagCount],
    cooccurrences: &[TagCooccurrence],
    csp_nonce: &str,
    site: &SiteMeta<'_>,
) -> String {
    let main = if tags.is_empty() {
        r#"<div class="tags-page-header">
          <h1>Tags <span class="accent-dot">·</span> <span class="accent-title">Graph View</span></h1>
          <p class="tags-subtitle">Visualize how your tags connect.</p>
        </div>
        <p class="empty">No tags yet.</p>"#
            .to_string()
    } else {
        let _max_count = tags.iter().map(|t| t.count).max().unwrap_or(1);

        // SVG canvas: 600×600, center at (300,300), satellite orbit radius 195.
        // tags[0] (most-used, DB sorts count desc) anchors the center.
        // All other tags fan out evenly on the orbit ring starting from the top.
        const CX: f64 = 300.0;
        const CY: f64 = 300.0;
        const ORBIT_R: f64 = 195.0;
        const CENTER_R: f64 = 62.0;

        let mut positions: HashMap<&str, (f64, f64)> = HashMap::new();
        positions.insert(tags[0].tag.as_str(), (CX, CY));

        let n_other = tags.len() - 1;
        if n_other > 0 {
            for (i, tag) in tags[1..].iter().enumerate() {
                // Start from top (−π/2) going clockwise for natural visual flow.
                let angle = -std::f64::consts::FRAC_PI_2 + 2.0 * PI * (i as f64) / (n_other as f64);
                let x = CX + ORBIT_R * angle.cos();
                let y = CY + ORBIT_R * angle.sin();
                positions.insert(tag.tag.as_str(), (x, y));
            }
        }

        // Orbital ring behind everything so edges and nodes paint on top.
        let ring = if n_other > 0 {
            format!(r#"<circle class="orbit-ring" cx="{CX:.0}" cy="{CY:.0}" r="{ORBIT_R:.0}"/>"#)
        } else {
            String::new()
        };

        // Edges scaled 1.0–3.0 in stroke-width. Missing positions skipped.
        let max_cooccurrence = cooccurrences.iter().map(|c| c.count).max().unwrap_or(1);
        let edges = cooccurrences
            .iter()
            .filter_map(|c| {
                let &(x1, y1) = positions.get(c.tag1.as_str())?;
                let &(x2, y2) = positions.get(c.tag2.as_str())?;
                let stroke_width = 1.0 + 2.0 * (c.count as f64 / max_cooccurrence as f64);
                Some(format!(
                    r#"<line class="tag-edge" x1="{x1:.1}" y1="{y1:.1}" x2="{x2:.1}" y2="{y2:.1}" stroke-width="{stroke_width:.1}"/>"#
                ))
            })
            .collect::<Vec<_>>()
            .join("\n");

        // Satellite radius: 24–38px based on count among non-center tags.
        let max_satellite_count = tags[1..].iter().map(|t| t.count).max().unwrap_or(1);

        let nodes = tags
            .iter()
            .enumerate()
            .map(|(idx, tag)| {
                let (x, y) = positions[tag.tag.as_str()];
                let is_center = idx == 0;
                let radius = if is_center {
                    CENTER_R
                } else {
                    24.0 + 14.0 * (tag.count as f64 / max_satellite_count as f64)
                };
                let label = escape_html(truncate_on_char_boundary(&tag.tag, 10));
                let note_word = if tag.count == 1 { "note" } else { "notes" };
                let node_class = if is_center {
                    "node-center"
                } else {
                    "node-satellite"
                };
                // Offset text up/down from center: name slightly above, count below.
                let label_y = y - if is_center { 8.0 } else { 6.0 };
                let count_y = y + if is_center { 12.0 } else { 9.0 };
                format!(
                    r#"<a href="/tags/{encoded}" class="tag-node" data-tag="{escaped}">
            <title>{escaped} ({count})</title>
            <circle class="{node_class}" cx="{x:.1}" cy="{y:.1}" r="{radius:.1}"/>
            <text x="{x:.1}" y="{label_y:.1}" class="node-label">{label}</text>
            <text x="{x:.1}" y="{count_y:.1}" class="node-count">{count} {note_word}</text>
          </a>"#,
                    encoded = urlencoding::encode(&tag.tag),
                    escaped = escape_html(&tag.tag),
                    count = tag.count,
                )
            })
            .collect::<Vec<_>>()
            .join("\n");

        let svg = format!(
            r#"<svg class="tag-graph-svg" viewBox="0 0 600 600">
{ring}
{edges}
{nodes}
          </svg>"#
        );

        // Sidebar items: tag-icon + name + pill count badge.
        let tag_item_icon = r##"<svg class="tag-item-icon" viewBox="0 0 24 24" fill="none" aria-hidden="true"><path d="M4 4.5h7L20 13l-7 7-9-9V4.5Z" stroke="currentColor" stroke-width="1.7" stroke-linejoin="round"/><circle cx="8" cy="8.5" r="1.5" fill="currentColor"/></svg>"##;

        let sidebar_items = tags
            .iter()
            .map(|tag| {
                format!(
                    r#"<li data-tag="{escaped}"><a href="/tags/{encoded}">{icon}<span class="tag-label">{escaped}</span><span class="count">{count}</span></a></li>"#,
                    escaped = escape_html(&tag.tag),
                    encoded = urlencoding::encode(&tag.tag),
                    icon = tag_item_icon,
                    count = tag.count,
                )
            })
            .collect::<Vec<_>>()
            .join("\n");

        let search_icon = r##"<svg class="search-icon" viewBox="0 0 24 24" fill="none" aria-hidden="true"><circle cx="11" cy="11" r="8" stroke="currentColor" stroke-width="1.7"/><path d="m21 21-4.35-4.35" stroke="currentColor" stroke-width="1.7" stroke-linecap="round"/></svg>"##;

        let divider_icon = r##"<svg class="divider-plant" viewBox="0 0 24 24" fill="none" aria-hidden="true"><path d="M12 21V8" stroke="currentColor" stroke-width="1.7" stroke-linecap="round"/><path d="M12 14c0-3.6-2.7-5.6-6.5-5.6C5.5 12 8.2 14 12 14Z" fill="currentColor"/><path d="M12 11c0-3 2.2-4.6 5.8-4.6C17.8 9.4 15.6 11 12 11Z" fill="currentColor"/></svg>"##;

        let pot_icon = r##"<svg class="pot-icon" viewBox="0 0 64 64" fill="none" aria-hidden="true"><path d="M32 26v-8" stroke="rgb(91 138 104)" stroke-width="2" stroke-linecap="round"/><path d="M32 20c0-5.5-4-8.5-10-8.5C22 16.5 26 20 32 20Z" fill="rgb(139 185 148)"/><path d="M32 17c0-4.5 3.5-7 9-7C41 14 37.5 17 32 17Z" fill="rgb(100 155 115)"/><path d="M20 32h24l-2.5 18h-19L20 32Z" fill="rgb(197 107 71)"/><path d="M18 30h28a1 1 0 0 1 0 4H18a1 1 0 0 1 0-4Z" fill="rgb(176 90 56)"/></svg>"##;

        let nonce = escape_html(csp_nonce);

        format!(
            r#"<div class="tags-page-header">
          <div>
            <h1>Tags <span class="accent-dot">·</span> <span class="accent-title">Graph View</span></h1>
            <p class="tags-subtitle">Visualize how your tags connect.</p>
          </div>
        </div>
        <div class="tag-graph-layout">
          <div class="tag-graph-panel">{svg}</div>
          <div class="tag-sidebar-panel">
            <div class="tag-search-wrapper">
              {search_icon}
              <input id="tag-filter" type="search" placeholder="Search tags…" aria-label="Search tags" autocomplete="off" />
            </div>
            <div class="tag-list-header">
              <span class="tag-list-title">All Tags</span>
              <div class="tag-list-divider">
                <span class="divider-line"></span>
                {divider_icon}
                <span class="divider-line"></span>
              </div>
            </div>
            <ul id="tag-sidebar-list">
{sidebar_items}
            </ul>
            <div class="tag-sidebar-footer">
              {pot_icon}
              <p class="sidebar-quote">&ldquo;Cultivate connections.<br>Grow knowledge.&rdquo;</p>
            </div>
          </div>
        </div>
        <script nonce="{nonce}">
(function(){{
  var inp = document.getElementById('tag-filter');
  var list = document.getElementById('tag-sidebar-list');
  if (!inp || !list) return;
  inp.addEventListener('input', function(){{
    var q = inp.value.trim().toLowerCase();
    var items = list.querySelectorAll('li');
    items.forEach(function(li){{
      var tag = (li.dataset.tag || '').toLowerCase();
      li.style.display = !q || tag.indexOf(q) !== -1 ? '' : 'none';
    }});
  }});
}})();
</script>"#
        )
    };

    render_page(
        site,
        HeadMeta {
            title: &format!("Tags \u{2014} {}", site.name),
            description: Some("Browse published documents by tag."),
            canonical_url: format!("{}/tags", site.base_url),
            og_type: "website",
            json_ld: None,
            csp_nonce: Some(csp_nonce),
            nav_current: Some("tags"),
            wide_layout: true,
        },
        &main,
    )
}

pub fn render_tag_page(
    tag: &str,
    documents: &[DocumentSummary],
    page: i64,
    total_pages: i64,
    site: &SiteMeta<'_>,
) -> String {
    let list = if documents.is_empty() {
        r#"<p class="empty">No published documents with this tag.</p>"#.to_string()
    } else {
        render_document_list(documents)
    };
    let prev = if page > 1 {
        let href = if page - 1 <= 1 {
            format!("/tags/{}", urlencoding::encode(tag))
        } else {
            format!("/tags/{}/page/{}", urlencoding::encode(tag), page - 1)
        };
        format!(r#"<a rel="prev" href="{}">&larr; Newer</a>"#, href)
    } else {
        r#"<span class="spacer">&larr; Newer</span>"#.to_string()
    };
    let next = if page < total_pages {
        format!(
            r#"<a rel="next" href="/tags/{}/page/{}">Older &rarr;</a>"#,
            urlencoding::encode(tag),
            page + 1
        )
    } else {
        r#"<span class="spacer">Older &rarr;</span>"#.to_string()
    };
    let pager = if total_pages > 1 {
        format!(r#"<nav class="pager">{}{}</nav>"#, prev, next)
    } else {
        String::new()
    };
    let title = if page > 1 {
        format!("{} \u{2014} {} \u{2014} Page {}", tag, site.name, page)
    } else {
        format!("{} \u{2014} {}", tag, site.name)
    };
    let canonical = if page > 1 {
        format!(
            "{}/tags/{}/page/{}",
            site.base_url,
            urlencoding::encode(tag),
            page
        )
    } else {
        format!("{}/tags/{}", site.base_url, urlencoding::encode(tag))
    };
    render_page(
        site,
        HeadMeta {
            title: &title,
            description: Some(&format!(
                "Published documents tagged \u{201c}{}\u{201d}.",
                tag
            )),
            canonical_url: canonical,
            og_type: "website",
            json_ld: None,
            csp_nonce: None,
            nav_current: Some("tags"),
            wide_layout: false,
        },
        &format!(
            "<h1>Tagged &ldquo;{}&rdquo;</h1>\n        {}{}",
            escape_html(tag),
            list,
            pager
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tag(name: &str, count: i64) -> TagCount {
        TagCount {
            tag: name.to_string(),
            count,
        }
    }

    fn cooccurrence(a: &str, b: &str, count: i64) -> TagCooccurrence {
        TagCooccurrence {
            tag1: a.to_string(),
            tag2: b.to_string(),
            count,
        }
    }

    #[test]
    fn empty_tags_render_the_no_tags_message_without_a_graph() {
        let site = SiteMeta::defaults();
        let html = render_tag_index_page(&[], &[], "nonce123", &site);

        assert!(html.contains(r#"<p class="empty">No tags yet.</p>"#));
        assert!(!html.contains("tag-graph-layout"));
        assert!(!html.contains("tag-node"));
        assert!(!html.contains("tag-sidebar-list"));
    }

    #[test]
    fn non_empty_tags_render_the_split_panel_graph_and_sidebar() {
        let site = SiteMeta::defaults();
        let tags = vec![tag("rust", 5), tag("axum", 3), tag("sqlx", 2)];
        let cooccurrences = vec![cooccurrence("axum", "rust", 2)];
        let html = render_tag_index_page(&tags, &cooccurrences, "nonce123", &site);

        // Split-panel scaffold.
        assert!(html.contains(r#"<div class="tag-graph-layout">"#));
        assert!(html.contains(r#"<div class="tag-graph-panel">"#));
        assert!(html.contains(r#"<div class="tag-sidebar-panel">"#));
        assert!(html.contains(r#"<svg class="tag-graph-svg" viewBox="0 0 600 600">"#));

        // Orbital ring present when there are satellites.
        assert!(html.contains(r#"class="orbit-ring""#));

        // Center node at (300, 300) with fixed 62px radius.
        assert!(html.contains(r#"<circle class="node-center" cx="300.0" cy="300.0" r="62.0"/>"#));

        // Node link + accessible title for a tag.
        assert!(html.contains(r#"<a href="/tags/rust" class="tag-node" data-tag="rust">"#));
        assert!(html.contains("<title>rust (5)</title>"));

        // An edge is rendered for the co-occurring pair.
        assert!(html.contains(r#"<line class="tag-edge""#));

        // Sidebar: search input, list, new structure.
        assert!(html.contains(r#"<input id="tag-filter" type="search""#));
        assert!(html.contains(r#"<ul id="tag-sidebar-list">"#));
        assert!(html.contains(r#"data-tag="rust""#));
        assert!(html.contains(r#"href="/tags/rust""#));
        assert!(html.contains(r#"<span class="count">5</span>"#));

        // Page title with accent.
        assert!(html.contains(r#"<span class="accent-title">Graph View</span>"#));

        // Footer quote.
        assert!(html.contains("Cultivate connections"));
    }

    #[test]
    fn long_tag_labels_are_truncated_to_ten_chars_in_the_svg_text() {
        let site = SiteMeta::defaults();
        let tags = vec![tag("supercalifragilistic", 4)];
        let html = render_tag_index_page(&tags, &[], "nonce123", &site);

        // The SVG text label is clipped to 10 chars...
        assert!(html.contains(">supercalif</text>"));
        // ...while the link target and accessible title keep the full tag.
        assert!(html.contains(r#"href="/tags/supercalifragilistic""#));
        assert!(html.contains("<title>supercalifragilistic (4)</title>"));
    }

    #[test]
    fn the_filter_script_carries_the_html_escaped_nonce() {
        let site = SiteMeta::defaults();
        let html = render_tag_index_page(&[tag("rust", 1)], &[], "abc123", &site);
        assert!(html.contains(r#"<script nonce="abc123">"#));

        // A hostile nonce must be escaped, never break out of the attribute.
        let hostile = render_tag_index_page(&[tag("rust", 1)], &[], r#""><x"#, &site);
        assert!(!hostile.contains(r#"<script nonce=""><x">"#));
        assert!(hostile.contains("&quot;&gt;&lt;x"));
    }
}
