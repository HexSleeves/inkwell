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
        r#"<h1>Tags</h1>
        <p class="empty">No tags yet.</p>"#
            .to_string()
    } else {
        let max_count = tags.iter().map(|t| t.count).max().unwrap_or(1);

        // Lay out the nodes: the most-used tag (tags[0], the DB already sorts by
        // count desc) anchors the center; every other tag fans out evenly around
        // a circle so edges stay readable. Positions feed both nodes and edges.
        let mut positions: HashMap<&str, (f64, f64)> = HashMap::new();
        positions.insert(tags[0].tag.as_str(), (250.0, 250.0));
        let n_other = tags.len() - 1;
        for (i, tag) in tags[1..].iter().enumerate() {
            let angle = 2.0 * PI * (i as f64) / (n_other as f64);
            let x = 250.0 + 200.0 * angle.cos();
            let y = 250.0 + 200.0 * angle.sin();
            positions.insert(tag.tag.as_str(), (x, y));
        }

        // Edges first so they paint behind the nodes. Stroke-width scales from
        // 1.0 to 3.0 with the pair's co-occurrence count. A co-occurrence whose
        // tags are missing from the layout is skipped rather than panicking.
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

        // Nodes: radius scales linearly from 12.0 to 28.0 with the tag's usage.
        // The visible label is truncated to 10 chars so it stays inside the disc.
        let nodes = tags
            .iter()
            .map(|tag| {
                let (x, y) = positions[tag.tag.as_str()];
                let radius = 12.0 + 16.0 * (tag.count as f64 / max_count as f64);
                let label = escape_html(truncate_on_char_boundary(&tag.tag, 10));
                format!(
                    r#"<a href="/tags/{encoded}" class="tag-node" data-tag="{escaped}">
            <title>{escaped} ({count})</title>
            <circle cx="{x:.1}" cy="{y:.1}" r="{radius:.1}" fill="rgb(47 93 69)"/>
            <text x="{x:.1}" y="{y:.1}">{label}</text>
          </a>"#,
                    encoded = urlencoding::encode(&tag.tag),
                    escaped = escape_html(&tag.tag),
                    count = tag.count,
                )
            })
            .collect::<Vec<_>>()
            .join("\n");

        let svg = format!(
            r#"<svg viewBox="0 0 500 500">
{edges}
{nodes}
          </svg>"#
        );

        // Sidebar: the same tags (already count-desc) as a filterable list. The
        // lowercase data-tag attribute is what the inline filter matches against.
        let sidebar_items = tags
            .iter()
            .map(|tag| {
                format!(
                    r#"<li data-tag="{escaped}"><a href="/tags/{encoded}">{escaped} <span class="count">{count}</span></a></li>"#,
                    escaped = escape_html(&tag.tag),
                    encoded = urlencoding::encode(&tag.tag),
                    count = tag.count,
                )
            })
            .collect::<Vec<_>>()
            .join("\n");

        let nonce = escape_html(csp_nonce);
        format!(
            r#"<h1>Tags</h1>
        <div class="tag-graph-layout">
          <div class="tag-graph-panel">{svg}</div>
          <div class="tag-sidebar-panel">
            <input id="tag-filter" type="search" placeholder="Filter tags…" aria-label="Filter tags" autocomplete="off" />
            <ul id="tag-sidebar-list">
{sidebar_items}
            </ul>
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
        assert!(html.contains(r#"<svg viewBox="0 0 500 500">"#));

        // The most-used tag anchors the graph center.
        assert!(html.contains(r#"<circle cx="250.0" cy="250.0" r="28.0" fill="rgb(47 93 69)"/>"#));

        // Node link + accessible title for a tag.
        assert!(html.contains(r#"<a href="/tags/rust" class="tag-node" data-tag="rust">"#));
        assert!(html.contains("<title>rust (5)</title>"));

        // An edge is rendered for the co-occurring pair.
        assert!(html.contains(r#"<line class="tag-edge""#));

        // Sidebar filter input + list items.
        assert!(html.contains(r#"<input id="tag-filter" type="search""#));
        assert!(html.contains(r#"<ul id="tag-sidebar-list">"#));
        assert!(html.contains(
            r#"<li data-tag="rust"><a href="/tags/rust">rust <span class="count">5</span></a></li>"#
        ));
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
