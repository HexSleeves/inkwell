//! Magic-byte MIME sniffing for the upload allowlist (CYP-45).
//!
//! The upload handler trusts nothing the client declares: it sniffs the leading
//! bytes and rejects the request when the sniffed type disagrees with the
//! declared `Content-Type`. That closes the "declare `image/png`, upload HTML"
//! hole — a stored-XSS vector for any browser that ignores `nosniff`, and a
//! polyglot-file vector for downstream consumers.
//!
//! Only the four allowlisted raster formats are recognised; anything else
//! sniffs as `None` and is refused. Deliberately dependency-free: these four
//! signatures are short, stable, and fully specified.

/// Sniff `bytes` and return the canonical MIME type, or `None` when the content
/// does not match an allowlisted image format.
pub fn sniff_image(bytes: &[u8]) -> Option<&'static str> {
    // PNG: 89 50 4E 47 0D 0A 1A 0A
    const PNG: &[u8] = &[0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a];
    if bytes.starts_with(PNG) {
        return Some("image/png");
    }
    // JPEG: FF D8 FF (SOI + first marker). Every JFIF/Exif variant starts here.
    if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        return Some("image/jpeg");
    }
    // GIF: "GIF87a" or "GIF89a"
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return Some("image/gif");
    }
    // WebP: RIFF container — "RIFF" <u32 size> "WEBP"
    if bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" {
        return Some("image/webp");
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sniffs_each_allowlisted_format() {
        assert_eq!(
            sniff_image(&[0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00]),
            Some("image/png")
        );
        assert_eq!(
            sniff_image(&[0xff, 0xd8, 0xff, 0xe0, 0x00]),
            Some("image/jpeg")
        );
        assert_eq!(sniff_image(b"GIF89a....."), Some("image/gif"));
        assert_eq!(sniff_image(b"GIF87a....."), Some("image/gif"));
        assert_eq!(
            sniff_image(b"RIFF\x00\x00\x00\x00WEBPVP8 "),
            Some("image/webp")
        );
    }

    #[test]
    fn rejects_non_image_and_active_content() {
        assert_eq!(sniff_image(b""), None);
        assert_eq!(sniff_image(b"hello"), None);
        // HTML masquerading as an image is the case that matters most.
        assert_eq!(sniff_image(b"<html><script>alert(1)</script>"), None);
        // SVG is XML, deliberately not allowlisted.
        assert_eq!(
            sniff_image(b"<svg xmlns=\"http://www.w3.org/2000/svg\"/>"),
            None
        );
        // RIFF container that is not WebP (e.g. WAV) must not pass.
        assert_eq!(sniff_image(b"RIFF\x00\x00\x00\x00WAVEfmt "), None);
    }

    #[test]
    fn truncated_headers_do_not_panic_or_pass() {
        assert_eq!(sniff_image(&[0x89, 0x50]), None);
        assert_eq!(sniff_image(b"RIFF"), None);
        assert_eq!(sniff_image(b"GIF8"), None);
    }
}
