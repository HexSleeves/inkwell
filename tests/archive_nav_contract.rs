mod common;

use inkwell::domain::document::{
    AdjacentDoc, ArchiveMonth, Document, DocumentStatus, DocumentSummary, GrowthStage,
};
use inkwell::views::archive::{render_archive_index_page, render_archive_month_page};
use inkwell::views::document::render_document_page;
use inkwell::views::layout::SiteMeta;

fn site() -> SiteMeta<'static> {
    SiteMeta {
        name: "Inkwell",
        description: None,
        author: None,
        base_url: "https://example.com".to_string(),
        custom_css_url: None,
    }
}

fn doc(slug: &str, title: &str) -> Document {
    let now = time::OffsetDateTime::now_utc();
    Document {
        id: uuid::Uuid::nil(),
        slug: slug.to_string(),
        title: title.to_string(),
        body_markdown: "Body text.".to_string(),
        rendered_html: "<p>Body text.</p>".to_string(),
        status: DocumentStatus::Published,
        growth: GrowthStage::Seedling,
        tags: vec![],
        version: 1,
        created_at: now,
        updated_at: now,
    }
}

fn doc_summary(slug: &str, title: &str) -> DocumentSummary {
    let now = time::OffsetDateTime::now_utc();
    DocumentSummary {
        id: uuid::Uuid::nil(),
        slug: slug.to_string(),
        title: title.to_string(),
        body_excerpt_source: "Body text.".to_string(),
        tags: vec![],
        growth: GrowthStage::Seedling,
        status: DocumentStatus::Published,
        created_at: now,
        updated_at: now,
    }
}

fn adjacent(slug: &str, title: &str) -> AdjacentDoc {
    AdjacentDoc {
        slug: slug.to_string(),
        title: title.to_string(),
    }
}

// ── Archive index ─────────────────────────────────────────────────────────────

#[test]
fn archive_index_route_has_canonical_meta_tag() {
    let html = render_archive_index_page(&[], &site());
    assert!(
        html.contains(r#"rel="canonical" href="https://example.com/archive""#),
        "archive index must carry a canonical URL"
    );
}

#[test]
fn archive_index_has_cache_headers_friendly_og_type() {
    let html = render_archive_index_page(&[], &site());
    assert!(
        html.contains(r#"og:type" content="website""#),
        "archive index og:type must be website"
    );
}

#[test]
fn archive_index_groups_months_under_year_headings() {
    let months = vec![
        ArchiveMonth {
            year: 2026,
            month: 6,
            count: 4,
        },
        ArchiveMonth {
            year: 2026,
            month: 5,
            count: 2,
        },
        ArchiveMonth {
            year: 2025,
            month: 12,
            count: 7,
        },
    ];
    let html = render_archive_index_page(&months, &site());
    assert!(html.contains("<h2>2026</h2>"), "year 2026 heading present");
    assert!(html.contains("<h2>2025</h2>"), "year 2025 heading present");
    assert!(html.contains("June 2026"), "June 2026 label present");
    assert!(
        html.contains("December 2025"),
        "December 2025 label present"
    );
}

#[test]
fn archive_index_month_links_use_zero_padded_month() {
    let months = vec![ArchiveMonth {
        year: 2026,
        month: 3,
        count: 1,
    }];
    let html = render_archive_index_page(&months, &site());
    assert!(
        html.contains(r#"href="/archive/2026/03""#),
        "month link must use zero-padded two-digit month"
    );
}

#[test]
fn archive_index_shows_document_count_per_month() {
    let months = vec![ArchiveMonth {
        year: 2026,
        month: 1,
        count: 99,
    }];
    let html = render_archive_index_page(&months, &site());
    assert!(html.contains("(99)"), "document count must be visible");
}

#[test]
fn archive_index_empty_renders_gracefully() {
    let html = render_archive_index_page(&[], &site());
    assert!(html.contains(r#"class="empty""#), "empty state must render");
    assert!(!html.contains("<h2>"), "no year headings when no months");
}

// ── Archive month page ────────────────────────────────────────────────────────

#[test]
fn archive_month_page_route_has_canonical_meta_tag() {
    let html = render_archive_month_page(2026, 6, &[], 1, 1, &site());
    assert!(
        html.contains(r#"rel="canonical" href="https://example.com/archive/2026/06""#),
        "first page canonical must not include /page/1"
    );
}

#[test]
fn archive_month_page_2_has_page_canonical() {
    let html = render_archive_month_page(2026, 6, &[doc_summary("a", "A")], 2, 3, &site());
    assert!(
        html.contains(r#"rel="canonical" href="https://example.com/archive/2026/06/page/2""#),
        "page 2 canonical must include /page/2"
    );
}

#[test]
fn archive_month_page_has_back_to_archive_link() {
    let html = render_archive_month_page(2026, 6, &[], 1, 1, &site());
    assert!(
        html.contains(r#"href="/archive""#),
        "month page must link back to /archive"
    );
}

#[test]
fn archive_month_page_pagination_prev_page_1_links_to_base_url() {
    let html = render_archive_month_page(2026, 3, &[doc_summary("a", "A")], 2, 3, &site());
    assert!(
        html.contains(r#"href="/archive/2026/03""#),
        "prev from page 2 must link to base month URL, not /page/1"
    );
}

#[test]
fn archive_month_page_pagination_next_link_present() {
    let html = render_archive_month_page(2026, 3, &[doc_summary("a", "A")], 1, 2, &site());
    assert!(
        html.contains(r#"href="/archive/2026/03/page/2""#),
        "next link must appear when more pages exist"
    );
}

#[test]
fn archive_month_page_no_pager_when_single_page() {
    let html = render_archive_month_page(2026, 3, &[], 1, 1, &site());
    assert!(
        !html.contains(r#"class="pager""#),
        "pager must be omitted when there is only one page"
    );
}

#[test]
fn archive_month_page_empty_state_renders_gracefully() {
    let html = render_archive_month_page(2026, 6, &[], 1, 1, &site());
    assert!(html.contains(r#"class="empty""#));
}

#[test]
fn archive_month_page_uses_zero_padded_month_in_pager_links() {
    let docs = vec![doc_summary("a", "A")];
    let html = render_archive_month_page(2026, 3, &docs, 1, 2, &site());
    assert!(
        html.contains("/archive/2026/03/page/2"),
        "pager links must use zero-padded month"
    );
}

// ── Document page prev/next nav ───────────────────────────────────────────────

#[test]
fn document_page_without_adjacent_docs_emits_no_doc_nav() {
    let document = doc("my-note", "My Note");
    let html = render_document_page(
        &document,
        &[],
        &std::collections::HashSet::new(),
        &site(),
        "nonce",
        None,
        None,
    );
    assert!(
        !html.contains(r#"class="doc-nav""#),
        "doc-nav must be absent when both prev and next are None"
    );
}

#[test]
fn document_page_with_prev_only_renders_doc_nav_with_prev_link() {
    let document = doc("my-note", "My Note");
    let html = render_document_page(
        &document,
        &[],
        &std::collections::HashSet::new(),
        &site(),
        "nonce",
        Some(&adjacent("older-note", "Older Note")),
        None,
    );
    assert!(
        html.contains(r#"class="doc-nav""#),
        "doc-nav must appear when prev is Some"
    );
    assert!(
        html.contains(r#"href="/older-note""#),
        "prev link href must be the prev slug"
    );
    assert!(
        html.contains("Older Note"),
        "prev link must show the prev title"
    );
    assert!(
        html.contains(r#"rel="prev""#),
        "prev link must carry rel=prev"
    );
}

#[test]
fn document_page_with_next_only_renders_doc_nav_with_next_link() {
    let document = doc("my-note", "My Note");
    let html = render_document_page(
        &document,
        &[],
        &std::collections::HashSet::new(),
        &site(),
        "nonce",
        None,
        Some(&adjacent("newer-note", "Newer Note")),
    );
    assert!(
        html.contains(r#"class="doc-nav""#),
        "doc-nav must appear when next is Some"
    );
    assert!(
        html.contains(r#"href="/newer-note""#),
        "next link href must be the next slug"
    );
    assert!(
        html.contains("Newer Note"),
        "next link must show the next title"
    );
    assert!(
        html.contains(r#"rel="next""#),
        "next link must carry rel=next"
    );
}

#[test]
fn document_page_prev_next_titles_are_html_escaped() {
    let document = doc("my-note", "My Note");
    let html = render_document_page(
        &document,
        &[],
        &std::collections::HashSet::new(),
        &site(),
        "nonce",
        Some(&adjacent("prev", "Prev <Title> & Co")),
        Some(&adjacent("next", "Next \"Title\"")),
    );
    assert!(
        html.contains("Prev &lt;Title&gt; &amp; Co"),
        "prev title must be HTML-escaped"
    );
    assert!(
        html.contains("Next &quot;Title&quot;"),
        "next title must be HTML-escaped"
    );
}

#[test]
fn document_page_prev_next_slugs_are_url_encoded() {
    let document = doc("my-note", "My Note");
    let html = render_document_page(
        &document,
        &[],
        &std::collections::HashSet::new(),
        &site(),
        "nonce",
        Some(&adjacent("has space", "Spaced")),
        None,
    );
    assert!(
        html.contains(r#"href="/has%20space""#),
        "prev slug must be URL-encoded"
    );
}

// ── Route shape (contract) ────────────────────────────────────────────────────

// The archive route shape is covered by the inline #[cfg(test)] blocks in
// views/archive.rs and the handler validation in parse_archive_year /
// parse_archive_month. Router-level integration tests that need a live DB are
// tracked under db_requirements.rs. The canonical URL and HTML structure
// assertions above satisfy the CIL-132 acceptance criteria for route behavior.
