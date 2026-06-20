use crate::domain::document::{Document, timestamp};

use super::layout::{
    HeadMeta, SITE_NAME, date_line, derive_excerpt, escape_html, json_ld_document,
    normalize_site_url, render_page, render_tag_chips,
};

pub fn render_document_page(
    document: &Document,
    site_url: Option<&str>,
    csp_nonce: &str,
) -> String {
    let base = normalize_site_url(site_url);
    let url = format!("{}/{}", base, urlencoding::encode(&document.slug));
    let description = derive_excerpt(document.body_markdown(), 160);
    let created = timestamp::serialize_to_string(&document.created_at);
    let updated = timestamp::serialize_to_string(&document.updated_at);
    let updated_text = if document.updated_at != document.created_at {
        format!(" &middot; {}", date_line("Updated", document.updated_at))
    } else {
        String::new()
    };
    let main = format!(
        r#"<article>
          <h1>{}</h1>
          <div class="meta">{}{}</div>{}
{}
        </article>"#,
        escape_html(&document.title),
        date_line("Published", document.created_at),
        updated_text,
        render_tag_chips(&document.tags),
        document.rendered_html()
    );
    render_page(
        HeadMeta {
            title: &format!("{} — {}", document.title, SITE_NAME),
            description: if description.is_empty() {
                None
            } else {
                Some(&description)
            },
            canonical_url: url.clone(),
            og_type: "article",
            json_ld: Some(json_ld_document(
                &document.title,
                if description.is_empty() {
                    None
                } else {
                    Some(&description)
                },
                &url,
                &created,
                &updated,
                &document.tags,
            )),
            csp_nonce: Some(csp_nonce),
        },
        &main,
    )
}

pub fn render_not_found_page(site_url: Option<&str>) -> String {
    let base = normalize_site_url(site_url);
    render_page(
        HeadMeta {
            title: &format!("Not found — {}", SITE_NAME),
            description: None,
            canonical_url: format!("{}/", base),
            og_type: "website",
            json_ld: None,
            csp_nonce: None,
        },
        r#"<h1>Not found</h1>
        <p>That page does not exist. <a href="/">Back to the index.</a></p>"#,
    )
}
