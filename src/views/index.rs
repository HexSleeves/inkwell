use crate::domain::document::Document;

use super::layout::{
    HeadMeta, SITE_NAME, date_line, derive_excerpt, escape_html, normalize_site_url, render_page,
    render_tag_chips,
};

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
        let items = documents
            .iter()
            .map(|doc| {
                let excerpt = derive_excerpt(doc.body_markdown(), 160);
                let excerpt_html = if excerpt.is_empty() {
                    String::new()
                } else {
                    format!(
                        r#"\n            <p class="excerpt">{}</p>"#,
                        escape_html(&excerpt)
                    )
                };
                format!(
                    r#"          <li>
            <a class="title" href="/{}">{}</a>
            <div class="meta">{}</div>{}{}
          </li>"#,
                    urlencoding::encode(&doc.slug),
                    escape_html(&doc.title),
                    date_line("Published", doc.created_at),
                    excerpt_html,
                    render_tag_chips(&doc.tags),
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        format!(
            r#"<ul class="index">
{}
        </ul>"#,
            items
        )
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
