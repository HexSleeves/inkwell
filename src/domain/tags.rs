use crate::domain::document::{MAX_TAG_LENGTH, MAX_TAGS};
use crate::domain::slug::is_valid_slug;

pub fn normalize_tags(tags: &[String]) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    let mut seen = std::collections::BTreeSet::new();

    for raw in tags {
        let tag = raw.trim().to_ascii_lowercase();
        if tag.is_empty() || tag.len() > MAX_TAG_LENGTH || !is_valid_slug(&tag) {
            return Err(format!(
                "Tag \"{raw}\" must be lowercase alphanumerics separated by single hyphens (≤ {MAX_TAG_LENGTH} chars)."
            ));
        }
        if seen.insert(tag.clone()) {
            out.push(tag);
        }
    }

    if out.len() > MAX_TAGS {
        return Err(format!("A document may have at most {MAX_TAGS} tags."));
    }

    Ok(out)
}
