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
  :root { color-scheme: light dark; }
  * { box-sizing: border-box; }
  body { margin: 0; }
  .site-body {
    min-height: 100vh;
    background: rgb(250 250 250);
    color: rgb(9 9 11);
    font-family: Inter, ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
    -webkit-font-smoothing: antialiased;
    -moz-osx-font-smoothing: grayscale;
  }
  .site-shell { min-height: 100vh; border-top: 4px solid rgb(14 165 233); }
  .site-header {
    border-bottom: 1px solid rgb(228 228 231 / 0.8);
    background: rgb(255 255 255 / 0.85);
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
    color: rgb(9 9 11);
    font-size: 1.125rem;
    font-weight: 900;
    letter-spacing: 0;
  }
  .site-brand:hover { color: rgb(2 132 199); }
  .site-nav {
    color: rgb(113 113 122);
    font-size: 0.875rem;
    font-weight: 500;
  }
  .site-nav:hover { color: rgb(2 132 199); }
  .site-main {
    padding-top: 3rem;
    padding-bottom: 4rem;
  }
  .site-footer {
    padding-bottom: 3rem;
    color: rgb(113 113 122);
    font-size: 0.875rem;
  }
  main { line-height: 1.7; }
  main h1, main h2, main h3, main h4, main h5, main h6 {
    color: rgb(24 24 27); font-weight: 750; line-height: 1.15; letter-spacing: 0;
    margin: 2.25rem 0 0.85rem;
  }
  main h1 { font-size: clamp(2rem, 4vw, 3rem); margin-top: 0; }
  main h2 { font-size: 1.65rem; }
  main h3 { font-size: 1.25rem; }
  main p, main ul, main ol, main blockquote, main table, main pre, main figure { margin: 0 0 1.15rem; }
  main ul:not(.index):not(.tags), main ol { padding-left: 1.4rem; }
  main li { margin: 0.25rem 0; }
  main a { color: rgb(2 132 199); text-decoration-thickness: 0.08em; text-underline-offset: 0.18em; }
  main img { max-width: 100%; height: auto; border-radius: 0.5rem; }
  main pre {
    background: rgb(24 24 27); color: rgb(244 244 245); padding: 1rem; border-radius: 0.5rem;
    overflow-x: auto; border: 1px solid rgb(39 39 42);
  }
  main code { background: rgb(244 244 245); color: rgb(63 63 70); padding: 0.15em 0.35em; border-radius: 0.25rem; font-size: 0.9em; }
  main pre code { background: transparent; color: inherit; padding: 0; }
  .hljs { color: inherit; }
  main blockquote { border-left: 4px solid rgb(14 165 233); margin-left: 0; padding-left: 1rem; color: rgb(82 82 91); }
  main table { border-collapse: collapse; width: 100%; font-size: 0.95rem; }
  main th, main td { border: 1px solid rgb(228 228 231); padding: 0.55rem 0.7rem; text-align: left; }
  main th { background: rgb(244 244 245); }
  .meta { color: rgb(113 113 122); font-size: 0.875rem; margin: 0.35rem 0 0.85rem; }
  ul.tags { list-style: none; padding: 0; margin: 0.75rem 0 1.25rem; display: flex; flex-wrap: wrap; gap: 0.5rem; }
  ul.tags li { margin: 0; }
  ul.tags a {
    display: inline-flex; align-items: center; min-height: 1.75rem; font-size: 0.78rem; line-height: 1;
    text-decoration: none; padding: 0.35rem 0.65rem; border: 1px solid rgb(186 230 253);
    border-radius: 999px; color: rgb(3 105 161); background: rgb(240 249 255);
  }
  ul.tags a:hover { border-color: rgb(14 165 233); background: rgb(224 242 254); }
  ul.tags .count { color: rgb(113 113 122); }
  ul.index { list-style: none; padding: 0; display: grid; gap: 1.15rem; }
  ul.index li { margin: 0; padding: 1.1rem 0; border-bottom: 1px solid rgb(228 228 231); }
  ul.index a.title { color: rgb(24 24 27); font-size: 1.2rem; font-weight: 700; text-decoration: none; }
  ul.index a.title:hover { color: rgb(2 132 199); }
  ul.index .excerpt { margin: 0.35rem 0 0; color: rgb(82 82 91); }
  form.search { display: flex; gap: 0.65rem; margin: 0 0 2rem; }
  form.search input[type="search"] {
    flex: 1; min-width: 0; padding: 0.65rem 0.85rem; font-size: 1rem; border: 1px solid rgb(212 212 216);
    border-radius: 0.5rem; background: #fff; color: inherit;
  }
  form.search button {
    padding: 0.65rem 1rem; font-size: 1rem; border: 1px solid rgb(2 132 199); border-radius: 0.5rem;
    background: rgb(2 132 199); color: #fff; cursor: pointer;
  }
  nav.pager { display: flex; justify-content: space-between; gap: 1rem; margin-top: 2.5rem; }
  nav.pager a { text-decoration: none; color: rgb(2 132 199); font-weight: 650; }
  nav.pager .spacer { color: transparent; }
  .empty { color: rgb(113 113 122); font-style: italic; }
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
    .site-body { background: rgb(9 9 11); color: rgb(244 244 245); }
    .site-header {
      border-bottom-color: rgb(39 39 42);
      background: rgb(9 9 11 / 0.8);
    }
    .site-brand { color: rgb(250 250 250); }
    .site-brand:hover, .site-nav:hover { color: rgb(56 189 248); }
    .site-nav, .site-footer { color: rgb(161 161 170); }
    main h1, main h2, main h3, main h4, main h5, main h6, ul.index a.title { color: rgb(250 250 250); }
    main a, nav.pager a, ul.index a.title:hover { color: rgb(56 189 248); }
    main code { background: rgb(39 39 42); color: rgb(228 228 231); }
    main pre { background: rgb(9 9 11); border-color: rgb(63 63 70); }
    main blockquote, ul.index .excerpt { color: rgb(212 212 216); }
    main th, main td, ul.index li { border-color: rgb(63 63 70); }
    main th { background: rgb(39 39 42); }
    .meta, .empty { color: rgb(161 161 170); }
    ul.tags a { color: rgb(125 211 252); background: rgb(8 47 73); border-color: rgb(12 74 110); }
    ul.tags a:hover { background: rgb(7 89 133); border-color: rgb(14 165 233); }
    form.search input[type="search"] { background: rgb(24 24 27); border-color: rgb(63 63 70); }
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
    let mut end = max_length;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    let clipped = &text[..end];
    let clipped = clipped
        .rsplit_once(' ')
        .map(|(head, _)| head)
        .unwrap_or(clipped);
    format!("{}…", clipped.trim_end())
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
                format!(
                    r#"\n            <p class="excerpt">{}</p>"#,
                    escape_html(&excerpt)
                )
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
    format!(r#"\n            <ul class="tags">{}</ul>"#, items)
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
