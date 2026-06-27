use inkwell::domain::document::{Document, DocumentStatus, GrowthStage};
use inkwell::views::document::render_document_page;
use inkwell::views::index::render_index_page;
use inkwell::views::layout::{HeadMeta, SiteMeta, derive_excerpt, render_page};
use inkwell::views::search::render_search_page;
use inkwell::views::tags::render_tag_page;
use serde_json::json;

/// A published note carrying tags and an excerpt-worthy body — exactly the
/// shape that surfaces the raw-string `\n` regression (visible chips/excerpt
/// separators) in the rendered HTML.
fn tagged_document() -> Document {
    let now = time::OffsetDateTime::now_utc();
    Document {
        id: uuid::Uuid::nil(),
        slug: "hello-world".to_string(),
        title: "Hello World".to_string(),
        body_markdown: "A first note with enough prose to derive a non-empty excerpt.".to_string(),
        rendered_html: "<p>A first note.</p>".to_string(),
        status: DocumentStatus::Published,
        growth: GrowthStage::Seedling,
        tags: vec!["rust".to_string(), "garden".to_string()],
        version: 1,
        created_at: now,
        updated_at: now,
    }
}

#[test]
fn derive_excerpt_truncates_on_char_boundary_without_panicking() {
    // Place a multibyte char straddling byte index 160 so a raw byte slice
    // (&text[..160]) would panic mid-codepoint.
    let body = format!("{}{}", "a".repeat(159), "😀 trailing words here");
    let excerpt = derive_excerpt(&body, 160);
    assert!(excerpt.ends_with('…'));
    assert!(excerpt.len() <= body.len());
}

#[test]
fn derive_excerpt_trims_ascii_on_word_boundary() {
    let body = format!("{} tail", "word ".repeat(40));
    let excerpt = derive_excerpt(&body, 160);
    assert!(excerpt.ends_with('…'));
    // Word-boundary trim: no trailing partial token before the ellipsis.
    assert!(!excerpt.contains("wor…"));
}

fn browser_runtime_src() -> String {
    ["https://cdn.", "tailwind", "css.com"].concat()
}

fn runtime_config_marker() -> String {
    ["tailwind", ".config"].concat()
}

#[test]
fn render_page_emits_valid_html_attributes() {
    let site = SiteMeta::defaults();
    let html = render_page(
        &site,
        HeadMeta {
            title: "Test",
            description: Some("Description"),
            canonical_url: "http://localhost/".to_string(),
            og_type: "website",
            json_ld: None,
            csp_nonce: None,
        },
        "<p>Body</p>",
    );

    assert!(html.contains(r#"<html lang="en">"#));
    assert!(html.contains(r#"<body class="site-body">"#));
    assert!(html.contains(r#"<div class="site-shell">"#));
    assert!(html.contains(r#"<header class="site-header">"#));
    assert!(html.contains(r#"<div class="site-header-inner">"#));
    assert!(html.contains(r#"<a class="site-brand" href="/">"#));
    // The nav and brand now carry a leading inline-SVG glyph, so match the
    // opening tag and the trailing label rather than the bare old markup.
    assert!(html.contains(r#"<a class="site-nav" href="/tags">"#));
    assert!(html.contains(r#"Tags</a>"#));
    assert!(html.contains(r#"<span class="brand-name">Inkwell</span>"#));
    assert!(html.contains(r#"<div class="botanical-band" aria-hidden="true">"#));
    assert!(html.contains(r#"<main class="site-main">"#));
    assert!(html.contains(r#"<footer class="site-footer">Published with Inkwell.</footer>"#));
    assert!(!html.contains(r#"\""#));
}

#[test]
fn render_page_omits_tailwind_browser_build() {
    let browser_runtime_src = browser_runtime_src();
    let runtime_config_marker = runtime_config_marker();
    let site = SiteMeta::defaults();
    let html = render_page(
        &site,
        HeadMeta {
            title: "Test",
            description: None,
            canonical_url: "http://localhost/".to_string(),
            og_type: "website",
            json_ld: None,
            csp_nonce: None,
        },
        "<p>Body</p>",
    );

    assert!(!html.contains(&browser_runtime_src));
    assert!(!html.contains(&runtime_config_marker));
    assert!(!html.contains("<script"));
}

#[test]
fn render_page_allows_json_ld_with_csp_nonce_and_without_tailwind_runtime() {
    let browser_runtime_src = browser_runtime_src();
    let runtime_config_marker = runtime_config_marker();
    let site = SiteMeta::defaults();
    let html = render_page(
        &site,
        HeadMeta {
            title: "Test",
            description: None,
            canonical_url: "http://localhost/".to_string(),
            og_type: "website",
            json_ld: Some(json!({
                "@context": "https://schema.org",
                "@type": "WebSite",
                "name": "Inkwell"
            })),
            csp_nonce: Some("nonce123"),
        },
        "<p>Body</p>",
    );

    assert!(html.contains(r#"<script type="application/ld+json" nonce="nonce123">"#));
    assert!(!html.contains(&browser_runtime_src));
    assert!(!html.contains(&runtime_config_marker));
}

#[test]
fn document_page_with_tags_has_no_literal_backslash_n() {
    // Regression: several view helpers built HTML with a RAW string literal that
    // started with `\n` (two literal chars: backslash + n), which the browser
    // rendered as visible `\n` text between the meta line and the tag chips.
    // The fixture carries tags so the tag-chip template is exercised; this
    // assertion would have FAILED before the fix.
    let document = tagged_document();
    let site = SiteMeta::defaults();
    let html = render_document_page(
        &document,
        &[],
        &std::collections::HashSet::new(),
        &site,
        "nonce123",
        None,
        None,
    );
    assert!(
        html.contains(r#"<ul class="tags">"#),
        "tag chips must render"
    );
    assert!(
        !html.contains("\\n"),
        "rendered HTML must not contain a literal backslash-n"
    );
}

#[test]
fn index_listing_with_tags_has_no_literal_backslash_n() {
    // The shared document list renders excerpts and tag chips for each entry;
    // both came from raw-string `\n` templates. Exercise the listing and assert
    // no literal backslash-n leaks into the rendered output.
    let documents = vec![tagged_document()];
    let site = SiteMeta::defaults();
    let html = render_index_page(&documents, 1, 1, &site);
    assert!(
        html.contains(r#"<p class="excerpt">"#),
        "excerpt must render"
    );
    assert!(
        html.contains(r#"<ul class="tags">"#),
        "tag chips must render"
    );
    assert!(
        !html.contains("\\n"),
        "rendered HTML must not contain a literal backslash-n"
    );
}

#[test]
fn search_results_pager_has_no_literal_backslash_n() {
    // The search pager template was built from a raw-string `\n` literal. It only
    // renders when there is more than one page, so drive `total_pages > 1` to
    // exercise the `<nav class="pager">` path that was fixed.
    let documents = vec![tagged_document()];
    let site = SiteMeta::defaults();
    let html = render_search_page("hello", &documents, 1, 3, &site);
    assert!(
        html.contains(r#"<nav class="pager">"#),
        "search pager must render when total_pages > 1"
    );
    assert!(
        !html.contains("\\n"),
        "rendered HTML must not contain a literal backslash-n"
    );
}

#[test]
fn tag_page_pager_has_no_literal_backslash_n() {
    // The per-tag pager template was likewise built from a raw-string `\n`
    // literal. Drive `total_pages > 1` so the `<nav class="pager">` path renders.
    let documents = vec![tagged_document()];
    let site = SiteMeta::defaults();
    let html = render_tag_page("rust", &documents, 1, 3, &site);
    assert!(
        html.contains(r#"<nav class="pager">"#),
        "tag pager must render when total_pages > 1"
    );
    assert!(
        !html.contains("\\n"),
        "rendered HTML must not contain a literal backslash-n"
    );
}

// ── CIL-131: configurable site metadata ──────────────────────────────────────

#[test]
fn render_page_uses_configured_site_name_in_brand_and_og_site_name() {
    let site = SiteMeta {
        name: "My Garden",
        description: None,
        author: None,
        base_url: "http://localhost".to_string(),
        custom_css_url: None,
    };
    let html = render_page(
        &site,
        HeadMeta {
            title: "A Note — My Garden",
            description: None,
            canonical_url: "http://localhost/a-note".to_string(),
            og_type: "article",
            json_ld: None,
            csp_nonce: None,
        },
        "<p>Body</p>",
    );
    assert!(
        html.contains(r#"<span class="brand-name">My Garden</span>"#),
        "brand name must reflect INKWELL_SITE_TITLE"
    );
    assert!(
        html.contains(r#"og:site_name" content="My Garden""#),
        "og:site_name must reflect INKWELL_SITE_TITLE"
    );
    assert!(
        !html.contains(r#"<span class="brand-name">Inkwell</span>"#),
        "default brand name must not appear when overridden"
    );
    assert!(
        html.contains("Published with My Garden."),
        "footer must use configured site name"
    );
}

#[test]
fn render_page_defaults_to_inkwell_when_no_site_name_configured() {
    let site = SiteMeta::defaults();
    let html = render_page(
        &site,
        HeadMeta {
            title: "Inkwell",
            description: None,
            canonical_url: "http://localhost/".to_string(),
            og_type: "website",
            json_ld: None,
            csp_nonce: None,
        },
        "<p>Body</p>",
    );
    assert!(html.contains(r#"<span class="brand-name">Inkwell</span>"#));
}

#[test]
fn render_page_injects_custom_css_link_when_url_is_set() {
    let site = SiteMeta {
        name: "Inkwell",
        description: None,
        author: None,
        base_url: "http://localhost".to_string(),
        custom_css_url: Some("https://example.com/theme.css"),
    };
    let html = render_page(
        &site,
        HeadMeta {
            title: "Test",
            description: None,
            canonical_url: "http://localhost/".to_string(),
            og_type: "website",
            json_ld: None,
            csp_nonce: None,
        },
        "<p>Body</p>",
    );
    assert!(
        html.contains(r#"<link rel="stylesheet" href="https://example.com/theme.css" />"#),
        "custom CSS URL must be injected as a stylesheet link"
    );
}

#[test]
fn render_page_omits_extra_stylesheet_when_custom_css_url_is_absent() {
    let site = SiteMeta::defaults();
    let html = render_page(
        &site,
        HeadMeta {
            title: "Test",
            description: None,
            canonical_url: "http://localhost/".to_string(),
            og_type: "website",
            json_ld: None,
            csp_nonce: None,
        },
        "<p>Body</p>",
    );
    assert!(
        !html.contains(r#"rel="stylesheet" href="http"#),
        "no external stylesheet must be injected when custom_css_url is None"
    );
}

#[test]
fn render_index_page_uses_configured_description_over_default() {
    let site = SiteMeta {
        name: "My Garden",
        description: Some("Notes on code and life."),
        author: None,
        base_url: "http://localhost".to_string(),
        custom_css_url: None,
    };
    let html = render_index_page(&[], 1, 1, &site);
    assert!(
        html.contains(r#"content="Notes on code and life.""#),
        "configured site description must appear in meta description"
    );
    assert!(
        !html.contains("API-first Markdown"),
        "built-in fallback description must not appear when overridden"
    );
}
