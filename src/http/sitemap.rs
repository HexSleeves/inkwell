use axum::extract::{Path, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};

use crate::db::documents;
use crate::domain::document::{Document, DocumentStatus, ListOptions, TagCount};
use crate::http::AppState;
use crate::http::cache;
use crate::views::layout::{escape_xml, normalize_site_url};

pub const SITEMAP_CONTENT_TYPE: &str = "application/xml; charset=utf-8";
const SITEMAP_MAX_URLS: u32 = 10_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SitemapShape {
    Single,
    Index { document_pages: u32, tag_pages: u32 },
}

pub async fn sitemap(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let base = normalize_site_url(state.config.site_url.as_deref());
    let document_count = match documents::count_documents(
        &state.pool,
        crate::domain::document::StatusFilter {
            status: Some(DocumentStatus::Published),
        },
    )
    .await
    {
        Ok(count) => count,
        Err(_) => return xml_error_response(),
    };
    let tag_count = match documents::count_published_tags(&state.pool).await {
        Ok(count) => count,
        Err(_) => return xml_error_response(),
    };

    match plan_sitemap_shape(document_count, tag_count) {
        SitemapShape::Single => {
            let document_limit = clamp_count_to_u32(document_count);
            let tag_limit = clamp_count_to_u32(tag_count);
            let documents = match documents::list_documents(
                &state.pool,
                ListOptions {
                    limit: Some(document_limit),
                    offset: Some(0),
                    status: Some(DocumentStatus::Published),
                },
            )
            .await
            {
                Ok(documents) => documents,
                Err(_) => return xml_error_response(),
            };
            let tags = if tag_limit == 0 {
                Vec::new()
            } else {
                match documents::list_published_tags_page(&state.pool, tag_limit, 0).await {
                    Ok(tags) => tags,
                    Err(_) => return xml_error_response(),
                }
            };
            cache::xml_response(
                &headers,
                "sitemap",
                StatusCode::OK,
                SITEMAP_CONTENT_TYPE,
                render_small_sitemap(&base, &documents, &tags),
            )
        }
        shape => cache::xml_response(
            &headers,
            "sitemap",
            StatusCode::OK,
            SITEMAP_CONTENT_TYPE,
            render_sitemap_index(&base, shape),
        ),
    }
}

pub async fn sitemap_static(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let base = normalize_site_url(state.config.site_url.as_deref());
    let tag_count = match documents::count_published_tags(&state.pool).await {
        Ok(count) => count,
        Err(_) => return xml_error_response(),
    };
    let xml = render_static_sitemap(&base, tag_count > 0);
    cache::xml_response(
        &headers,
        "sitemap-static",
        StatusCode::OK,
        SITEMAP_CONTENT_TYPE,
        xml,
    )
}

pub async fn sitemap_documents_page(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(page): Path<String>,
) -> Response {
    let Some(page) = parse_page_number(&page) else {
        return xml_response(StatusCode::NOT_FOUND, String::new());
    };

    let base = normalize_site_url(state.config.site_url.as_deref());
    let documents =
        match documents::list_documents(&state.pool, published_document_page_options(page)).await {
            Ok(documents) => documents,
            Err(_) => return xml_error_response(),
        };
    cache::xml_response(
        &headers,
        "sitemaps-documents",
        StatusCode::OK,
        SITEMAP_CONTENT_TYPE,
        render_document_sitemap(&base, &documents),
    )
}

pub async fn sitemap_tags_page(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(page): Path<String>,
) -> Response {
    let Some(page) = parse_page_number(&page) else {
        return xml_response(StatusCode::NOT_FOUND, String::new());
    };

    let base = normalize_site_url(state.config.site_url.as_deref());
    let tags = match documents::list_published_tags_page(
        &state.pool,
        SITEMAP_MAX_URLS,
        sitemap_page_offset(page),
    )
    .await
    {
        Ok(tags) => tags,
        Err(_) => return xml_error_response(),
    };
    cache::xml_response(
        &headers,
        "sitemaps-tags",
        StatusCode::OK,
        SITEMAP_CONTENT_TYPE,
        render_tag_sitemap(&base, &tags),
    )
}

fn clamp_count_to_u32(count: i64) -> u32 {
    count.clamp(0, i64::from(u32::MAX)) as u32
}

fn plan_sitemap_shape(document_count: i64, tag_count: i64) -> SitemapShape {
    let total_urls = document_count
        .max(0)
        .saturating_add(tag_count.max(0))
        .saturating_add(static_url_count(tag_count));
    if total_urls <= i64::from(SITEMAP_MAX_URLS) {
        SitemapShape::Single
    } else {
        SitemapShape::Index {
            document_pages: sitemap_page_count(document_count),
            tag_pages: sitemap_page_count(tag_count),
        }
    }
}

fn static_url_count(tag_count: i64) -> i64 {
    // home + archive + optional tags index
    2 + if tag_count > 0 { 1 } else { 0 }
}

fn sitemap_page_count(count: i64) -> u32 {
    let count = count.max(0) as u64;
    if count == 0 {
        0
    } else {
        count.div_ceil(u64::from(SITEMAP_MAX_URLS)) as u32
    }
}

fn sitemap_page_offset(page: u32) -> u32 {
    page.saturating_sub(1).saturating_mul(SITEMAP_MAX_URLS)
}

fn published_document_page_options(page: u32) -> ListOptions {
    ListOptions {
        limit: Some(SITEMAP_MAX_URLS),
        offset: Some(sitemap_page_offset(page)),
        status: Some(DocumentStatus::Published),
    }
}

fn render_small_sitemap(base: &str, documents: &[Document], tags: &[TagCount]) -> String {
    let mut entries = vec![render_home_url(base, documents.first())];
    entries.extend(render_document_urls(base, documents));
    entries.push(render_plain_url(&format!("{}/archive", escape_xml(base))));
    if !tags.is_empty() {
        entries.push(render_plain_url(&format!("{}/tags", escape_xml(base))));
        entries.extend(render_tag_urls(base, tags));
    }
    render_urlset(entries)
}

fn render_static_sitemap(base: &str, has_tags: bool) -> String {
    let mut entries = vec![
        render_plain_url(&format!("{}/", escape_xml(base))),
        render_plain_url(&format!("{}/archive", escape_xml(base))),
    ];
    if has_tags {
        entries.push(render_plain_url(&format!("{}/tags", escape_xml(base))));
    }
    render_urlset(entries)
}

fn render_document_sitemap(base: &str, documents: &[Document]) -> String {
    render_urlset(render_document_urls(base, documents))
}

fn render_tag_sitemap(base: &str, tags: &[TagCount]) -> String {
    render_urlset(render_tag_urls(base, tags))
}

fn render_sitemap_index(base: &str, shape: SitemapShape) -> String {
    let mut entries = vec![render_index_entry(&format!(
        "{}/sitemap-static.xml",
        escape_xml(base)
    ))];

    if let SitemapShape::Index {
        document_pages,
        tag_pages,
    } = shape
    {
        for page in 1..=document_pages {
            entries.push(render_index_entry(&format!(
                "{}/sitemaps/documents/{}",
                escape_xml(base),
                page
            )));
        }
        for page in 1..=tag_pages {
            entries.push(render_index_entry(&format!(
                "{}/sitemaps/tags/{}",
                escape_xml(base),
                page
            )));
        }
    }

    format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<sitemapindex xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\">\n{}\n</sitemapindex>\n",
        entries.join("\n")
    )
}

fn render_document_urls(base: &str, documents: &[Document]) -> Vec<String> {
    documents
        .iter()
        .map(|document| {
            render_url_with_lastmod(
                &format!(
                    "{}/{}",
                    escape_xml(base),
                    urlencoding::encode(&document.slug)
                ),
                &crate::domain::document::timestamp::serialize_to_string(&document.updated_at),
            )
        })
        .collect()
}

fn render_tag_urls(base: &str, tags: &[TagCount]) -> Vec<String> {
    tags.iter()
        .map(|tag| {
            render_plain_url(&format!(
                "{}/tags/{}",
                escape_xml(base),
                urlencoding::encode(&tag.tag)
            ))
        })
        .collect()
}

fn render_home_url(base: &str, document: Option<&Document>) -> String {
    match document {
        Some(document) => render_url_with_lastmod(
            &format!("{}/", escape_xml(base)),
            &crate::domain::document::timestamp::serialize_to_string(&document.updated_at),
        ),
        None => render_plain_url(&format!("{}/", escape_xml(base))),
    }
}

fn render_urlset(entries: Vec<String>) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<urlset xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\">\n{}\n</urlset>\n",
        entries.join("\n")
    )
}

fn render_index_entry(loc: &str) -> String {
    format!("  <sitemap>\n    <loc>{loc}</loc>\n  </sitemap>")
}

fn render_plain_url(loc: &str) -> String {
    format!("  <url>\n    <loc>{loc}</loc>\n  </url>")
}

fn render_url_with_lastmod(loc: &str, lastmod: &str) -> String {
    format!("  <url>\n    <loc>{loc}</loc>\n    <lastmod>{lastmod}</lastmod>\n  </url>")
}

fn parse_page_number(value: &str) -> Option<u32> {
    if value.is_empty() || value.starts_with('0') || !value.chars().all(|ch| ch.is_ascii_digit()) {
        return None;
    }
    value.parse().ok()
}

fn xml_response(status: StatusCode, xml: String) -> Response {
    (
        status,
        [(
            header::CONTENT_TYPE,
            HeaderValue::from_static(SITEMAP_CONTENT_TYPE),
        )],
        xml,
    )
        .into_response()
}

fn xml_error_response() -> Response {
    xml_response(StatusCode::INTERNAL_SERVER_ERROR, String::new())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::document::DocumentStatus;

    #[test]
    fn sitemap_stays_single_when_total_urls_fit_within_limit() {
        // static_url_count now includes home + /archive (+ optional /tags), so
        // the single-sitemap ceiling is 3 lower than before:
        //   9_996 docs + 1 tag + static(1 tag)=3 = 10_000 → still Single.
        assert_eq!(plan_sitemap_shape(9_996, 1), SitemapShape::Single);
    }

    #[test]
    fn sitemap_index_splits_document_and_tag_pages_when_total_urls_exceed_limit() {
        assert_eq!(
            plan_sitemap_shape(10_000, 10_001),
            SitemapShape::Index {
                document_pages: 1,
                tag_pages: 2,
            }
        );
    }

    #[test]
    fn published_document_page_options_use_bounded_limit_and_offset() {
        let options = published_document_page_options(2);
        assert_eq!(options.limit, Some(SITEMAP_MAX_URLS));
        assert_eq!(options.offset, Some(SITEMAP_MAX_URLS));
        assert_eq!(options.status, Some(DocumentStatus::Published));
    }

    #[test]
    fn sitemap_index_lists_static_document_and_tag_parts() {
        let xml = render_sitemap_index(
            "https://example.com",
            SitemapShape::Index {
                document_pages: 2,
                tag_pages: 1,
            },
        );

        assert!(xml.contains("<loc>https://example.com/sitemap-static.xml</loc>"));
        assert!(xml.contains("<loc>https://example.com/sitemaps/documents/1</loc>"));
        assert!(xml.contains("<loc>https://example.com/sitemaps/documents/2</loc>"));
        assert!(xml.contains("<loc>https://example.com/sitemaps/tags/1</loc>"));
    }

    #[test]
    fn small_sitemap_includes_archive_url() {
        let xml = render_small_sitemap("https://example.com", &[], &[]);
        assert!(
            xml.contains("<loc>https://example.com/archive</loc>"),
            "small sitemap must include /archive so crawlers discover date browsing"
        );
    }

    #[test]
    fn static_sitemap_includes_archive_url() {
        let xml = render_static_sitemap("https://example.com", false);
        assert!(
            xml.contains("<loc>https://example.com/archive</loc>"),
            "static sitemap must include /archive"
        );
    }
}
