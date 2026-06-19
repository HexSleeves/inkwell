use inkwell::rendering::render_document_html;

#[test]
fn rendering_removes_scripts_and_keeps_basic_markdown() {
    let html = render_document_html(
        "# Hi

<script>alert(1)</script>

**bold**",
    );
    assert!(html.contains("<h1>Hi</h1>"));
    assert!(html.contains("<strong>bold</strong>"));
    assert!(!html.contains("<script>"));
}

#[test]
fn rendering_keeps_code_blocks() {
    let html = render_document_html(
        "```rust
fn main() {}
```",
    );
    assert!(html.contains("<pre><code"));
}
