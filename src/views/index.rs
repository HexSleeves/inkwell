use crate::domain::document::Document;

use super::layout::{HeadMeta, SITE_NAME, normalize_site_url, render_document_list, render_page};

pub fn render_index_page(
    documents: &[Document],
    page: i64,
    total_pages: i64,
    site_url: Option<&str>,
) -> String {
    let base = normalize_site_url(site_url);
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
        format!(r#"\n        <nav class="pager">{}{}</nav>"#, prev, next)
    } else {
        String::new()
    };
    let title = if page > 1 {
        format!("{} — Page {}", SITE_NAME, page)
    } else {
        SITE_NAME.to_string()
    };
    let canonical = if page > 1 {
        format!("{}/page/{}", base, page)
    } else {
        format!("{}/", base)
    };
    render_page(
        HeadMeta {
            title: &title,
            description: Some("An open, API-first Markdown publishing platform."),
            canonical_url: canonical,
            og_type: "website",
            json_ld: None,
            csp_nonce: None,
        },
        &format!("{}{}", list, pager),
    )
}
