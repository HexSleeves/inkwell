use crate::domain::document::{Document, TagCount};

use super::layout::{
    HeadMeta, SITE_NAME, date_line, derive_excerpt, escape_html, normalize_site_url, render_page,
    render_tag_chips,
};

fn render_doc_list(documents: &[Document]) -> String {
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
                render_tag_chips(&doc.tags)
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
}

pub fn render_tag_index_page(tags: &[TagCount], site_url: Option<&str>) -> String {
    let base = normalize_site_url(site_url);
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
        HeadMeta {
            title: &format!("Tags — {}", SITE_NAME),
            description: Some("Browse published documents by tag."),
            canonical_url: format!("{}/tags", base),
            og_type: "website",
            json_ld: None,
        },
        &format!("<h1>Tags</h1>\n        {}", body),
    )
}

pub fn render_tag_page(
    tag: &str,
    documents: &[Document],
    page: i64,
    total_pages: i64,
    site_url: Option<&str>,
) -> String {
    let base = normalize_site_url(site_url);
    let list = if documents.is_empty() {
        r#"<p class="empty">No published documents with this tag.</p>"#.to_string()
    } else {
        render_doc_list(documents)
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
        format!(r#"\n        <nav class="pager">{}{}</nav>"#, prev, next)
    } else {
        String::new()
    };
    let title = if page > 1 {
        format!("{} — {} — Page {}", tag, SITE_NAME, page)
    } else {
        format!("{} — {}", tag, SITE_NAME)
    };
    let canonical = if page > 1 {
        format!("{}/tags/{}/page/{}", base, urlencoding::encode(tag), page)
    } else {
        format!("{}/tags/{}", base, urlencoding::encode(tag))
    };
    render_page(
        HeadMeta {
            title: &title,
            description: Some(&format!("Published documents tagged “{}”.", tag)),
            canonical_url: canonical,
            og_type: "website",
            json_ld: None,
        },
        &format!(
            "<h1>Tagged &ldquo;{}&rdquo;</h1>\n        {}{}",
            escape_html(tag),
            list,
            pager
        ),
    )
}
