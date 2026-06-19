use comrak::{Options, markdown_to_html};

use super::sanitize::sanitize_html;

pub fn render_markdown(markdown: &str) -> String {
    if markdown.trim().is_empty() {
        return String::new();
    }

    let mut options = Options::default();
    options.extension.autolink = true;
    options.extension.strikethrough = true;
    options.extension.table = true;
    options.parse.smart = true;
    options.render.escape = false;
    options.render.r#unsafe = true;
    options.render.github_pre_lang = true;

    let html = markdown_to_html(markdown, &options);
    sanitize_html(&html)
}
