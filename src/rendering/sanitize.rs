use std::collections::HashSet;

pub fn sanitize_html(html: &str) -> String {
    let tags = [
        "h1",
        "h2",
        "h3",
        "h4",
        "h5",
        "h6",
        "p",
        "blockquote",
        "hr",
        "br",
        "a",
        "em",
        "strong",
        "del",
        "s",
        "sub",
        "sup",
        "mark",
        "abbr",
        "small",
        "code",
        "pre",
        "kbd",
        "samp",
        "ul",
        "ol",
        "li",
        "table",
        "thead",
        "tbody",
        "tfoot",
        "tr",
        "th",
        "td",
        "img",
        "figure",
        "figcaption",
        "div",
        "span",
    ]
    .into_iter()
    .collect::<HashSet<_>>();

    ammonia::Builder::default()
        .tags(tags)
        .generic_attributes(["class"].into_iter().collect())
        .add_tag_attributes("a", ["href", "title"])
        .add_tag_attributes("img", ["src", "alt", "title"])
        .add_tag_attributes("abbr", ["title"])
        .add_tag_attributes("th", ["align"])
        .add_tag_attributes("td", ["align"])
        .url_schemes(["http", "https", "mailto"].into_iter().collect())
        .link_rel(Some("noopener noreferrer nofollow"))
        .clean(html)
        .to_string()
}
