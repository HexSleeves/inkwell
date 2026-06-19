use axum::extract::State;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::IntoResponse;

use crate::db::documents;
use crate::http::AppState;
use crate::views::layout::{escape_xml, normalize_site_url};

pub const SITEMAP_CONTENT_TYPE: &str = "application/xml; charset=utf-8";

pub async fn sitemap(State(state): State<AppState>) -> impl IntoResponse {
    let base = normalize_site_url(state.config.site_url.as_deref());
    let documents = documents::list_documents(
        &state.pool,
        crate::domain::document::ListOptions {
            limit: None,
            offset: None,
            status: Some(crate::domain::document::DocumentStatus::Published),
        },
    )
    .await
    .unwrap_or_default();
    let tags = documents::list_published_tags(&state.pool)
        .await
        .unwrap_or_default();
    let home = if let Some(document) = documents.first() {
        format!(
            "  <url>\n    <loc>{}/</loc>\n    <lastmod>{}</lastmod>\n  </url>",
            escape_xml(&base),
            crate::domain::document::timestamp::serialize_to_string(&document.updated_at)
        )
    } else {
        format!("  <url>\n    <loc>{}/</loc>\n  </url>", escape_xml(&base))
    };
    let urls = documents
        .iter()
        .map(|doc| {
            format!(
                "  <url>\n    <loc>{}/{}</loc>\n    <lastmod>{}</lastmod>\n  </url>",
                escape_xml(&base),
                urlencoding::encode(&doc.slug),
                crate::domain::document::timestamp::serialize_to_string(&doc.updated_at)
            )
        })
        .collect::<Vec<_>>();
    let mut all = vec![home];
    all.extend(urls);
    if !tags.is_empty() {
        all.push(format!(
            "  <url>\n    <loc>{}/tags</loc>\n  </url>",
            escape_xml(&base)
        ));
        all.extend(tags.iter().map(|tag| {
            format!(
                "  <url>\n    <loc>{}/tags/{}</loc>\n  </url>",
                escape_xml(&base),
                urlencoding::encode(&tag.tag)
            )
        }));
    }
    let xml = format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<urlset xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\">\n{}\n</urlset>\n",
        all.join("\n")
    );
    (
        [(
            header::CONTENT_TYPE,
            HeaderValue::from_static(SITEMAP_CONTENT_TYPE),
        )],
        (StatusCode::OK, xml),
    )
}
