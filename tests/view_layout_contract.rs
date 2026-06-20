use inkwell::views::layout::{HeadMeta, render_page};

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
    assert!(html.contains(r#"<div class="min-h-screen "#));
    assert!(html.contains(r#"<main class="mx-auto "#));
    assert!(!html.contains(r#"\""#));
}

#[test]
fn render_page_includes_tailwind_browser_build() {
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

    assert!(html.contains(r#"<script src="https://cdn.tailwindcss.com"></script>"#));
    assert!(html.contains("tailwind.config"));
}
