use crate::domain::document::DocumentSummary;

use super::layout::{HeadMeta, SiteMeta, render_document_list, render_page};

pub fn render_index_page(
    documents: &[DocumentSummary],
    page: i64,
    total_pages: i64,
    site: &SiteMeta<'_>,
) -> String {
    let list = if documents.is_empty() {
        r#"<p class="empty">No documents published yet.</p>"#.to_string()
    } else {
        render_document_list(documents)
    };

    let prev = if page > 1 {
        let href = if page - 1 <= 1 {
            "/".to_string()
        } else {
            format!("/page/{}", page - 1)
        };
        format!(r#"<a rel="prev" href="{}">&larr; Newer</a>"#, href)
    } else {
        r#"<span class="spacer">&larr; Newer</span>"#.to_string()
    };
    let next = if page < total_pages {
        format!(
            r#"<a rel="next" href="/page/{}">Older &rarr;</a>"#,
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
        format!("{} — Page {}", site.name, page)
    } else {
        site.name.to_string()
    };
    let canonical = if page > 1 {
        format!("{}/page/{}", site.base_url, page)
    } else {
        format!("{}/", site.base_url)
    };
    // Use the operator-configured site description when available, else the
    // built-in fallback so the index meta is never empty.
    let description = site
        .description
        .unwrap_or("An open, API-first Markdown publishing platform.");
    render_page(
        site,
        HeadMeta {
            title: &title,
            description: Some(description),
            canonical_url: canonical,
            og_type: "website",
            json_ld: None,
            csp_nonce: None,
        },
        &format!("{}{}", list, pager),
    )
}
