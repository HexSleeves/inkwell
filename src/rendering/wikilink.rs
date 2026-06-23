//! Wikilink + embed parsing over the Comrak AST.
//!
//! One walker recognizes both `[[note]]` (wikilink) and `![[note]]` (embed,
//! enabled in a later phase). It operates on the parsed AST and only touches
//! `Text` nodes, so `[[x]]` written inside an inline code span or a fenced
//! code block is never rewritten (Comrak stores code content as the literal of
//! a `Code`/`CodeBlock` node, not as child `Text` nodes).
//!
//! Targets are normalized with [`slugify`] — the same function that generates
//! document slugs — so `[[My Note]]`, `[[my-note]]`, and `[[MY NOTE]]` all
//! resolve to the slug `my-note`.
//!
//! Two entry points:
//!   - [`extract_wikilinks`] — pure; returns the references in a document so the
//!     write path can batch-resolve them in one query.
//!   - [`render_markdown_with_links`] — renders to sanitized HTML, rewriting
//!     resolved wikilinks to `<a href="/{slug}">` and unresolved ones to a
//!     `<a class="stub" …>` (so a future create/rename can light them up).

use std::collections::{HashMap, HashSet};

use comrak::nodes::{AstNode, NodeValue};
use comrak::{Arena, Options, format_html, parse_document};

use crate::domain::slug::slugify;

use super::sanitize::sanitize_html;

/// Max characters kept for a backlink context snippet.
const CONTEXT_SNIPPET_MAX_CHARS: usize = 160;

/// How a single `![[note]]` embed should render. The DB-aware pipeline
/// ([`crate::garden`]) resolves every embed to one of these before handing the
/// map to [`render_markdown_with_embeds`]; the pure renderer never decides
/// visibility or recursion, it only splices in what it is told.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EmbedResolution {
    /// Splice this already-rendered, already-sanitized HTML in place of the
    /// embed (a published target's transcluded content).
    Content(String),
    /// Render a neutral placeholder. Used for a draft/missing target (no leak),
    /// a cycle, or an exceeded depth limit — never reveals anything about the
    /// target beyond the requested slug.
    Placeholder,
}

/// A wikilink or embed reference found in a document.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WikilinkRef {
    /// Resolution key: the inner text run through [`slugify`].
    pub target_slug: String,
    /// What the link should display (the alias if `[[slug|alias]]`, else the
    /// raw inner text).
    pub display: String,
    /// `true` for `![[note]]` embeds, `false` for `[[note]]` links.
    pub is_embed: bool,
    /// Surrounding text for the backlinks "linked from" panel (char-boundary
    /// safe, whitespace-collapsed, capped).
    pub context_snippet: String,
}

fn build_options() -> Options<'static> {
    let mut options = Options::default();
    options.extension.autolink = true;
    options.extension.strikethrough = true;
    options.extension.table = true;
    options.parse.smart = true;
    options.render.escape = false;
    options.render.r#unsafe = true;
    options.render.github_pre_lang = true;
    options
}

/// A raw `[[...]]` / `![[...]]` occurrence within a single text run, in byte
/// offsets into that run. All delimiters are ASCII, so the offsets always fall
/// on char boundaries even when the inner text is multibyte.
#[derive(Clone, Debug)]
struct RawMatch {
    start: usize,
    end: usize,
    is_embed: bool,
    inner: String,
    alias: Option<String>,
}

/// Scan one text run for wikilink/embed occurrences. Malformed forms (empty
/// inner, nested `[`, or an unterminated `[[`) are skipped, leaving the text
/// untouched.
fn scan(text: &str) -> Vec<RawMatch> {
    let bytes = text.as_bytes();
    let mut matches = Vec::new();
    let mut i = 0;
    while i + 1 < bytes.len() {
        if bytes[i] == b'[' && bytes[i + 1] == b'[' {
            let is_embed = i >= 1 && bytes[i - 1] == b'!';
            let start = if is_embed { i - 1 } else { i };
            // Find the closing "]]".
            if let Some(close) = find_close(bytes, i + 2) {
                let inner_raw = &text[i + 2..close];
                if inner_raw.contains('[') || inner_raw.contains('\n') {
                    // Malformed; skip past the opening and keep scanning.
                    i += 2;
                    continue;
                }
                let (inner, alias) = match inner_raw.split_once('|') {
                    Some((left, right)) => {
                        (left.trim().to_string(), Some(right.trim().to_string()))
                    }
                    None => (inner_raw.trim().to_string(), None),
                };
                if inner.is_empty() {
                    i += 2;
                    continue;
                }
                matches.push(RawMatch {
                    start,
                    end: close + 2,
                    is_embed,
                    inner,
                    alias,
                });
                i = close + 2;
                continue;
            } else {
                break;
            }
        }
        i += 1;
    }
    matches
}

/// Byte offset of the next "]]" at or after `from`, or `None`.
fn find_close(bytes: &[u8], from: usize) -> Option<usize> {
    let mut j = from;
    while j + 1 < bytes.len() {
        if bytes[j] == b']' && bytes[j + 1] == b']' {
            return Some(j);
        }
        j += 1;
    }
    None
}

/// Collect every `Text` node's literal in document order.
fn text_literals<'a>(node: &'a AstNode<'a>, out: &mut Vec<String>) {
    if let NodeValue::Text(ref text) = node.data.borrow().value {
        out.push(text.to_string());
    }
    for child in node.children() {
        text_literals(child, out);
    }
}

/// Extract all wikilink/embed references from `markdown` (pure; no DB).
pub fn extract_wikilinks(markdown: &str) -> Vec<WikilinkRef> {
    if markdown.trim().is_empty() {
        return Vec::new();
    }
    let arena = Arena::new();
    let root = parse_document(&arena, markdown, &build_options());

    let mut literals = Vec::new();
    text_literals(root, &mut literals);

    let mut refs = Vec::new();
    for literal in &literals {
        for m in scan(literal) {
            let display = m.alias.clone().unwrap_or_else(|| m.inner.clone());
            refs.push(WikilinkRef {
                target_slug: slugify(&m.inner),
                display,
                is_embed: m.is_embed,
                context_snippet: context_snippet(literal),
            });
        }
    }
    refs
}

/// Render `markdown` to sanitized HTML, rewriting `[[note]]` wikilinks to
/// anchors. `resolved` is the set of slugs that currently exist; a wikilink
/// whose slug is absent renders as a `stub`. Embeds (`![[note]]`) are left as
/// literal text — use [`render_markdown_with_embeds`] (via the DB-aware
/// pipeline) to expand them.
pub fn render_markdown_with_links(markdown: &str, resolved: &HashSet<String>) -> String {
    render_inner(markdown, resolved, &HashMap::new())
}

/// Render a backlinks **context snippet** to inline HTML.
///
/// The snippet is a *preview* of the source note's raw text, so the literal
/// `[[ ]]` brackets are **kept** — they signal "wikilink as authored" and set
/// the panel apart from a normal prose link (the rendered body, by contrast,
/// strips the brackets). This rewrites only the `[[target]]` / `[[target|alias]]`
/// tokens into a clickable `<a href="/{slug}">[[display]]</a>` (or
/// `<a class="stub" …>` when the slug is not in `resolved`), keeping the
/// brackets *inside* the link so the whole token is clickable, and HTML-escapes
/// everything around them. Unlike [`render_markdown_with_links`] it stays inline
/// (no `<p>` wrapper) so it can sit inside the panel's own
/// `<p class="backlink-context">`, and it builds the markup directly (the
/// surrounding text and `display` are escaped, `slug` is already `[a-z0-9-]`), so
/// no HTML sanitizer pass is needed. An `![[embed]]` mention isn't navigational
/// in a preview, so it stays as literal, non-clickable `![[display]]` text.
pub fn render_snippet_with_links(snippet: &str, resolved: &HashSet<String>) -> String {
    let matches = scan(snippet);
    if matches.is_empty() {
        return escape_html(snippet);
    }
    let mut out = String::with_capacity(snippet.len() + 16);
    let mut cursor = 0usize;
    for m in &matches {
        if m.start > cursor {
            out.push_str(&escape_html(&snippet[cursor..m.start]));
        }
        let display = m.alias.clone().unwrap_or_else(|| m.inner.clone());
        let bracketed = escape_html(&format!("[[{display}]]"));
        if m.is_embed {
            // Embeds are transclusion mentions, not navigation: keep the literal
            // `![[…]]` form, un-clickable.
            out.push_str(&format!("!{bracketed}"));
        } else {
            let slug = slugify(&m.inner);
            let class = if resolved.contains(&slug) {
                ""
            } else {
                " class=\"stub\""
            };
            out.push_str(&format!(
                "<a{class} href=\"/{slug}\">{bracketed}</a>",
                class = class,
                slug = slug,
                bracketed = bracketed,
            ));
        }
        cursor = m.end;
    }
    if cursor < snippet.len() {
        out.push_str(&escape_html(&snippet[cursor..]));
    }
    out
}

/// Like [`render_markdown_with_links`], but each `![[note]]` embed is replaced
/// by its [`EmbedResolution`]: a published target's pre-rendered content, or a
/// neutral placeholder for a draft/missing target, a cycle, or an exceeded
/// depth limit. An embed whose slug is absent from `embeds` falls back to a
/// placeholder so a forgotten resolution can never leak the literal `![[...]]`.
///
/// The recursion, cycle/depth guard, and visibility decision all live in
/// [`crate::garden`]; this function only splices in the resolutions it is given.
/// Embeds inside code spans / fenced blocks stay literal (only `Text` nodes are
/// rewritten), exactly like wikilinks.
pub fn render_markdown_with_embeds(
    markdown: &str,
    resolved: &HashSet<String>,
    embeds: &HashMap<String, EmbedResolution>,
) -> String {
    render_inner(markdown, resolved, embeds)
}

fn render_inner(
    markdown: &str,
    resolved: &HashSet<String>,
    embeds: &HashMap<String, EmbedResolution>,
) -> String {
    if markdown.trim().is_empty() {
        return String::new();
    }
    let arena = Arena::new();
    let options = build_options();
    let root = parse_document(&arena, markdown, &options);

    // Collect text nodes first; splicing while iterating descendants is unsafe.
    let mut text_nodes = Vec::new();
    collect_text_nodes(root, &mut text_nodes);

    for node in text_nodes {
        let literal = match node.data.borrow().value {
            NodeValue::Text(ref text) => text.to_string(),
            _ => continue,
        };
        let matches = scan(&literal);
        if matches.is_empty() {
            continue;
        }

        let mut cursor = 0usize;
        for m in &matches {
            if m.start > cursor {
                append_text(&arena, node, &literal[cursor..m.start]);
            }
            if m.is_embed {
                let slug = slugify(&m.inner);
                match embeds.get(&slug) {
                    Some(EmbedResolution::Content(content)) => {
                        append_html(&arena, node, embed_wrapper(&slug, content));
                    }
                    Some(EmbedResolution::Placeholder) => {
                        append_html(&arena, node, embed_placeholder(&slug).to_string());
                    }
                    None => {
                        // No resolution supplied (e.g. the pure
                        // `render_markdown_with_links` path, or a forgotten
                        // slug): emit a placeholder rather than leaking the raw
                        // `![[...]]` markup.
                        if embeds.is_empty() {
                            // Pure path: keep the literal markup unchanged so
                            // existing wikilink-only rendering is untouched.
                            append_text(&arena, node, &literal[m.start..m.end]);
                        } else {
                            append_html(&arena, node, embed_placeholder(&slug).to_string());
                        }
                    }
                }
            } else {
                let slug = slugify(&m.inner);
                let display = m.alias.clone().unwrap_or_else(|| m.inner.clone());
                let class = if resolved.contains(&slug) {
                    ""
                } else {
                    " class=\"stub\""
                };
                let html = format!(
                    "<a{class} href=\"/{slug}\">{}</a>",
                    escape_html(&display),
                    class = class,
                    slug = slug,
                );
                append_html(&arena, node, html);
            }
            cursor = m.end;
        }
        if cursor < literal.len() {
            append_text(&arena, node, &literal[cursor..]);
        }
        node.detach();
    }

    let mut html = String::new();
    // format_html only errors on a write failure into a String, which can't happen.
    let _ = format_html(root, &options, &mut html);
    sanitize_html(&html)
}

/// Wrap transcluded content in a labelled `<figure>` so it is visually distinct
/// from the host note. `content` is already sanitized HTML and the whole result
/// is re-sanitized by the caller. Only the `class` attribute is used, so nothing
/// is stripped by the sanitizer.
fn embed_wrapper(_slug: &str, content: &str) -> String {
    format!(
        r#"<figure class="embed">{content}</figure>"#,
        content = content
    )
}

/// Neutral placeholder for an embed that cannot (or must not) be expanded: a
/// draft/missing target, a cycle, or an exceeded depth limit. It reveals nothing
/// about the target — not even the slug — upholding draft invisibility.
fn embed_placeholder(_slug: &str) -> &'static str {
    r#"<figure class="embed embed-missing"><p>Embed omitted.</p></figure>"#
}

fn collect_text_nodes<'a>(node: &'a AstNode<'a>, out: &mut Vec<&'a AstNode<'a>>) {
    if matches!(node.data.borrow().value, NodeValue::Text(_)) {
        out.push(node);
    }
    for child in node.children() {
        collect_text_nodes(child, out);
    }
}

/// Insert a `Text` sibling before `anchor`.
fn append_text<'a>(arena: &'a Arena<'a>, anchor: &'a AstNode<'a>, text: &str) {
    if text.is_empty() {
        return;
    }
    let node = arena.alloc(AstNode::from(NodeValue::Text(text.to_string().into())));
    anchor.insert_before(node);
}

/// Insert a raw inline-HTML sibling before `anchor`.
fn append_html<'a>(arena: &'a Arena<'a>, anchor: &'a AstNode<'a>, html: String) {
    let node = arena.alloc(AstNode::from(NodeValue::HtmlInline(html)));
    anchor.insert_before(node);
}

fn escape_html(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            other => out.push(other),
        }
    }
    out
}

/// Whitespace-collapsed, char-boundary-safe, capped snippet for backlinks.
fn context_snippet(text: &str) -> String {
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    char_truncate(&collapsed, CONTEXT_SNIPPET_MAX_CHARS)
}

/// Truncate to at most `max` characters, never splitting a multibyte char.
fn char_truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let truncated: String = text.chars().take(max).collect();
    format!("{truncated}…")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slugs(refs: &[WikilinkRef]) -> Vec<String> {
        refs.iter().map(|r| r.target_slug.clone()).collect()
    }

    #[test]
    fn extracts_simple_wikilink_and_normalizes_to_slug() {
        let refs = extract_wikilinks("see [[My Note]] here");
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].target_slug, "my-note");
        assert_eq!(refs[0].display, "My Note");
        assert!(!refs[0].is_embed);
    }

    #[test]
    fn alias_sets_display_but_slug_comes_from_target() {
        let refs = extract_wikilinks("[[my-note|Click here]]");
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].target_slug, "my-note");
        assert_eq!(refs[0].display, "Click here");
    }

    #[test]
    fn embed_is_recognized() {
        let refs = extract_wikilinks("![[diagram]]");
        assert_eq!(refs.len(), 1);
        assert!(refs[0].is_embed);
        assert_eq!(refs[0].target_slug, "diagram");
    }

    #[test]
    fn wikilink_inside_inline_code_is_ignored() {
        let refs = extract_wikilinks("real [[note]] but `[[not-a-link]]` stays literal");
        assert_eq!(slugs(&refs), vec!["note".to_string()]);
    }

    #[test]
    fn wikilink_inside_fenced_code_block_is_ignored() {
        let md = "text [[real]]\n\n```\n[[fenced]]\n```\n";
        assert_eq!(slugs(&extract_wikilinks(md)), vec!["real".to_string()]);
    }

    #[test]
    fn multibyte_text_around_link_does_not_panic_and_snippets_safely() {
        let refs = extract_wikilinks("café ☕ [[note]] naïve façade");
        assert_eq!(refs.len(), 1);
        // Snippet contains the multibyte context, unbroken.
        assert!(refs[0].context_snippet.contains("café"));
    }

    #[test]
    fn malformed_links_are_skipped() {
        assert!(extract_wikilinks("unterminated [[oops").is_empty());
        assert!(extract_wikilinks("empty [[]] here").is_empty());
    }

    #[test]
    fn resolved_wikilink_renders_plain_anchor() {
        let resolved: HashSet<String> = ["my-note".to_string()].into_iter().collect();
        let html = render_markdown_with_links("see [[My Note]]", &resolved);
        assert!(html.contains("href=\"/my-note\""));
        assert!(!html.contains("class=\"stub\""));
        assert!(html.contains(">My Note<"));
    }

    #[test]
    fn unresolved_wikilink_renders_stub() {
        let resolved: HashSet<String> = HashSet::new();
        let html = render_markdown_with_links("see [[Missing]]", &resolved);
        assert!(html.contains("class=\"stub\""));
        assert!(html.contains("href=\"/missing\""));
    }

    #[test]
    fn wikilink_in_code_is_not_rewritten_when_rendering() {
        let resolved: HashSet<String> = HashSet::new();
        let html = render_markdown_with_links("`[[code]]`", &resolved);
        // The literal stays inside the code element, not turned into an anchor.
        assert!(!html.contains("href=\"/code\""));
        assert!(html.contains("<code>"));
    }

    #[test]
    fn surrounding_text_is_preserved_and_escaped() {
        let resolved: HashSet<String> = ["a".to_string()].into_iter().collect();
        let html = render_markdown_with_links("x < y and [[a]] done", &resolved);
        assert!(html.contains("href=\"/a\""));
        assert!(html.contains("&lt;"));
    }

    #[test]
    fn snippet_resolved_link_keeps_brackets_clickable_and_escapes_text() {
        let resolved: HashSet<String> = ["welcome".to_string()].into_iter().collect();
        let html = render_snippet_with_links("Start at [[welcome]] & go", &resolved);
        // Brackets are preserved and live *inside* the anchor, so the whole
        // `[[welcome]]` token is clickable.
        assert_eq!(html, r#"Start at <a href="/welcome">[[welcome]]</a> &amp; go"#);
    }

    #[test]
    fn snippet_unresolved_link_is_a_stub_with_brackets() {
        let resolved: HashSet<String> = HashSet::new();
        let html = render_snippet_with_links("read [[Missing Note]]", &resolved);
        assert!(html.contains(r#"<a class="stub" href="/missing-note">[[Missing Note]]</a>"#));
    }

    #[test]
    fn snippet_alias_shows_bracketed_alias_links_to_target() {
        let resolved: HashSet<String> = ["my-note".to_string()].into_iter().collect();
        let html = render_snippet_with_links("see [[my-note|the alias]]", &resolved);
        assert!(html.contains(r#"<a href="/my-note">[[the alias]]</a>"#));
    }

    #[test]
    fn snippet_embed_mention_stays_literal_and_unclickable() {
        let html = render_snippet_with_links("here ![[diagram]] there", &HashSet::new());
        assert_eq!(html, "here ![[diagram]] there");
        assert!(!html.contains("href"));
    }

    #[test]
    fn snippet_without_wikilinks_is_just_escaped() {
        let html = render_snippet_with_links("plain <text> & more", &HashSet::new());
        assert_eq!(html, "plain &lt;text&gt; &amp; more");
    }

    #[test]
    fn embed_with_content_resolution_splices_the_content() {
        let mut embeds = HashMap::new();
        embeds.insert(
            "diagram".to_string(),
            EmbedResolution::Content("<p>embedded body</p>".to_string()),
        );
        let html =
            render_markdown_with_embeds("before ![[diagram]] after", &HashSet::new(), &embeds);
        assert!(html.contains("embedded body"), "content is spliced inline");
        assert!(html.contains(r#"class="embed""#));
        assert!(
            !html.contains("![["),
            "the raw embed markup is consumed, not left literal"
        );
    }

    #[test]
    fn embed_with_placeholder_resolution_renders_neutral_placeholder() {
        let mut embeds = HashMap::new();
        embeds.insert("secret".to_string(), EmbedResolution::Placeholder);
        let html = render_markdown_with_embeds("![[secret]]", &HashSet::new(), &embeds);
        assert!(html.contains("Embed omitted"), "placeholder is rendered");
        assert!(
            !html.contains("secret"),
            "a placeholder reveals nothing about the target, not even its slug"
        );
    }

    #[test]
    fn unmapped_embed_in_db_path_falls_back_to_placeholder_not_literal() {
        // A non-empty embed map signals the DB-aware path: a missing key must
        // become a placeholder, never leak the literal `![[...]]`.
        let mut embeds = HashMap::new();
        embeds.insert("other".to_string(), EmbedResolution::Placeholder);
        let html = render_markdown_with_embeds("![[forgotten]]", &HashSet::new(), &embeds);
        assert!(html.contains("Embed omitted"));
        assert!(!html.contains("![["));
    }

    #[test]
    fn embed_inside_code_stays_literal_even_with_a_resolution() {
        let mut embeds = HashMap::new();
        embeds.insert(
            "note".to_string(),
            EmbedResolution::Content("<p>should not appear</p>".to_string()),
        );
        let html = render_markdown_with_embeds("`![[note]]`", &HashSet::new(), &embeds);
        assert!(html.contains("<code>"), "code span is preserved");
        assert!(
            !html.contains("should not appear"),
            "an embed inside code is never expanded"
        );
    }

    #[test]
    fn embeds_stay_literal_in_the_pure_wikilink_path() {
        // render_markdown_with_links uses an empty embed map → embeds untouched.
        let html = render_markdown_with_links("![[diagram]]", &HashSet::new());
        assert!(
            html.contains("![[diagram]]"),
            "the wikilink-only path leaves embeds literal"
        );
    }
}
