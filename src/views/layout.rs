use crate::domain::document::Document;
use serde_json::json;

pub const SITE_NAME: &str = "Inkwell";
pub const DEFAULT_SITE_URL: &str = "http://localhost";
pub const PAGE_SIZE: i64 = 10;

pub struct HeadMeta<'a> {
    pub title: &'a str,
    pub description: Option<&'a str>,
    pub canonical_url: String,
    pub og_type: &'a str,
    pub json_ld: Option<serde_json::Value>,
    pub csp_nonce: Option<&'a str>,
}

pub fn normalize_site_url(site_url: Option<&str>) -> String {
    let base = site_url.unwrap_or("").trim();
    let base = if base.is_empty() {
        DEFAULT_SITE_URL
    } else {
        base
    };
    base.trim_end_matches('/').to_string()
}

pub fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

pub fn escape_xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn json_for_script(value: serde_json::Value) -> String {
    value
        .to_string()
        .replace('<', r#"\u003c"#)
        .replace('>', r#"\u003e"#)
        .replace('&', r#"\u0026"#)
}

const STYLES: &str = r#"
  /* Theme: "Botanical Soft" — friendly, organic, rounded. Forest-green
     headings, warm-clay links, sage tag pills, soft rounded backlink cards.
     Font note: prefers Nunito/Quicksand if installed, else falls back to the
     OS rounded sans (SF Rounded on Apple). A bundled web font would make this
     identical cross-platform; tracked as a follow-up. */
  :root { color-scheme: light dark; }
  * { box-sizing: border-box; }
  body { margin: 0; }
  .site-body {
    min-height: 100vh;
    background: rgb(251 250 246);
    color: rgb(54 64 58);
    font-family: "Nunito", "Quicksand", ui-rounded, "SF Pro Rounded", system-ui, -apple-system, "Segoe UI", sans-serif;
    -webkit-font-smoothing: antialiased;
    -moz-osx-font-smoothing: grayscale;
  }
  .site-shell { min-height: 100vh; border-top: 4px solid rgb(168 192 170); }
  .site-header {
    border-bottom: 1px solid rgb(224 232 224 / 0.8);
    background: rgb(251 250 246 / 0.85);
    backdrop-filter: blur(12px);
  }
  .site-header-inner, .site-main, .site-footer {
    width: min(100%, 48rem);
    margin: 0 auto;
    padding-left: 1.25rem;
    padding-right: 1.25rem;
  }
  .site-header-inner {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 1rem;
    padding-top: 1.25rem;
    padding-bottom: 1.25rem;
  }
  .site-brand, .site-nav { text-decoration: none; }
  .site-brand {
    color: rgb(47 93 69);
    font-size: 1.125rem;
    font-weight: 800;
    letter-spacing: 0;
  }
  .site-brand:hover { color: rgb(197 107 71); }
  .site-nav {
    color: rgb(120 132 123);
    font-size: 0.875rem;
    font-weight: 600;
  }
  .site-nav:hover { color: rgb(197 107 71); }
  .site-main {
    padding-top: 3rem;
    padding-bottom: 4rem;
  }
  .site-footer {
    padding-bottom: 3rem;
    color: rgb(120 132 123);
    font-size: 0.875rem;
  }
  main { line-height: 1.75; }
  main h1, main h2, main h3, main h4, main h5, main h6 {
    color: rgb(47 93 69); font-weight: 800; line-height: 1.18; letter-spacing: -0.01em;
    margin: 2.25rem 0 0.85rem;
  }
  main h1 { font-size: clamp(2rem, 4vw, 3rem); margin-top: 0; }
  main h2 { font-size: 1.65rem; }
  main h3 { font-size: 1.25rem; }
  main p, main ul, main ol, main blockquote, main table, main pre, main figure { margin: 0 0 1.15rem; }
  main ul:not(.index):not(.tags):not(.backlinks-list), main ol { padding-left: 1.4rem; }
  main li { margin: 0.25rem 0; }
  main a { color: rgb(197 107 71); text-decoration-thickness: 0.08em; text-underline-offset: 0.18em; }
  main a:hover { color: rgb(176 90 56); }
  main img { max-width: 100%; height: auto; border-radius: 0.85rem; }
  main pre {
    background: rgb(43 53 47); color: rgb(238 242 236); padding: 1rem; border-radius: 0.85rem;
    overflow-x: auto; border: 1px solid rgb(58 70 61);
  }
  main code { background: rgb(234 241 234); color: rgb(61 84 68); padding: 0.15em 0.4em; border-radius: 0.4rem; font-size: 0.9em; }
  main pre code { background: transparent; color: inherit; padding: 0; }
  .hljs { color: inherit; }
  main blockquote { border-left: 4px solid rgb(168 192 170); margin-left: 0; padding-left: 1rem; color: rgb(96 110 100); }
  main table { border-collapse: collapse; width: 100%; font-size: 0.95rem; }
  main th, main td { border: 1px solid rgb(224 232 224); padding: 0.55rem 0.7rem; text-align: left; }
  main th { background: rgb(238 244 238); }
  .meta { color: rgb(120 132 123); font-size: 0.875rem; margin: 0.35rem 0 0.85rem; }
  .growth {
    display: inline-flex; align-items: center; min-height: 1.5rem; font-size: 0.72rem; font-weight: 700;
    text-transform: lowercase; letter-spacing: 0.02em; padding: 0.2rem 0.6rem; border-radius: 999px;
    color: rgb(61 110 78); background: rgb(228 240 230); border: 1px solid rgb(198 222 202);
  }
  ul.tags { list-style: none; padding: 0; margin: 0.75rem 0 1.25rem; display: flex; flex-wrap: wrap; gap: 0.5rem; }
  ul.tags li { margin: 0; }
  ul.tags a {
    display: inline-flex; align-items: center; min-height: 1.75rem; font-size: 0.78rem; line-height: 1; font-weight: 600;
    text-decoration: none; padding: 0.35rem 0.7rem; border: 1px solid rgb(201 219 203);
    border-radius: 999px; color: rgb(166 87 53); background: rgb(234 241 234);
  }
  ul.tags a:hover { border-color: rgb(168 192 170); background: rgb(224 235 225); }
  ul.tags .count { color: rgb(120 132 123); }
  ul.index { list-style: none; padding: 0; display: grid; gap: 1.15rem; }
  ul.index li { margin: 0; padding: 1.1rem 0; border-bottom: 1px solid rgb(224 232 224); }
  ul.index a.title { color: rgb(47 93 69); font-size: 1.2rem; font-weight: 800; text-decoration: none; }
  ul.index a.title:hover { color: rgb(197 107 71); }
  ul.index .excerpt { margin: 0.35rem 0 0; color: rgb(96 110 100); }
  .backlinks { margin-top: 3rem; }
  .backlinks h2 { color: rgb(47 93 69); font-size: 1.35rem; margin: 0 0 1rem; }
  ul.backlinks-list { list-style: none; padding: 0; margin: 0; display: grid; gap: 0.85rem; }
  ul.backlinks-list li {
    margin: 0; padding: 1rem 1.1rem; background: rgb(255 255 255);
    border: 1px solid rgb(226 234 226); border-radius: 1rem;
    box-shadow: 0 1px 3px rgb(47 93 69 / 0.06), 0 1px 2px rgb(47 93 69 / 0.04);
  }
  a.backlink { color: rgb(47 93 69); font-weight: 800; text-decoration: none; }
  a.backlink:hover { color: rgb(197 107 71); }
  .backlink-context { margin: 0.35rem 0 0; color: rgb(96 110 100); font-size: 0.92rem; line-height: 1.55; }
  .backlink-context a { color: rgb(197 107 71); }
  .backlink-context a.stub { color: rgb(150 120 104); text-decoration-style: dotted; }
  form.search { display: flex; gap: 0.65rem; margin: 0 0 2rem; }
  form.search input[type="search"] {
    flex: 1; min-width: 0; padding: 0.65rem 0.85rem; font-size: 1rem; border: 1px solid rgb(206 220 208);
    border-radius: 0.85rem; background: #fff; color: inherit;
  }
  form.search button {
    padding: 0.65rem 1.1rem; font-size: 1rem; font-weight: 700; border: 1px solid rgb(197 107 71); border-radius: 0.85rem;
    background: rgb(197 107 71); color: #fff; cursor: pointer;
  }
  form.search button:hover { background: rgb(176 90 56); border-color: rgb(176 90 56); }
  nav.pager { display: flex; justify-content: space-between; gap: 1rem; margin-top: 2.5rem; }
  nav.pager a { text-decoration: none; color: rgb(197 107 71); font-weight: 700; }
  nav.pager .spacer { color: transparent; }
  .empty { color: rgb(120 132 123); font-style: italic; }
  @media (min-width: 640px) {
    .site-header-inner, .site-main, .site-footer {
      padding-left: 1.5rem;
      padding-right: 1.5rem;
    }
    .site-main {
      padding-top: 4rem;
      padding-bottom: 4rem;
    }
  }
  @media (prefers-color-scheme: dark) {
    .site-body { background: rgb(20 26 22); color: rgb(220 228 219); }
    .site-shell { border-top-color: rgb(78 112 86); }
    .site-header {
      border-bottom-color: rgb(40 50 42);
      background: rgb(20 26 22 / 0.8);
    }
    .site-brand { color: rgb(150 195 162); }
    .site-brand:hover, .site-nav:hover { color: rgb(222 150 116); }
    .site-nav, .site-footer { color: rgb(140 156 143); }
    main h1, main h2, main h3, main h4, main h5, main h6, ul.index a.title, .backlinks h2 { color: rgb(150 195 162); }
    main a, nav.pager a, ul.index a.title:hover, a.backlink:hover, .backlink-context a { color: rgb(222 150 116); }
    a.backlink { color: rgb(150 195 162); }
    main code { background: rgb(34 44 37); color: rgb(206 220 208); }
    main pre { background: rgb(15 20 17); border-color: rgb(48 60 51); }
    main blockquote, ul.index .excerpt, .backlink-context { color: rgb(180 196 183); }
    main blockquote { border-left-color: rgb(78 112 86); }
    main th, main td, ul.index li { border-color: rgb(48 60 51); }
    main th { background: rgb(30 40 33); }
    .meta, .empty { color: rgb(140 156 143); }
    .growth { color: rgb(160 205 172); background: rgb(28 44 33); border-color: rgb(46 70 52); }
    ul.tags a { color: rgb(226 168 140); background: rgb(34 46 38); border-color: rgb(52 72 57); }
    ul.tags a:hover { background: rgb(42 56 46); border-color: rgb(78 112 86); }
    ul.backlinks-list li { background: rgb(28 36 30); border-color: rgb(46 58 49); box-shadow: none; }
    .backlink-context a.stub { color: rgb(176 150 134); }
    form.search input[type="search"] { background: rgb(28 36 30); border-color: rgb(48 60 51); }
  }
"#;

pub fn render_page(meta: HeadMeta<'_>, main: &str) -> String {
    let mut tags = vec![
        r#"<meta charset="utf-8" />"#.to_string(),
        r#"<meta name="viewport" content="width=device-width, initial-scale=1" />"#.to_string(),
        format!(r#"<title>{}</title>"#, escape_html(meta.title)),
        format!(
            r#"<link rel="canonical" href="{}" />"#,
            escape_html(&meta.canonical_url)
        ),
        format!(
            r#"<link rel="alternate" type="application/atom+xml" title="{}" href="/feed.xml" />"#,
            escape_html(SITE_NAME)
        ),
        format!(r#"<meta property="og:type" content="{}" />"#, meta.og_type),
        format!(
            r#"<meta property="og:site_name" content="{}" />"#,
            escape_html(SITE_NAME)
        ),
        format!(
            r#"<meta property="og:title" content="{}" />"#,
            escape_html(meta.title)
        ),
        format!(
            r#"<meta property="og:url" content="{}" />"#,
            escape_html(&meta.canonical_url)
        ),
        r#"<meta name="twitter:card" content="summary" />"#.to_string(),
        format!(
            r#"<meta name="twitter:title" content="{}" />"#,
            escape_html(meta.title)
        ),
    ];
    if let Some(description) = meta.description {
        tags.push(format!(
            r#"<meta name="description" content="{}" />"#,
            escape_html(description)
        ));
        tags.push(format!(
            r#"<meta property="og:description" content="{}" />"#,
            escape_html(description)
        ));
        tags.push(format!(
            r#"<meta name="twitter:description" content="{}" />"#,
            escape_html(description)
        ));
    }
    if let Some(json_ld) = meta.json_ld {
        let nonce_attr = meta
            .csp_nonce
            .map(|value| format!(r#" nonce="{}""#, escape_html(value)))
            .unwrap_or_default();
        tags.push(format!(
            r#"<script type="application/ld+json"{}>{}</script>"#,
            nonce_attr,
            json_for_script(json_ld)
        ));
    }

    format!(
        r#"<!doctype html>
<html lang="en">
  <head>
    {}
    <style>{}</style>
  </head>
  <body class="site-body">
    <div class="site-shell">
      <header class="site-header">
        <div class="site-header-inner">
          <a class="site-brand" href="/">{}</a>
          <a class="site-nav" href="/tags">Tags</a>
        </div>
      </header>
      <main class="site-main">
{}
      </main>
      <footer class="site-footer">Published with Inkwell.</footer>
    </div>
  </body>
</html>
"#,
        tags.join("\n    "),
        STYLES,
        escape_html(SITE_NAME),
        main
    )
}

pub fn derive_excerpt(markdown: &str, max_length: usize) -> String {
    let stripped = markdown
        .replace("```", " ")
        .replace('`', "")
        .replace("**", "")
        .replace("__", "")
        .replace(['*', '_', '~'], "");
    let text = stripped
        .lines()
        .map(|line| line.trim_start_matches('#').trim())
        .collect::<Vec<_>>()
        .join(" ");
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.len() <= max_length {
        return text;
    }
    let clipped = truncate_on_char_boundary(&text, max_length);
    let clipped = clipped
        .rsplit_once(' ')
        .map(|(head, _)| head)
        .unwrap_or(clipped);
    format!("{}…", clipped.trim_end())
}

/// Truncate `text` to at most `max_length` bytes without ever splitting a
/// multibyte UTF-8 char: walk the boundary down until it lands on a char start
/// (the 25908c8 fix). NEVER byte-slice a `&str` at an arbitrary index. Returns
/// the whole string when it already fits.
pub fn truncate_on_char_boundary(text: &str, max_length: usize) -> &str {
    if text.len() <= max_length {
        return text;
    }
    let mut end = max_length;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

/// Render the shared `<ul class="index">` document list used by the index,
/// tag, and search views. Callers gate the empty state themselves.
pub(crate) fn render_document_list(documents: &[Document]) -> String {
    let items = documents
        .iter()
        .map(|doc| {
            let excerpt = derive_excerpt(doc.body_markdown(), 160);
            let excerpt_html = if excerpt.is_empty() {
                String::new()
            } else {
                format!(r#"<p class="excerpt">{}</p>"#, escape_html(&excerpt))
            };
            format!(
                r#"          <li>
            <a class="title" href="/{}">{}</a>
            <div class="meta">{}</div>{}{}
          </li>"#,
                urlencoding::encode(&doc.slug),
                escape_html(&doc.title),
                date_line("Published", doc.created_at),
                excerpt_html,
                render_tag_chips(&doc.tags)
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        r#"<ul class="index">
{}
        </ul>"#,
        items
    )
}

pub fn date_line(label: &str, timestamp: time::OffsetDateTime) -> String {
    let text = crate::domain::document::timestamp::serialize_to_string(&timestamp);
    format!(
        r#"<time datetime="{}">{} {}</time>"#,
        text,
        label,
        &text[..10]
    )
}

pub fn render_tag_chips(tags: &[String]) -> String {
    if tags.is_empty() {
        return String::new();
    }
    let items = tags
        .iter()
        .map(|tag| {
            format!(
                r#"<li><a href="/tags/{}">{}</a></li>"#,
                urlencoding::encode(tag),
                escape_html(tag)
            )
        })
        .collect::<Vec<_>>()
        .join("");
    format!(r#"<ul class="tags">{}</ul>"#, items)
}

pub fn json_ld_document(
    title: &str,
    description: Option<&str>,
    url: &str,
    created: &str,
    updated: &str,
    tags: &[String],
) -> serde_json::Value {
    let mut value = json!({
        "@context": "https://schema.org",
        "@type": "BlogPosting",
        "headline": title,
        "datePublished": created,
        "dateModified": updated,
        "url": url,
        "mainEntityOfPage": { "@type": "WebPage", "@id": url },
        "publisher": { "@type": "Organization", "name": SITE_NAME },
        "inLanguage": "en"
    });
    if let Some(description) = description {
        value["description"] = serde_json::Value::String(description.to_string());
    }
    if !tags.is_empty() {
        value["keywords"] = serde_json::Value::String(tags.join(", "));
    }
    value
}
