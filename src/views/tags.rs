use crate::domain::document::{DocumentSummary, TagCount};

use super::layout::{HeadMeta, SiteMeta, escape_html, render_document_list, render_page};

pub fn render_tag_index_page(tags: &[TagCount], site: &SiteMeta<'_>) -> String {
    let body = if tags.is_empty() {
        r#"<p class="empty">No tags yet.</p>"#.to_string()
    } else {
        let items = tags
            .iter()
            .map(|tag| {
                format!(
                    r#"          <li><a href="/tags/{}">{} <span class="count">{}</span></a></li>"#,
                    urlencoding::encode(&tag.tag),
                    escape_html(&tag.tag),
                    tag.count
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        format!(
            r#"<ul class="tags">
{}
        </ul>"#,
            items
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
            csp_nonce: None,
        },
        &format!("<h1>Tags</h1>\n        {}", body),
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
