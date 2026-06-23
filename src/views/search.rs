use crate::domain::document::Document;

use super::layout::{
    HeadMeta, SITE_NAME, escape_html, normalize_site_url, render_document_list, render_page,
};

pub fn render_search_page(
    query: &str,
    documents: &[Document],
    page: i64,
    total_pages: i64,
    site_url: Option<&str>,
) -> String {
    let base = normalize_site_url(site_url);
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
        format!("Search — {}", SITE_NAME)
    } else {
        format!("Search: {} — {}", trimmed, SITE_NAME)
    };
    render_page(
        HeadMeta {
            title: &title,
            description: Some("Search published documents."),
            canonical_url: format!("{}/search", base),
            og_type: "website",
            json_ld: None,
            csp_nonce: None,
        },
        &format!("<h1>Search</h1>\n        {}\n        {}", form, body),
    )
}
