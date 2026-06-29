use crate::domain::document::DocumentSummary;

use super::layout::{HeadMeta, SiteMeta, escape_html, render_document_list, render_page};

pub fn render_search_page(
    query: &str,
    documents: &[DocumentSummary],
    page: i64,
    total_pages: i64,
    site: &SiteMeta<'_>,
) -> String {
    let trimmed = query.trim();
    let form = format!(
        r#"<form class="search" action="/search" method="get" role="search">
          <input type="search" name="q" value="{}" placeholder="Search published documents..." aria-label="Search" />
          <button type="submit">Search</button>
        </form>"#,
        escape_html(query)
    );
    let body = if trimmed.is_empty() {
        String::new()
    } else if documents.is_empty() {
        format!(
            r#"<p class="empty">No results for &ldquo;{}&rdquo;.</p>"#,
            escape_html(trimmed)
        )
    } else {
        let pager = if total_pages > 1 {
            let prev = if page > 1 {
                let href = if page - 1 <= 1 {
                    format!("/search?q={}", urlencoding::encode(trimmed))
                } else {
                    format!(
                        "/search?q={}&page={}",
                        urlencoding::encode(trimmed),
                        page - 1
                    )
                };
                format!(r#"<a rel="prev" href="{}">&larr; Newer</a>"#, href)
            } else {
                r#"<span class="spacer">&larr; Newer</span>"#.to_string()
            };
            let next = if page < total_pages {
                format!(
                    r#"<a rel="next" href="/search?q={}&page={}">Older &rarr;</a>"#,
                    urlencoding::encode(trimmed),
                    page + 1
                )
            } else {
                r#"<span class="spacer">Older &rarr;</span>"#.to_string()
            };
            format!(r#"<nav class="pager">{}{}</nav>"#, prev, next)
        } else {
            String::new()
        };
        format!("{}{}", render_document_list(documents), pager)
    };
    let title = if trimmed.is_empty() {
        format!("Search — {}", site.name)
    } else {
        format!("Search: {} — {}", trimmed, site.name)
    };
    render_page(
        site,
        HeadMeta {
            title: &title,
            description: Some("Search published documents."),
            canonical_url: format!("{}/search", site.base_url),
            og_type: "website",
            json_ld: None,
            csp_nonce: None,
            nav_current: None,
            wide_layout: false,
        },
        &format!("<h1>Search</h1>\n        {}\n        {}", form, body),
    )
}
