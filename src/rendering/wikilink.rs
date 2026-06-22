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

use std::collections::HashSet;

use comrak::nodes::{AstNode, NodeValue};
use comrak::{Arena, Options, format_html, parse_document};

use crate::domain::slug::slugify;

use super::sanitize::sanitize_html;

/// Max characters kept for a backlink context snippet.
const CONTEXT_SNIPPET_MAX_CHARS: usize = 160;

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
/// literal text in this phase.
pub fn render_markdown_with_links(markdown: &str, resolved: &HashSet<String>) -> String {
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
                // P1: embeds are recognized but not rendered — keep them literal.
                append_text(&arena, node, &literal[m.start..m.end]);
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
}
