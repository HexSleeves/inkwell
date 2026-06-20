use inkwell::views::layout::{HeadMeta, render_page};
use serde_json::json;

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
        },
        "<p>Body</p>",
    );

    assert!(!html.contains(&browser_runtime_src));
    assert!(!html.contains(&runtime_config_marker));
    assert!(!html.contains("<script"));
}

#[test]
fn render_page_allows_json_ld_without_tailwind_runtime() {
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
        },
        "<p>Body</p>",
    );

    assert!(html.contains(r#"<script type="application/ld+json">"#));
    assert!(!html.contains(&browser_runtime_src));
    assert!(!html.contains(&runtime_config_marker));
}
