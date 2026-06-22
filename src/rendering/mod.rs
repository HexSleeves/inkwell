pub mod highlight;
pub mod markdown;
pub mod sanitize;
pub mod wikilink;

pub use markdown::render_markdown;

pub fn render_document_html(markdown: &str) -> String {
    markdown::render_markdown(markdown)
}
