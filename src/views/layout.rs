use crate::config::Config;
use crate::domain::document::DocumentSummary;
use serde_json::json;

/// Fallback brand name used when `INKWELL_SITE_TITLE` is not set.
pub const SITE_NAME: &str = crate::config::DEFAULT_SITE_TITLE;
pub const DEFAULT_SITE_URL: &str = "http://localhost";
pub const PAGE_SIZE: i64 = 10;

/// Site-level metadata threaded through every public HTML page, feed, and
/// JSON-LD block. Construct with [`SiteMeta::from_config`] in handlers; use
/// [`SiteMeta::defaults`] in unit tests that don't need operator overrides.
pub struct SiteMeta<'a> {
    /// Brand/site title (e.g. "My Garden"). Used in the header, `<title>`,
    /// og:site_name, and the feed title.
    pub name: &'a str,
    /// Optional site-level description for the index page and feed subtitle.
    pub description: Option<&'a str>,
    /// Optional default author inserted into JSON-LD and the Atom `<author>`
    /// element when no document-level author is available.
    pub author: Option<&'a str>,
    /// Normalized base URL (no trailing slash). Use instead of calling
    /// [`normalize_site_url`] in each view.
    pub base_url: String,
    /// Optional URL of an extra stylesheet injected via
    /// `<link rel="stylesheet">` after the built-in styles. Allows operators
    /// to apply a custom theme without modifying source code.
    pub custom_css_url: Option<&'a str>,
}

impl<'a> SiteMeta<'a> {
    /// Build from server [`Config`]. The primary constructor for handlers.
    pub fn from_config(config: &'a Config) -> Self {
        Self {
            name: &config.site_title,
            description: config.site_description.as_deref(),
            author: config.site_author.as_deref(),
            base_url: normalize_site_url(config.site_url.as_deref()),
            custom_css_url: config.custom_css_url.as_deref(),
        }
    }

    /// Construct a minimal default instance for unit tests. Uses "Inkwell" as
    /// the name and `http://localhost` as the base URL; no description/author.
    pub fn defaults() -> SiteMeta<'static> {
        SiteMeta {
            name: SITE_NAME,
            description: None,
            author: None,
            base_url: DEFAULT_SITE_URL.to_string(),
            custom_css_url: None,
        }
    }
}

pub struct HeadMeta<'a> {
    pub title: &'a str,
    pub description: Option<&'a str>,
    pub canonical_url: String,
    pub og_type: &'a str,
    pub json_ld: Option<serde_json::Value>,
    pub csp_nonce: Option<&'a str>,
    /// Active nav item key: "dashboard" | "notes" | "tags" | "graph" | "settings"
    pub nav_current: Option<&'a str>,
    /// When true, `.site-main` expands beyond the default 48rem max-width.
    pub wide_layout: bool,
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

pub(crate) fn json_for_script(value: serde_json::Value) -> String {
    value
        .to_string()
        .replace('<', r#"\u003c"#)
        .replace('>', r#"\u003e"#)
        .replace('&', r#"\u0026"#)
}

pub(crate) const STYLES: &str = r#"
  /* Theme: "Botanical Soft" — friendly, organic, rounded. Forest-green
     headings, warm-clay links, sage tag pills, soft rounded backlink cards.
     Font note: prefers Nunito/Quicksand if installed, else falls back to the
     OS rounded sans (SF Rounded on Apple). A bundled web font would make this
     identical cross-platform; tracked as a follow-up. */
  @font-face {
    font-family: "Nunito";
    font-style: normal;
    font-weight: 200 1000;
    font-display: swap;
    src: url(/assets/fonts/nunito.woff2) format("woff2");
  }
  /* This theme is intentionally light-only: the warm cream canvas is the
     identity. (A faithful botanical dark mode can be added later.) */
  :root { color-scheme: light; }
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
  .site-header-inner {
    width: min(100%, 1280px);
    margin: 0 auto;
    padding-left: 1.25rem;
    padding-right: 1.25rem;
  }
  .site-main, .site-footer {
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
  ul.index li { margin: 0; padding: 0.35rem 0; }
  ul.index a.title { color: rgb(47 93 69); font-size: 1.2rem; font-weight: 800; text-decoration: none; }
  ul.index a.title:hover { color: rgb(197 107 71); }
  ul.index .excerpt { margin: 0.35rem 0 0; color: rgb(96 110 100); }
  .backlinks { margin-top: 3rem; }
  .backlinks h2 {
    display: inline-flex; align-items: center; gap: 0.5rem;
    color: rgb(47 93 69); font-size: 1.45rem; margin: 0 0 1.1rem;
  }
  .backlinks h2 .ico { width: 1.05em; height: 1.05em; color: rgb(91 138 104); }
  ul.backlinks-list {
    list-style: none; padding: 0; margin: 0; display: grid; gap: 1rem;
    grid-template-columns: repeat(auto-fit, minmax(19rem, 1fr)); align-items: stretch;
  }
  ul.backlinks-list li {
    margin: 0; padding: 1rem 1.15rem; background: rgb(255 255 255);
    border: 1px solid rgb(226 234 226); border-radius: 1.1rem;
    box-shadow: 0 2px 5px rgb(47 93 69 / 0.05), 0 1px 2px rgb(47 93 69 / 0.04);
  }
  a.backlink { color: rgb(47 93 69); font-weight: 800; text-decoration: none; line-height: 1.3; display: inline-block; }
  a.backlink:hover { color: rgb(197 107 71); }
  .backlink-context { margin: 0.45rem 0 0; color: rgb(96 110 100); font-size: 0.9rem; line-height: 1.5; }
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
  nav.doc-nav {
    display: flex; justify-content: space-between; gap: 1rem;
    margin-top: 3rem; padding-top: 1.5rem;
    border-top: 1px solid rgb(224 232 224 / 0.8);
    font-size: 0.9rem;
  }
  nav.doc-nav a { text-decoration: none; color: rgb(197 107 71); font-weight: 700; }
  nav.doc-nav a:hover { color: rgb(176 90 56); }
  nav.doc-nav .spacer { color: transparent; }
  .doc-nav-prev { text-align: left; flex: 1; }
  .doc-nav-next { text-align: right; flex: 1; }
  .archive-back { font-size: 0.875rem; margin-bottom: 1.5rem; }
  .archive-back a { color: rgb(197 107 71); font-weight: 600; }
  .archive-year { margin-bottom: 2rem; }
  .archive-year h2 { color: rgb(47 93 69); font-size: 1.4rem; margin-bottom: 0.5rem; }
  ul.archive-months { list-style: none; padding: 0; margin: 0; display: grid; gap: 0.4rem; }
  ul.archive-months li { margin: 0; }
  ul.archive-months a {
    color: rgb(197 107 71); text-decoration: none; font-size: 0.95rem; font-weight: 600;
  }
  ul.archive-months a:hover { color: rgb(176 90 56); text-decoration: underline; }
  ul.archive-months .count { color: rgb(120 132 123); font-weight: 400; }
  .empty { color: rgb(120 132 123); font-style: italic; }
  /* Bubbly header: wordmark + nav as rounded pills with a leaf/tag glyph. */
  .site-brand {
    display: inline-flex; align-items: center; gap: 0.5rem;
    padding: 0.4rem 0.9rem 0.4rem 0.75rem; border-radius: 999px;
    background: rgb(255 255 255); border: 1px solid rgb(206 224 208);
    box-shadow: 0 1px 2px rgb(47 93 69 / 0.05);
  }
  .site-brand:hover { color: rgb(47 93 69); border-color: rgb(168 192 170); background: rgb(247 251 247); }
  .site-nav {
    display: inline-flex; align-items: center; gap: 0.4rem;
    padding: 0.4rem 0.85rem; border-radius: 999px;
    background: rgb(255 255 255); border: 1px solid rgb(212 220 213);
  }
  .site-nav:hover { background: rgb(247 251 247); border-color: rgb(168 192 170); }
  .ico { display: inline-block; flex: none; vertical-align: middle; }
  .site-brand .ico { color: rgb(91 138 104); }
  .site-nav .ico { color: rgb(150 162 152); }
  .growth .ico { margin-right: 0.3rem; color: rgb(91 138 104); }
  /* Backlink cards: a circular plant-icon badge beside the title + snippet. */
  ul.backlinks-list li { display: flex; gap: 0.85rem; align-items: flex-start; }
  .backlink-badge {
    flex: none; width: 2.4rem; height: 2.4rem; border-radius: 999px;
    display: grid; place-items: center; background: rgb(234 241 234);
    border: 1px solid rgb(206 224 208); color: rgb(74 122 88);
  }
  .backlink-main { min-width: 0; }
  /* Decorative botanical band painted along the bottom of every page. */
  .botanical-band {
    position: fixed; left: 0; right: 0; bottom: 0; height: clamp(84px, 13vw, 168px);
    pointer-events: none; z-index: 0; overflow: hidden; line-height: 0;
  }
  .botanical-band svg { width: 100%; height: 100%; display: block; }
  .site-header, .site-main, .site-footer { position: relative; z-index: 1; }
  .site-footer { padding-bottom: clamp(5rem, 14vw, 9rem); }
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
  /* Authoring web UI (CYP-42): rounded form controls, status badges, and a
     two-column edit/preview grid that collapses to one column on narrow screens. */
  .btn {
    display: inline-flex; align-items: center; gap: 0.4rem; cursor: pointer;
    padding: 0.55rem 1.05rem; font-size: 0.95rem; font-weight: 700; font-family: inherit;
    border: 1px solid rgb(197 107 71); border-radius: 0.85rem;
    background: rgb(197 107 71); color: #fff; text-decoration: none;
  }
  .btn:hover { background: rgb(176 90 56); border-color: rgb(176 90 56); color: #fff; }
  .btn-secondary { background: #fff; color: rgb(166 87 53); }
  .btn-secondary:hover { background: rgb(247 251 247); color: rgb(166 87 53); }
  .btn-publish { background: rgb(61 110 78); border-color: rgb(61 110 78); }
  .btn-publish:hover { background: rgb(47 93 69); border-color: rgb(47 93 69); }
  .editor-actions { display: flex; gap: 0.75rem; align-items: center; flex-wrap: wrap; margin: 1.25rem 0; }
  .editor-form { display: grid; gap: 0.35rem; }
  .editor-form label { font-weight: 700; color: rgb(47 93 69); margin-top: 0.75rem; }
  .editor-form .hint { font-weight: 400; color: rgb(120 132 123); font-size: 0.85rem; }
  .editor-form input, .editor-form select, .editor-form textarea {
    width: 100%; padding: 0.6rem 0.8rem; font-size: 1rem; font-family: inherit;
    border: 1px solid rgb(206 220 208); border-radius: 0.7rem; background: #fff; color: inherit;
  }
  .editor-form textarea { font-family: ui-monospace, "SFMono-Regular", Menlo, monospace; line-height: 1.55; resize: vertical; }
  .editor-statusline { display: flex; align-items: center; gap: 0.75rem; margin-bottom: 0.5rem; font-size: 0.9rem; color: rgb(96 110 100); }
  .badge {
    display: inline-flex; align-items: center; min-height: 1.5rem; font-size: 0.72rem; font-weight: 700;
    text-transform: lowercase; letter-spacing: 0.02em; padding: 0.2rem 0.6rem; border-radius: 999px;
    color: rgb(120 132 123); background: rgb(238 240 238); border: 1px solid rgb(214 222 214);
  }
  .badge-published { color: rgb(61 110 78); background: rgb(228 240 230); border-color: rgb(198 222 202); }
  .badge-draft { color: rgb(166 87 53); background: rgb(245 235 230); border-color: rgb(228 210 200); }
  table.doc-list { border-collapse: collapse; width: 100%; font-size: 0.95rem; margin-top: 1rem; }
  table.doc-list th, table.doc-list td { border-bottom: 1px solid rgb(224 232 224); padding: 0.6rem 0.7rem; text-align: left; }
  table.doc-list th { color: rgb(47 93 69); font-weight: 800; }
  table.doc-list .row-actions a { color: rgb(197 107 71); font-weight: 600; text-decoration: none; }
  table.doc-list .row-actions a:hover { color: rgb(176 90 56); }
  .editor-grid { display: grid; gap: 2rem; }
  .preview { border-top: 1px solid rgb(224 232 224); padding-top: 1rem; }
  .preview h2 { font-size: 1.2rem; color: rgb(47 93 69); margin-top: 0; }
  .preview-body { min-height: 4rem; }
  @media (min-width: 900px) {
    .editor-grid { grid-template-columns: 1fr 1fr; align-items: start; }
    .preview { border-top: none; border-left: 1px solid rgb(224 232 224); padding-top: 0; padding-left: 2rem; }
  }
  /* Wide layout: tags graph page overrides the default 48rem cap */
  .site-main.wide-layout {
    width: min(100%, 1280px);
  }
  /* Tags page header area */
  .tags-page-header { margin-bottom: 1.5rem; }
  .tags-page-header h1 { margin-bottom: 0.25rem; }
  .accent-dot { color: rgb(120 132 123); font-weight: 400; margin: 0 0.4rem; }
  .accent-title { color: rgb(197 107 71); font-weight: 800; }
  .tags-subtitle { margin: 0; color: rgb(120 132 123); font-size: 0.95rem; }
  /* Tag graph split-panel layout */
  .tag-graph-layout {
    display: flex;
    gap: 2rem;
    align-items: flex-start;
    min-height: 500px;
  }
  .tag-graph-panel {
    flex: 0 0 62%;
    min-width: 0;
  }
  .tag-graph-svg {
    width: 100%;
    height: auto;
    display: block;
  }
  /* SVG node colors */
  .node-center { fill: rgb(47 93 69); cursor: pointer; transition: fill 0.15s; }
  .node-center:hover { fill: rgb(197 107 71); }
  .node-satellite { fill: rgb(155 179 154); cursor: pointer; transition: fill 0.15s; }
  .node-satellite:hover { fill: rgb(197 107 71); }
  .orbit-ring { stroke: rgb(180 200 180); stroke-width: 1.5; fill: none; }
  .tag-node .node-label {
    font-family: inherit;
    font-size: 13px;
    font-weight: 700;
    fill: rgb(255 255 255);
    pointer-events: none;
    text-anchor: middle;
    dominant-baseline: auto;
  }
  .tag-node .node-count {
    font-family: inherit;
    font-size: 10px;
    font-weight: 500;
    fill: rgb(255 255 255);
    pointer-events: none;
    text-anchor: middle;
    dominant-baseline: auto;
    opacity: 0.85;
  }
  .tag-edge { stroke: rgb(168 192 170); stroke-opacity: 0.4; fill: none; }
  /* Sidebar */
  .tag-sidebar-panel {
    flex: 0 0 38%;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 1rem;
  }
  .tag-search-wrapper {
    position: relative;
    display: flex;
    align-items: center;
  }
  .tag-search-wrapper .search-icon {
    position: absolute;
    left: 0.75rem;
    width: 1rem;
    height: 1rem;
    color: rgb(150 162 152);
    pointer-events: none;
    flex: none;
  }
  #tag-filter {
    width: 100%;
    padding: 0.6rem 0.75rem 0.6rem 2.4rem;
    font-size: 0.9rem;
    font-family: inherit;
    border: 1px solid rgb(210 224 212);
    border-radius: 0.75rem;
    background: rgb(255 255 255);
    color: inherit;
  }
  #tag-filter:focus { outline: 2px solid rgb(168 192 170); outline-offset: 1px; border-color: rgb(168 192 170); }
  .tag-list-header {
    display: flex;
    align-items: center;
    gap: 0.6rem;
  }
  .tag-list-title {
    font-size: 0.95rem;
    font-weight: 700;
    color: rgb(54 64 58);
    white-space: nowrap;
  }
  .tag-list-divider {
    display: flex;
    align-items: center;
    gap: 0.3rem;
    flex: 1;
  }
  .divider-line {
    flex: 1;
    height: 1px;
    background: rgb(210 224 212);
  }
  .divider-plant {
    width: 1.1rem;
    height: 1.1rem;
    color: rgb(197 107 71);
    flex: none;
  }
  #tag-sidebar-list {
    list-style: none;
    padding: 0;
    margin: 0;
    max-height: 440px;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
  }
  #tag-sidebar-list li { margin: 0; }
  #tag-sidebar-list a {
    display: flex;
    align-items: center;
    gap: 0.6rem;
    padding: 0.55rem 0.5rem;
    border-radius: 0.5rem;
    text-decoration: none;
    color: rgb(54 64 58);
    font-size: 0.875rem;
    font-weight: 500;
    border-bottom: 1px solid rgb(234 240 234);
  }
  #tag-sidebar-list a:hover { background: rgb(242 247 242); }
  .tag-item-icon {
    width: 1rem;
    height: 1rem;
    color: rgb(120 132 123);
    flex: none;
  }
  #tag-sidebar-list .tag-label { flex: 1; min-width: 0; }
  #tag-sidebar-list .count {
    font-size: 0.78rem;
    font-weight: 600;
    color: rgb(61 110 78);
    background: rgb(228 240 230);
    border: 1px solid rgb(198 222 202);
    border-radius: 999px;
    padding: 0.1rem 0.5rem;
    flex: none;
  }
  .tag-sidebar-footer {
    display: flex;
    align-items: center;
    gap: 0.85rem;
    padding-top: 0.75rem;
    margin-top: auto;
  }
  .pot-icon { width: 3rem; height: 3rem; flex: none; }
  .sidebar-quote {
    font-style: italic;
    color: rgb(197 107 71);
    font-size: 0.9rem;
    line-height: 1.5;
    margin: 0;
  }
  /* Notes index page */
  .notes-page-header { margin-bottom: 1.25rem; }
  .notes-page-header h1 { margin-bottom: 0.25rem; }
  .notes-subtitle { margin: 0; color: rgb(120 132 123); font-size: 0.95rem; }
  .notes-toolbar {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 0.75rem;
    margin-bottom: 1.25rem;
  }
  #notes-filter {
    flex: 1 1 14rem;
    min-width: 0;
    padding: 0.5rem 0.85rem;
    border: 1px solid rgb(199 216 201);
    border-radius: 0.6rem;
    background: rgb(252 253 251);
    font: inherit;
    color: inherit;
  }
  #notes-filter:focus {
    outline: none;
    border-color: rgb(120 160 132);
    box-shadow: 0 0 0 3px rgb(168 192 170 / 0.35);
  }
  .notes-sort { display: flex; gap: 0.25rem; }
  .notes-sort button {
    padding: 0.4rem 0.8rem;
    border: 1px solid rgb(199 216 201);
    border-radius: 0.6rem;
    background: rgb(252 253 251);
    font: inherit;
    font-size: 0.9rem;
    color: rgb(80 96 84);
    cursor: pointer;
    transition: background 0.15s, border-color 0.15s, color 0.15s;
  }
  .notes-sort button:hover { background: rgb(247 251 247); border-color: rgb(168 192 170); }
  .notes-sort button[aria-pressed="true"] {
    background: rgb(47 93 69);
    border-color: rgb(47 93 69);
    color: rgb(255 255 255);
  }
  ul.notes-list { list-style: none; margin: 0; padding: 0; }
  .note-row {
    display: flex;
    flex-wrap: wrap;
    align-items: baseline;
    gap: 0.5rem 0.85rem;
    padding: 0.7rem 0;
    border-bottom: 1px solid rgb(232 240 232);
  }
  .note-row-title {
    font-weight: 700;
    color: rgb(47 93 69);
    text-decoration: none;
    flex: 1 1 16rem;
    min-width: 0;
  }
  .note-row-title:hover { color: rgb(197 107 71); }
  .note-row-meta { color: rgb(120 132 123); font-size: 0.85rem; white-space: nowrap; }
  .notes-truncation { color: rgb(120 132 123); font-size: 0.9rem; font-style: italic; margin: 0 0 1rem; }
  .notes-no-matches { margin-top: 1rem; }
  /* Settings page */
  .settings-page-header { margin-bottom: 1.5rem; }
  .settings-page-header h1 { margin-bottom: 0.25rem; }
  .settings-subtitle { margin: 0; color: rgb(120 132 123); font-size: 0.95rem; }
  .settings-section { margin: 0 0 2rem; }
  .settings-section h2 { font-size: 1.2rem; margin: 0 0 0.75rem; }
  dl.settings-list {
    display: grid;
    grid-template-columns: minmax(7rem, max-content) 1fr;
    gap: 0.45rem 1.25rem;
    margin: 0;
  }
  dl.settings-list dt { color: rgb(120 132 123); font-weight: 700; }
  dl.settings-list dd { margin: 0; word-break: break-word; }
  ul.capabilities { list-style: none; margin: 0; padding: 0; }
  ul.capabilities li {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 1rem;
    padding: 0.55rem 0;
    border-bottom: 1px solid rgb(232 240 232);
  }
  .cap-label { min-width: 0; }
  .cap-state {
    flex: none;
    font-size: 0.8rem;
    font-weight: 700;
    padding: 0.15rem 0.6rem;
    border-radius: 1rem;
    border: 1px solid transparent;
  }
  .cap-on { color: rgb(61 110 78); background: rgb(228 240 230); border-color: rgb(198 222 202); }
  .cap-off { color: rgb(120 132 123); background: rgb(238 240 238); border-color: rgb(220 226 220); }
  .cap-value { color: rgb(80 96 84); background: rgb(238 244 238); border-color: rgb(214 226 215); }
  .settings-stats {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(8rem, 1fr));
    gap: 0.85rem;
  }
  .stat-card {
    display: flex;
    flex-direction: column;
    gap: 0.2rem;
    padding: 1rem;
    border: 1px solid rgb(224 232 224);
    border-radius: 0.85rem;
    background: rgb(252 253 251);
  }
  .stat-value { font-size: 1.6rem; font-weight: 800; color: rgb(47 93 69); line-height: 1.1; }
  .stat-label { color: rgb(120 132 123); font-size: 0.85rem; }
  .account-panel p { margin: 0 0 0.75rem; }
  .account-scopes { display: flex; flex-wrap: wrap; align-items: center; gap: 0.4rem; }
  .scope-chip {
    font-size: 0.8rem;
    font-weight: 700;
    color: rgb(61 84 68);
    background: rgb(234 241 234);
    border: 1px solid rgb(206 224 208);
    padding: 0.1rem 0.55rem;
    border-radius: 1rem;
  }
  .account-noscope { color: rgb(120 132 123); font-style: italic; }
  /* Graph page */
  .graph-page-header { margin-bottom: 1.25rem; }
  .graph-page-header h1 { margin-bottom: 0.25rem; }
  .graph-subtitle { margin: 0; color: rgb(120 132 123); font-size: 0.95rem; }
  /* No-JS fallback: a plain note list, hidden once the script activates. */
  ul.graph-fallback { columns: 2 18rem; gap: 1.25rem; list-style: none; margin: 0; padding: 0; }
  ul.graph-fallback li { margin: 0.3rem 0; break-inside: avoid; }
  ul.graph-fallback a { color: rgb(47 93 69); text-decoration: none; }
  ul.graph-fallback a:hover { color: rgb(197 107 71); }
  .graph-canvas { display: none; }
  .js-graph-active .graph-canvas { display: block; }
  .js-graph-active ul.graph-fallback { display: none; }
  .graph-canvas {
    margin-top: 0.5rem;
    border: 1px solid rgb(224 232 224);
    border-radius: 0.85rem;
    background:
      radial-gradient(circle at 1px 1px, rgb(224 232 224) 1px, transparent 0) 0 0 / 22px 22px,
      rgb(252 253 251);
    overflow: hidden;
  }
  .graph-svg { width: 100%; height: 70vh; min-height: 420px; display: block; cursor: grab; touch-action: none; }
  .graph-svg:active { cursor: grabbing; }
  .graph-edge { stroke: rgb(190 206 191); stroke-width: 1.2; }
  .graph-node circle { fill: rgb(91 138 104); stroke: rgb(252 253 251); stroke-width: 1.5; cursor: pointer; transition: fill 0.15s; }
  .graph-node:hover circle { fill: rgb(197 107 71); }
  .graph-node-label {
    font-family: inherit; font-size: 11px; font-weight: 700; fill: rgb(61 84 68);
    text-anchor: middle; pointer-events: none;
    opacity: 0; transition: opacity 0.15s;
  }
  .graph-node:hover .graph-node-label { opacity: 1; }
  /* Hover focus: dim the rest, surface the node + its neighborhood. */
  .graph-hovering .graph-edge { stroke-opacity: 0.25; }
  .graph-edge--hi { stroke: rgb(197 107 71) !important; stroke-opacity: 0.9 !important; stroke-width: 2; }
  .graph-node--dim { opacity: 0.3; }
  .graph-node--hi circle { fill: rgb(47 93 69); }
  .graph-node--hi .graph-node-label { opacity: 1; }
  /* Full nav header */
  .site-nav-group {
    display: flex;
    align-items: center;
    gap: 0.35rem;
    flex: 1;
    justify-content: center;
  }
  .site-nav--active {
    background: rgb(47 93 69) !important;
    color: rgb(255 255 255) !important;
    border-color: rgb(47 93 69) !important;
  }
  .site-nav--active .ico { color: rgb(200 220 202) !important; }
  .site-nav--active:hover { background: rgb(38 77 57) !important; border-color: rgb(38 77 57) !important; }
  .site-header-end { display: flex; align-items: center; }
  @media (max-width: 639px) {
    .site-nav-group { display: none; }
    .tag-graph-layout { flex-direction: column; }
    .tag-graph-panel, .tag-sidebar-panel { flex: none; width: 100%; }
    #tag-sidebar-list { max-height: 240px; }
  }
"#;

/// Leaf glyph for the wordmark pill.
pub(crate) const LEAF_ICON: &str = r##"<svg class="ico" width="20" height="20" viewBox="0 0 24 24" fill="none" aria-hidden="true"><path d="M5 19c0-7.5 5.2-13.2 14-14 .2 9.3-5.8 14-14 14Z" fill="currentColor"/><path d="M6 18C9.5 13.5 12.5 11 16.5 9" stroke="#fbfaf6" stroke-width="1.4" stroke-linecap="round"/></svg>"##;

/// Tag glyph for the nav pill.
pub(crate) const TAG_ICON: &str = r##"<svg class="ico" width="15" height="15" viewBox="0 0 24 24" fill="none" aria-hidden="true"><path d="M4 4.5h7L20 13l-7 7-9-9V4.5Z" stroke="currentColor" stroke-width="1.7" stroke-linejoin="round"/><circle cx="8" cy="8.5" r="1.5" fill="currentColor"/></svg>"##;

/// Dashboard (house) glyph for the nav pill.
pub(crate) const DASHBOARD_ICON: &str = r##"<svg class="ico" width="15" height="15" viewBox="0 0 24 24" fill="none" aria-hidden="true"><path d="M3 12L12 4l9 8" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round"/><path d="M5 10v8a1 1 0 0 0 1 1h4v-4h4v4h4a1 1 0 0 0 1-1v-8" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round"/></svg>"##;

/// Notes (document) glyph for the nav pill.
pub(crate) const NOTES_ICON: &str = r##"<svg class="ico" width="15" height="15" viewBox="0 0 24 24" fill="none" aria-hidden="true"><rect x="4" y="3" width="16" height="18" rx="2" stroke="currentColor" stroke-width="1.7"/><path d="M8 8h8M8 12h8M8 16h5" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"/></svg>"##;

/// Graph (nodes-connected) glyph for the nav pill.
pub(crate) const GRAPH_ICON: &str = r##"<svg class="ico" width="15" height="15" viewBox="0 0 24 24" fill="none" aria-hidden="true"><circle cx="12" cy="12" r="2.5" fill="currentColor"/><circle cx="5" cy="7" r="2" fill="currentColor"/><circle cx="19" cy="7" r="2" fill="currentColor"/><circle cx="5" cy="17" r="2" fill="currentColor"/><circle cx="19" cy="17" r="2" fill="currentColor"/><path d="M10 11L7 8.5M14 11l2 -1.5M10 13l-2.5 2.5M14 13l2.5 2.5" stroke="currentColor" stroke-width="1.4" stroke-linecap="round"/></svg>"##;

/// Settings (gear) glyph for the nav pill.
pub(crate) const SETTINGS_ICON: &str = r##"<svg class="ico" width="15" height="15" viewBox="0 0 24 24" fill="none" aria-hidden="true"><circle cx="12" cy="12" r="3" stroke="currentColor" stroke-width="1.7"/><path d="M12 2v2M12 20v2M2 12h2M20 12h2M4.93 4.93l1.41 1.41M17.66 17.66l1.41 1.41M17.66 6.34l-1.41 1.41M6.34 17.66l-1.41 1.41" stroke="currentColor" stroke-width="1.7" stroke-linecap="round"/></svg>"##;

/// Small sprout glyph for the growth/maturity chip.
pub(crate) const SPROUT_ICON: &str = r##"<svg class="ico" width="13" height="13" viewBox="0 0 24 24" fill="none" aria-hidden="true"><path d="M12 21v-8" stroke="currentColor" stroke-width="1.9" stroke-linecap="round"/><path d="M12 14c0-3.6-2.7-5.6-6.5-5.6C5.5 12 8.2 14 12 14Z" fill="currentColor"/><path d="M12 12.5c0-3 2.2-4.6 5.8-4.6C17.8 11 15.6 12.5 12 12.5Z" fill="currentColor"/></svg>"##;

/// Plant glyphs cycled across the "Linked from" backlink card badges.
pub(crate) const BADGE_ICONS: [&str; 3] = [
    // leaf
    r##"<svg width="22" height="22" viewBox="0 0 24 24" fill="none" aria-hidden="true"><path d="M5 19c0-7.5 5.2-13.2 14-14 .2 9.3-5.8 14-14 14Z" fill="currentColor"/><path d="M6 18C9.5 13.5 12.5 11 16.5 9" stroke="#eaf1ea" stroke-width="1.4" stroke-linecap="round"/></svg>"##,
    // sprig
    r##"<svg width="22" height="22" viewBox="0 0 24 24" fill="none" aria-hidden="true"><path d="M12 21V5" stroke="currentColor" stroke-width="1.7" stroke-linecap="round"/><path d="M12 13c0-3.2-2.4-5-6-5 0 3.2 2.4 5 6 5Z" fill="currentColor"/><path d="M12 10c0-3 2.2-4.4 5.6-4.4C17.6 8.6 15.4 10 12 10Z" fill="currentColor"/></svg>"##,
    // flower
    r##"<svg width="22" height="22" viewBox="0 0 24 24" fill="none" aria-hidden="true"><g fill="currentColor"><circle cx="12" cy="6.5" r="2.7"/><circle cx="12" cy="17.5" r="2.7"/><circle cx="6.5" cy="12" r="2.7"/><circle cx="17.5" cy="12" r="2.7"/></g><circle cx="12" cy="12" r="2.6" fill="#c56b47"/></svg>"##,
];

/// Decorative botanical band painted full-width along the bottom of every page.
/// Hardcoded sage/clay palette; purely ornamental (`aria-hidden`), anchored to
/// the bottom and cropped on the sides via `xMidYMax slice`.
pub(crate) const BOTANICAL_BAND: &str = r##"<svg viewBox="0 0 1440 168" preserveAspectRatio="xMidYMax slice" fill="none" aria-hidden="true"><g stroke="#7a9a7c" stroke-width="3" stroke-linecap="round"><path d="M120 168V70"/><path d="M360 168V96"/><path d="M620 168V60"/><path d="M880 168V104"/><path d="M1120 168V78"/><path d="M1320 168V110"/></g><g fill="#9cba9c"><path d="M120 96c0-22-15-34-40-34 0 20 15 34 40 34Z"/><path d="M120 78c0-19 14-30 36-30 0 18-14 30-36 30Z"/><path d="M360 118c0-18-12-28-33-28 0 16 12 28 33 28Z"/><path d="M620 92c0-24-16-38-44-38 0 22 16 38 44 38Z"/><path d="M620 70c0-20 15-32 38-32 0 19-15 32-38 32Z"/><path d="M880 128c0-16-11-26-30-26 0 15 11 26 30 26Z"/><path d="M1120 104c0-22-15-34-40-34 0 20 15 34 40 34Z"/><path d="M1120 86c0-18 13-29 35-29 0 17-13 29-35 29Z"/><path d="M1320 132c0-16-11-25-29-25 0 14 11 25 29 25Z"/></g><g fill="#8aab8c"><path d="M120 96c0-20 14-32 38-32 0 19-14 32-38 32Z" opacity=".5"/><path d="M620 92c0-22 15-36 42-36 0 21-15 36-42 36Z" opacity=".5"/><path d="M1120 104c0-20 14-32 38-32 0 19-14 32-38 32Z" opacity=".5"/></g><g><g transform="translate(240 58)"><circle r="7" fill="#c56b47"/><g fill="#e6b7a4"><circle cx="0" cy="-12" r="6"/><circle cx="0" cy="12" r="6"/><circle cx="-12" cy="0" r="6"/><circle cx="12" cy="0" r="6"/></g><circle r="5" fill="#c56b47"/></g><g transform="translate(760 44) scale(.85)"><g fill="#eac24a"><circle cx="0" cy="-12" r="6"/><circle cx="0" cy="12" r="6"/><circle cx="-12" cy="0" r="6"/><circle cx="12" cy="0" r="6"/></g><circle r="5" fill="#c56b47"/></g><g transform="translate(1240 66) scale(.9)"><g fill="#e6b7a4"><circle cx="0" cy="-12" r="6"/><circle cx="0" cy="12" r="6"/><circle cx="-12" cy="0" r="6"/><circle cx="12" cy="0" r="6"/></g><circle r="5" fill="#c56b47"/></g></g></svg>"##;

pub fn render_page(site: &SiteMeta<'_>, meta: HeadMeta<'_>, main: &str) -> String {
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
            escape_html(site.name)
        ),
        format!(r#"<meta property="og:type" content="{}" />"#, meta.og_type),
        format!(
            r#"<meta property="og:site_name" content="{}" />"#,
            escape_html(site.name)
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

    let extra_css = site
        .custom_css_url
        .map(|url| {
            format!(
                r#"
    <link rel="stylesheet" href="{}" />"#,
                escape_html(url)
            )
        })
        .unwrap_or_default();

    let nav_current = meta.nav_current.unwrap_or("");
    let main_class = if meta.wide_layout {
        "site-main wide-layout"
    } else {
        "site-main"
    };

    let nav_item = |key: &str, href: &str, icon: &str, label: &str| {
        let active = if nav_current == key {
            " site-nav--active"
        } else {
            ""
        };
        format!(r#"<a class="site-nav{active}" href="{href}">{icon}{label}</a>"#)
    };

    format!(
        r#"<!doctype html>
<html lang="en">
  <head>
    {}
    <link rel="preload" href="/assets/fonts/nunito.woff2" as="font" type="font/woff2" crossorigin />
    <link rel="stylesheet" href="/assets/site.css" />{}
  </head>
  <body class="site-body">
    <div class="site-shell">
      <header class="site-header">
        <div class="site-header-inner">
          <a class="site-brand" href="/">{}<span class="brand-name">{}</span></a>
          <nav class="site-nav-group" aria-label="Main navigation">
            {}
            {}
            {}
            {}
            {}
          </nav>
          <div class="site-header-end"></div>
        </div>
      </header>
      <main class="{}">
{}
      </main>
      <footer class="site-footer">Published with {}.</footer>
    </div>
    <div class="botanical-band" aria-hidden="true">{}</div>
  </body>
</html>
"#,
        tags.join("\n    "),
        extra_css,
        LEAF_ICON,
        escape_html(site.name),
        nav_item("dashboard", "/", DASHBOARD_ICON, "Dashboard"),
        nav_item("notes", "/notes", NOTES_ICON, "Notes"),
        nav_item("tags", "/tags", TAG_ICON, "Tags"),
        nav_item("graph", "/graph", GRAPH_ICON, "Graph"),
        nav_item("settings", "/settings", SETTINGS_ICON, "Settings"),
        main_class,
        main,
        escape_html(site.name),
        BOTANICAL_BAND
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
pub(crate) fn render_document_list(documents: &[DocumentSummary]) -> String {
    let items = documents
        .iter()
        .map(|doc| {
            let excerpt = derive_excerpt(&doc.body_excerpt_source, 160);
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

#[allow(clippy::too_many_arguments)]
pub fn json_ld_document(
    title: &str,
    description: Option<&str>,
    url: &str,
    created: &str,
    updated: &str,
    tags: &[String],
    site_name: &str,
    author: Option<&str>,
) -> serde_json::Value {
    let mut value = json!({
        "@context": "https://schema.org",
        "@type": "BlogPosting",
        "headline": title,
        "datePublished": created,
        "dateModified": updated,
        "url": url,
        "mainEntityOfPage": { "@type": "WebPage", "@id": url },
        "publisher": { "@type": "Organization", "name": site_name },
        "inLanguage": "en"
    });
    if let Some(description) = description {
        value["description"] = serde_json::Value::String(description.to_string());
    }
    if let Some(author) = author {
        value["author"] = json!({ "@type": "Person", "name": author });
    }
    if !tags.is_empty() {
        value["keywords"] = serde_json::Value::String(tags.join(", "));
    }
    value
}
