use unicode_normalization::UnicodeNormalization;
use unicode_normalization::char::is_combining_mark;

use crate::domain::document::MAX_SLUG_LENGTH;

pub fn slugify(title: &str) -> String {
    let mut out = String::new();
    let mut pending_hyphen = false;

    for ch in title.nfkd() {
        if is_combining_mark(ch) {
            continue;
        }
        let ch = ch.to_ascii_lowercase();
        if ch.is_ascii_alphanumeric() {
            if pending_hyphen && !out.is_empty() {
                out.push('-');
            }
            pending_hyphen = false;
            out.push(ch);
        } else if !out.is_empty() {
            pending_hyphen = true;
        }
    }

    out
}

pub fn is_valid_slug(slug: &str) -> bool {
    if slug.is_empty() || slug.len() > MAX_SLUG_LENGTH {
        return false;
    }

    let bytes = slug.as_bytes();
    if bytes.first() == Some(&b'-') || bytes.last() == Some(&b'-') {
        return false;
    }

    let mut prev_hyphen = false;
    for byte in bytes {
        let is_hyphen = *byte == b'-';
        let is_valid = byte.is_ascii_lowercase() || byte.is_ascii_digit() || is_hyphen;
        if !is_valid || (is_hyphen && prev_hyphen) {
            return false;
        }
        prev_hyphen = is_hyphen;
    }

    true
}
