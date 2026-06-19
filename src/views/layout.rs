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
  body {
    margin: 0;
    font-family: -apple-system, BlinkMacSystemFont, \"Segoe UI\", Roboto, Helvetica, Arial, sans-serif;
    line-height: 1.6;
    color: #1a1a1a;
    background: #fdfdfd;
  }
  .wrap { max-width: 42rem; margin: 0 auto; padding: 3rem 1.25rem 5rem; }
  header.site { margin-bottom: 2.5rem; }
  header.site a.brand { font-weight: 700; font-size: 1.1rem; color: inherit; text-decoration: none; }
  h1, h2, h3, h4, h5, h6 { line-height: 1.25; margin: 2rem 0 0.75rem; }
  h1 { font-size: 2rem; }
  p, ul, ol, blockquote, table, pre, figure { margin: 0 0 1.1rem; }
  a { color: #0b5fff; }
  img { max-width: 100%; height: auto; }
  pre { background: #f4f4f6; padding: 1rem; border-radius: 6px; overflow-x: auto; }
  code { background: #f4f4f6; padding: 0.15em 0.35em; border-radius: 4px; font-size: 0.9em; }
  pre code { background: none; padding: 0; }
  .hljs { color: #1a1a1a; }
  blockquote { border-left: 3px solid #d0d0d8; margin-left: 0; padding-left: 1rem; color: #555; }
  table { border-collapse: collapse; width: 100%; }
  th, td { border: 1px solid #e0e0e6; padding: 0.4rem 0.6rem; text-align: left; }
  .meta { color: #777; font-size: 0.875rem; }
  ul.tags { list-style: none; padding: 0; margin: 0.5rem 0 0; display: flex; flex-wrap: wrap; gap: 0.4rem; }
  ul.tags li { margin: 0; }
  ul.tags a {
    display: inline-block; font-size: 0.8rem; line-height: 1.4; text-decoration: none;
    padding: 0.1rem 0.55rem; border: 1px solid #d0d0d8; border-radius: 999px; color: #555;
  }
  ul.tags a:hover { border-color: #0b5fff; color: #0b5fff; }
  ul.tags .count { color: #999; }
  ul.index { list-style: none; padding: 0; }
  ul.index li { margin: 0 0 1.75rem; }
  ul.index a.title { font-size: 1.15rem; font-weight: 600; text-decoration: none; }
  ul.index a.title:hover { text-decoration: underline; }
  ul.index .excerpt { margin: 0.35rem 0 0; color: #444; }
  form.search { display: flex; gap: 0.5rem; margin: 0 0 2rem; }
  form.search input[type=\"search\"] {
    flex: 1; padding: 0.5rem 0.75rem; font-size: 1rem; border: 1px solid #d0d0d8; border-radius: 6px;
    background: #fff; color: inherit;
  }
  form.search button {
    padding: 0.5rem 1rem; font-size: 1rem; border: 1px solid #0b5fff; border-radius: 6px;
    background: #0b5fff; color: #fff; cursor: pointer;
  }
  nav.pager { display: flex; justify-content: space-between; margin-top: 2.5rem; }
  nav.pager a { text-decoration: none; }
  nav.pager .spacer { color: transparent; }
  .empty { color: #777; font-style: italic; }
  footer.site { margin-top: 4rem; color: #aaa; font-size: 0.8rem; }
"#;

pub fn render_page(meta: HeadMeta<'_>, main: &str) -> String {
    let mut tags = vec![
        r#"<meta charset=\"utf-8\" />"#.to_string(),
        r#"<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\" />"#.to_string(),
        format!(r#"<title>{}</title>"#, escape_html(meta.title)),
        format!(
            r#"<link rel=\"canonical\" href=\"{}\" />"#,
            escape_html(&meta.canonical_url)
        ),
        format!(
            r#"<link rel=\"alternate\" type=\"application/atom+xml\" title=\"{}\" href=\"/feed.xml\" />"#,
            escape_html(SITE_NAME)
        ),
        format!(
            r#"<meta property=\"og:type\" content=\"{}\" />"#,
            meta.og_type
        ),
        format!(
            r#"<meta property=\"og:site_name\" content=\"{}\" />"#,
            escape_html(SITE_NAME)
        ),
        format!(
            r#"<meta property=\"og:title\" content=\"{}\" />"#,
            escape_html(meta.title)
        ),
        format!(
            r#"<meta property=\"og:url\" content=\"{}\" />"#,
            escape_html(&meta.canonical_url)
        ),
        r#"<meta name=\"twitter:card\" content=\"summary\" />"#.to_string(),
        format!(
            r#"<meta name=\"twitter:title\" content=\"{}\" />"#,
            escape_html(meta.title)
        ),
    ];
    if let Some(description) = meta.description {
        tags.push(format!(
            r#"<meta name=\"description\" content=\"{}\" />"#,
            escape_html(description)
        ));
        tags.push(format!(
            r#"<meta property=\"og:description\" content=\"{}\" />"#,
            escape_html(description)
        ));
        tags.push(format!(
            r#"<meta name=\"twitter:description\" content=\"{}\" />"#,
            escape_html(description)
        ));
    }
    if let Some(json_ld) = meta.json_ld {
        tags.push(format!(
            r#"<script type=\"application/ld+json\">{}</script>"#,
            json_for_script(json_ld)
        ));
    }

    format!(
        r#"<!doctype html>
<html lang=\"en\">
  <head>
    {}
    <style>{}</style>
  </head>
  <body>
    <div class=\"wrap\">
      <header class=\"site\"><a class=\"brand\" href=\"/\">{}</a></header>
      <main>
{}
      </main>
      <footer class=\"site\">Published with Inkwell.</footer>
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
    let clipped = &text[..max_length];
    let clipped = clipped
        .rsplit_once(' ')
        .map(|(head, _)| head)
        .unwrap_or(clipped);
    format!("{}…", clipped.trim_end())
}

pub fn date_line(label: &str, timestamp: time::OffsetDateTime) -> String {
    let text = crate::domain::document::timestamp::serialize_to_string(&timestamp);
    format!(
        r#"<time datetime=\"{}\">{} {}</time>"#,
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
                r#"<li><a href=\"/tags/{}\">{}</a></li>"#,
                urlencoding::encode(tag),
                escape_html(tag)
            )
        })
        .collect::<Vec<_>>()
        .join("");
    format!(r#"\n            <ul class=\"tags\">{}</ul>"#, items)
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
