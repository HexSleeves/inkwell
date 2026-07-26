//! Contract for the public-page theming seam (CYP-56).
//!
//! The golden files in `testdata/theme/` were captured from the built-in
//! renderer *before* the theming seam existed. They are the AC-1 guarantee: with
//! `INKWELL_THEME_DIR` unset, public HTML is byte-identical to what Inkwell
//! shipped before themes.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use inkwell::db::links::Backlink;
use inkwell::domain::document::{
    AdjacentDoc, Document, DocumentStatus, DocumentSummary, GrowthStage,
};
use inkwell::views::document::render_document_page;
use inkwell::views::index::render_index_page;
use inkwell::views::layout::SiteMeta;
use inkwell::views::theme::Theme;
use pretty_assertions::assert_eq;

fn stamp(secs: i64) -> time::OffsetDateTime {
    time::OffsetDateTime::from_unix_timestamp(secs).unwrap()
}

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("testdata/themes")
        .join(name)
}

fn load(name: &str) -> Theme {
    Theme::load(fixture(name)).unwrap_or_else(|err| panic!("theme `{name}` must load: {err:#}"))
}

fn site<'a>(theme: Option<&'a Theme>) -> SiteMeta<'a> {
    SiteMeta {
        name: "My Garden",
        description: Some("Notes on code and plants."),
        author: Some("Alice"),
        base_url: "https://example.com".to_string(),
        custom_css_url: None,
        theme,
    }
}

fn summary(slug: &str, title: &str) -> DocumentSummary {
    DocumentSummary {
        id: uuid::Uuid::nil(),
        slug: slug.to_string(),
        title: title.to_string(),
        body_excerpt_source: "Some body text for the excerpt.".to_string(),
        tags: vec!["rust".to_string(), "notes".to_string()],
        growth: GrowthStage::Budding,
        status: DocumentStatus::Published,
        created_at: stamp(1_700_000_000),
        updated_at: stamp(1_700_000_000),
    }
}

fn document() -> Document {
    Document {
        id: uuid::Uuid::nil(),
        slug: "first".to_string(),
        title: "First Note".to_string(),
        body_markdown: "Hello *world*.".to_string(),
        rendered_html: "<p>Hello <em>world</em>.</p>".to_string(),
        status: DocumentStatus::Published,
        growth: GrowthStage::Evergreen,
        tags: vec!["rust".to_string()],
        version: 3,
        created_at: stamp(1_700_000_000),
        updated_at: stamp(1_700_086_400),
    }
}

fn index_html(theme: Option<&Theme>) -> String {
    render_index_page(
        &[summary("first", "First Note"), summary("second", "Second")],
        2,
        3,
        &site(theme),
    )
}

fn document_html(theme: Option<&Theme>) -> String {
    let backlinks = vec![Backlink {
        source_slug: "second".to_string(),
        source_title: "Second".to_string(),
        context_snippet: Some("links to [[first]] here".to_string()),
    }];
    let resolved: HashSet<String> = ["first".to_string()].into_iter().collect();
    render_document_page(
        &document(),
        &backlinks,
        &resolved,
        &site(theme),
        "test-nonce",
        Some(&AdjacentDoc {
            slug: "prev".to_string(),
            title: "Prev".to_string(),
        }),
        Some(&AdjacentDoc {
            slug: "next".to_string(),
            title: "Next".to_string(),
        }),
    )
}

fn golden(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("testdata/theme")
        .join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|err| panic!("{}: {err}", path.display()))
}

// ── AC 1: no theme ⇒ byte-identical to the pre-theming built-in output ────────

#[test]
fn without_a_theme_public_pages_are_byte_identical_to_the_builtin_golden() {
    assert_eq!(
        index_html(None),
        golden("golden-index.html"),
        "index HTML must not drift when no theme is configured"
    );
    assert_eq!(
        document_html(None),
        golden("golden-document.html"),
        "document HTML must not drift when no theme is configured"
    );
}

// ── AC 2: partial themes fall back per slot ──────────────────────────────────

#[test]
fn a_css_only_theme_leaves_every_html_slot_untouched() {
    let theme = load("css-only");
    assert_eq!(
        index_html(Some(&theme)),
        golden("golden-index.html"),
        "a theme that only ships extra.css must not change any HTML"
    );
    assert_eq!(document_html(Some(&theme)), golden("golden-document.html"));
}

#[test]
fn extra_css_is_appended_to_the_builtin_stylesheet() {
    let theme = load("css-only");
    let css = theme.stylesheet("BUILTIN");
    assert!(
        css.starts_with("BUILTIN"),
        "built-in CSS comes first: {css}"
    );
    assert!(css.contains("font-size: 18px"), "extra.css is appended");
}

#[test]
fn styles_css_replaces_the_builtin_stylesheet_and_extra_css_still_appends() {
    let theme = load("css-replace");
    let css = theme.stylesheet("BUILTIN");
    assert!(
        !css.contains("BUILTIN"),
        "styles.css replaces the built-in sheet outright: {css}"
    );
    assert!(css.contains("background: #111"));
    assert!(css.contains("max-width: 60rem"), "extra.css still appends");
}

#[test]
fn a_nav_only_theme_replaces_the_nav_but_keeps_the_builtin_header_and_footer() {
    let theme = load("nav-only");
    let html = index_html(Some(&theme));
    assert!(
        html.contains(r#"<nav class="tiny-nav""#),
        "theme nav is used"
    );
    assert!(
        !html.contains(r#"class="site-nav-group""#),
        "built-in nav is gone"
    );
    // Header/footer/shell/body all still built-in.
    assert!(html.contains(r#"<header class="site-header">"#));
    assert!(html.contains(r#"<a class="site-brand" href="/">"#));
    assert!(html.contains("Published with My Garden."));
    assert!(html.contains(r#"<ul class="index">"#));
    // The active-nav marker still tracks the current page.
    assert!(
        html.contains(r#"class="tiny-link site-nav--active" href="/""#),
        "the index page marks the dashboard link active: {html}"
    );
    assert!(
        html.contains(r#"class="tiny-link" href="/notes""#),
        "inactive links get an empty active class"
    );
}

// ── AC 2: full override ──────────────────────────────────────────────────────

#[test]
fn a_full_theme_replaces_the_shell_and_both_page_bodies() {
    let theme = load("full");
    let html = index_html(Some(&theme));
    assert!(html.starts_with("<!doctype html>"));
    assert!(html.contains(r#"<body class="paper">"#));
    assert!(html.contains(r#"<header class="paper-header">"#));
    assert!(html.contains(r#"<nav class="paper-nav">"#));
    assert!(html.contains("My Garden &mdash; themed"));
    assert!(html.contains(r#"<h1 class="paper-title">My Garden</h1>"#));
    assert!(html.contains("Notes on code and plants."));
    assert!(html.contains("page 2 of 3"));
    // Built-in chrome is fully gone.
    assert!(!html.contains(r#"class="site-shell""#));
    assert!(!html.contains("botanical-band"));
    assert!(!html.contains("Published with My Garden."));
    // The document list itself is still Inkwell-rendered and escaped.
    assert!(html.contains(r#"<a class="title" href="/first">First Note</a>"#));
    // head.html is additive: theme tags AND the canonical/OpenGraph tags.
    assert!(html.contains(r##"<meta name="theme-color" content="#101010" />"##));
    assert!(html.contains(r#"rel="canonical" href="https://example.com/page/2""#));
    assert!(html.contains(r#"property="og:site_name" content="My Garden""#));

    let doc = document_html(Some(&theme));
    assert!(doc.contains(r#"<article class="paper-doc" data-slug="first">"#));
    assert!(doc.contains("<p>Hello <em>world</em>.</p>"), "body_html");
    assert!(doc.contains(r#"<div class="meta">"#), "meta_line");
    assert!(doc.contains(r#"<ul class="tags">"#), "tags");
    assert!(doc.contains(r#"class="backlinks""#), "backlinks");
    assert!(doc.contains(r#"class="doc-nav""#), "doc_nav");
    // JSON-LD still emitted by Inkwell, not the theme.
    assert!(doc.contains(r#"type="application/ld+json""#));
}

// ── Scope: authenticated/admin surfaces are never themed ────────────────────

#[test]
fn admin_surfaces_keep_the_builtin_look_even_with_a_full_theme_configured() {
    let theme = load("full");
    // `/login` is representative: it takes the same SiteMeta (theme attached)
    // yet must render built-in chrome, so a broken theme can never lock an
    // operator out of the authoring UI.
    let login = inkwell::views::login::render_login_page(&site(Some(&theme)), Some("n"), false);
    assert!(
        login.contains(r#"<div class="site-shell">"#),
        "login page must keep the built-in shell: {login}"
    );
    assert!(!login.contains(r#"<body class="paper">"#));
    assert!(login.contains("Published with My Garden."));

    // The 404 page likewise.
    let not_found = inkwell::views::document::render_not_found_page(Some("https://example.com"));
    assert!(not_found.contains(r#"<div class="site-shell">"#));
    assert!(!not_found.contains(r#"<body class="paper">"#));
}

// ── AC 3: a theme is not an escape hatch around sanitization ─────────────────

#[test]
fn theme_document_body_is_the_sanitized_html_not_raw_markdown() {
    // `rendered_html` is what the sanitizer produced; a theme only ever sees
    // that field. Prove the raw markdown does not leak into a themed page.
    let theme = load("full");
    let mut doc = document();
    doc.body_markdown = "<script>alert(1)</script> raw-markdown-marker".to_string();
    doc.rendered_html = "<p>clean</p>".to_string();

    let html = render_document_page(
        &doc,
        &[],
        &HashSet::new(),
        &site(Some(&theme)),
        "nonce",
        None,
        None,
    );
    assert!(html.contains("<p>clean</p>"));
    assert!(
        !html.contains("<script>alert(1)</script>"),
        "unsanitized markdown must never reach a themed page as live markup: {html}"
    );
    // The themed body region gets `rendered_html` and nothing else. (The raw
    // markdown still feeds the escaped `<meta name="description">` excerpt, as
    // it does without a theme — that is Inkwell's own escaping, not a slot.)
    let article = html
        .split_once(r#"<article class="paper-doc""#)
        .and_then(|(_, rest)| rest.split_once("</article>"))
        .map(|(body, _)| body)
        .expect("themed article must be present");
    assert!(
        !article.contains("raw-markdown-marker"),
        "no document slot exposes the raw body markdown: {article}"
    );
}

#[test]
fn theme_slots_do_not_double_escape_or_re_escape_document_values() {
    let theme = load("full");
    let mut doc = document();
    doc.title = "Tom & <Jerry>".to_string();
    let html = render_document_page(
        &doc,
        &[],
        &HashSet::new(),
        &site(Some(&theme)),
        "nonce",
        None,
        None,
    );
    assert!(
        html.contains("<h1>Tom &amp; &lt;Jerry&gt;</h1>"),
        "titles reach themes already escaped, exactly once: {html}"
    );
}

// ── AC 4: malformed themes fail loudly at load time ─────────────────────────

fn load_err(name: &str) -> String {
    format!("{:#}", Theme::load(fixture(name)).unwrap_err())
}

#[test]
fn a_missing_theme_directory_is_an_error() {
    let err = load_err("does-not-exist");
    assert!(
        err.contains("cannot read theme directory"),
        "unexpected error: {err}"
    );
}

#[test]
fn a_theme_file_that_is_not_a_recognized_slot_is_an_error() {
    let err = load_err("bad-unknown-file");
    assert!(
        err.contains("styels.css"),
        "names the offending file: {err}"
    );
    assert!(
        err.contains("styles.css"),
        "lists the recognized slots so the typo is obvious: {err}"
    );
}

#[test]
fn an_unknown_template_variable_is_an_error_that_lists_what_is_available() {
    let err = load_err("bad-unknown-var");
    assert!(err.contains("author_bio"), "{err}");
    assert!(err.contains("unknown variable"), "{err}");
    assert!(err.contains("documents"), "{err}");
}

#[test]
fn an_unterminated_placeholder_is_an_error() {
    let err = load_err("bad-unterminated");
    assert!(err.contains("unterminated"), "{err}");
}

#[test]
fn a_theme_directory_with_no_slot_files_is_an_error() {
    let err = load_err("bad-no-slots");
    assert!(err.contains("no recognized slot files"), "{err}");
}

#[test]
fn a_subdirectory_in_a_theme_is_an_error() {
    let err = load_err("bad-subdir");
    assert!(err.contains("subdirectory"), "{err}");
    assert!(err.contains("partials"), "{err}");
}

// ── AC 5: the shipped example theme is a valid theme ────────────────────────

#[test]
fn the_shipped_example_theme_loads_and_renders() {
    let theme =
        Theme::load(Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/themes/plainpaper"))
            .expect("the example theme must load");
    let html = index_html(Some(&theme));
    assert!(html.starts_with("<!doctype html>"));
    assert!(html.contains("plainpaper"), "example theme markup is used");
    assert!(
        html.contains(r#"<a class="title" href="/first">First Note</a>"#),
        "the example theme still renders Inkwell's document list"
    );
    let doc = document_html(Some(&theme));
    assert!(doc.contains("<p>Hello <em>world</em>.</p>"));
}
