use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};

use crate::db::documents;
use crate::http::AppState;
use crate::http::cache;
use crate::views::layout::{escape_xml, normalize_site_url};

pub const ATOM_CONTENT_TYPE: &str = "application/atom+xml; charset=utf-8";
const FEED_MAX_ENTRIES: u32 = 20;

pub async fn feed(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let documents = match documents::list_documents(
        &state.pool,
        crate::domain::document::ListOptions {
            limit: Some(FEED_MAX_ENTRIES),
            offset: None,
            status: Some(crate::domain::document::DocumentStatus::Published),
        },
    )
    .await
    {
        Ok(documents) => documents,
        Err(_) => return xml_error_response(),
    };

    let base = normalize_site_url(state.config.site_url.as_deref());
    let updated = documents
        .first()
        .map(|doc| crate::domain::document::timestamp::serialize_to_string(&doc.updated_at))
        .unwrap_or_else(|| "1970-01-01T00:00:00.000Z".to_string());
    let entries = documents.iter().map(|doc| {
        let url = format!("{}/{}", base, urlencoding::encode(&doc.slug));
        format!("  <entry>\n    <title>{}</title>\n    <id>{}</id>\n    <link rel=\"alternate\" type=\"text/html\" href=\"{}\" />\n    <published>{}</published>\n    <updated>{}</updated>\n    <content type=\"html\">{}</content>\n  </entry>", escape_xml(&doc.title), escape_xml(&url), escape_xml(&url), crate::domain::document::timestamp::serialize_to_string(&doc.created_at), crate::domain::document::timestamp::serialize_to_string(&doc.updated_at), escape_xml(doc.rendered_html()))
    }).collect::<Vec<_>>().join("\n");
    let xml = format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<feed xmlns=\"http://www.w3.org/2005/Atom\">\n  <title>Inkwell</title>\n  <id>{}/</id>\n  <updated>{}</updated>\n  <link rel=\"self\" type=\"application/atom+xml\" href=\"{}/feed.xml\" />\n  <link rel=\"alternate\" type=\"text/html\" href=\"{}/\" />\n{}\n</feed>\n",
        escape_xml(&base),
        updated,
        escape_xml(&base),
        escape_xml(&base),
        entries
    );

    cache::xml_response(&headers, "feed", StatusCode::OK, ATOM_CONTENT_TYPE, xml)
}

fn xml_error_response() -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        [(
            header::CONTENT_TYPE,
            HeaderValue::from_static(ATOM_CONTENT_TYPE),
        )],
        String::new(),
    )
        .into_response()
}
