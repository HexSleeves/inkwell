use inkwell::views::layout::{HeadMeta, derive_excerpt, render_page};
use serde_json::json;

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
    let html = render_page(
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
    assert!(html.contains(r#"<a class="site-nav" href="/tags">Tags</a>"#));
    assert!(html.contains(r#"<main class="site-main">"#));
    assert!(html.contains(r#"<footer class="site-footer">Published with Inkwell.</footer>"#));
    assert!(!html.contains(r#"\""#));
}

#[test]
fn render_page_omits_tailwind_browser_build() {
    let browser_runtime_src = browser_runtime_src();
    let runtime_config_marker = runtime_config_marker();
    let html = render_page(
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
    let html = render_page(
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
