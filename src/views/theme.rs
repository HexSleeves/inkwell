//! Operator-supplied themes for Inkwell's **public** pages (CYP-56).
//!
//! A theme is a plain directory on disk, selected with `INKWELL_THEME_DIR`. It
//! contains zero or more *slot files*; each slot file replaces (or, for
//! `head.html`/`extra.css`, extends) one piece of the built-in rendering. Any
//! slot a theme does not provide falls back to the built-in markup, so a theme
//! that ships only `extra.css` is valid and a theme that ships only
//! `layout.html` keeps the built-in header, nav, and footer available as
//! variables.
//!
//! ## Why a substitution seam and not a template engine
//!
//! The whole surface is nine files and a `{{ name }}` substitution. That is
//! enough to satisfy the theming contract without adding minijinja/tera/askama
//! and their compile-time or runtime cost, and — more importantly — without
//! handing theme authors a loop/conditional language that could reach back into
//! document data. Themes receive **already-rendered, already-escaped or
//! already-sanitized** HTML fragments as variables; there is no way to ask a
//! theme template for raw document markdown, so [`crate::rendering::sanitize`]
//! stays the single gate on untrusted body HTML. Theme markup itself is
//! trusted-operator content and is emitted raw, exactly like the built-in
//! markup it replaces.
//!
//! ## Failure model
//!
//! Every problem is a startup error, never a broken page or a mid-request
//! panic: a missing directory, a subdirectory, an unrecognized file name, a
//! non-UTF-8 file, a malformed `{{` placeholder, an unknown variable, or a
//! directory with no recognized slot files at all. Themes are read once at
//! startup; editing a theme requires a restart.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};

/// Active-nav class fragment handed to `nav.html` for the current page. Matches
/// the class the built-in nav applies, leading space included, so a theme can
/// interpolate it straight into `class="…{{ active_notes }}"`.
pub const NAV_ACTIVE_CLASS: &str = " site-nav--active";

/// Slot file names. Referenced by [`Theme`] callers instead of bare strings so
/// a rename is a compile error rather than a silently dead override.
pub mod slot {
    /// Whole-page shell, from `<!doctype html>` down. The escape hatch when a
    /// theme wants full control of document structure.
    pub const LAYOUT: &str = "layout.html";
    /// Extra markup appended inside `<head>`, after the built-in tags. Additive
    /// on purpose: canonical/OpenGraph/JSON-LD correctness is not something a
    /// theme should be able to drop by accident. Use `layout.html` for full
    /// control of `<head>` placement.
    pub const HEAD: &str = "head.html";
    /// Site header block (the `<header>` element and everything inside it).
    pub const HEADER: &str = "header.html";
    /// Primary navigation block (the `<nav>` element and its links).
    pub const NAV: &str = "nav.html";
    /// Site footer block (the `<footer>` element).
    pub const FOOTER: &str = "footer.html";
    /// Replaces the built-in stylesheet served at `/assets/site.css`.
    pub const STYLES: &str = "styles.css";
    /// Appended after the stylesheet served at `/assets/site.css`, whether that
    /// is the built-in one or a `styles.css` replacement.
    pub const EXTRA_STYLES: &str = "extra.css";
    /// Body of the index (and paginated `/page/N`) pages.
    pub const INDEX: &str = "index.html";
    /// Body of a single document page.
    pub const DOCUMENT: &str = "document.html";
}

/// One slot: its file name, the variables its template may reference, and
/// whether it is a raw asset (no `{{ }}` substitution at all).
struct SlotSpec {
    file: &'static str,
    vars: &'static [&'static str],
    raw: bool,
}

/// Variables available to `layout.html`.
pub const LAYOUT_VARS: &[&str] = &[
    "lang",
    "head",
    "header",
    "nav",
    "footer",
    "main",
    "main_class",
    "site_name",
    "styles_url",
    "botanical_band",
];

/// Variables available to `head.html`.
pub const HEAD_VARS: &[&str] = &["site_name", "title", "canonical_url", "styles_url"];

/// Variables available to `header.html`.
pub const HEADER_VARS: &[&str] = &["site_name", "nav"];

/// Variables available to `nav.html`. The `active_*` variables expand to the
/// built-in active class (with a leading space) for the current page and to the
/// empty string otherwise, which is how a theme marks the current nav item
/// without needing template conditionals.
pub const NAV_VARS: &[&str] = &[
    "site_name",
    "nav_current",
    "active_dashboard",
    "active_notes",
    "active_tags",
    "active_graph",
    "active_settings",
];

/// Variables available to `footer.html`.
pub const FOOTER_VARS: &[&str] = &["site_name"];

/// Variables available to `index.html`.
pub const INDEX_VARS: &[&str] = &[
    "site_name",
    "site_description",
    "documents",
    "pager",
    "page",
    "total_pages",
];

/// Variables available to `document.html`. `body_html` is the sanitized
/// document body; the theme cannot obtain the unsanitized form.
pub const DOCUMENT_VARS: &[&str] = &[
    "site_name",
    "title",
    "slug",
    "url",
    "body_html",
    "meta_line",
    "published",
    "updated",
    "growth",
    "tags",
    "backlinks",
    "doc_nav",
];

const SLOTS: &[SlotSpec] = &[
    SlotSpec {
        file: slot::LAYOUT,
        vars: LAYOUT_VARS,
        raw: false,
    },
    SlotSpec {
        file: slot::HEAD,
        vars: HEAD_VARS,
        raw: false,
    },
    SlotSpec {
        file: slot::HEADER,
        vars: HEADER_VARS,
        raw: false,
    },
    SlotSpec {
        file: slot::NAV,
        vars: NAV_VARS,
        raw: false,
    },
    SlotSpec {
        file: slot::FOOTER,
        vars: FOOTER_VARS,
        raw: false,
    },
    SlotSpec {
        file: slot::STYLES,
        vars: &[],
        raw: true,
    },
    SlotSpec {
        file: slot::EXTRA_STYLES,
        vars: &[],
        raw: true,
    },
    SlotSpec {
        file: slot::INDEX,
        vars: INDEX_VARS,
        raw: false,
    },
    SlotSpec {
        file: slot::DOCUMENT,
        vars: DOCUMENT_VARS,
        raw: false,
    },
];

/// Files a theme directory may contain that are not slots. Present so a theme
/// can ship its own docs and licence without tripping the unknown-file check.
const IGNORED_FILES: &[&str] = &[
    "README.md",
    "README",
    "LICENSE",
    "LICENSE.md",
    "LICENSE.txt",
    "CHANGELOG.md",
];

fn spec_for(file: &str) -> Option<&'static SlotSpec> {
    SLOTS.iter().find(|spec| spec.file == file)
}

/// An operator-supplied theme: the slot templates found in one directory.
#[derive(Clone)]
pub struct Theme {
    dir: PathBuf,
    slots: BTreeMap<&'static str, String>,
}

impl std::fmt::Debug for Theme {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Print the slot names, not the template bodies: a theme can be large
        // and the interesting fact in a startup log is *which* slots are live.
        f.debug_struct("Theme")
            .field("dir", &self.dir)
            .field("slots", &self.slots.keys().collect::<Vec<_>>())
            .finish()
    }
}

impl Theme {
    /// Load every recognized slot file in `dir`.
    ///
    /// Returns an error — never a partially-loaded theme — for a missing or
    /// non-directory path, a subdirectory, an unrecognized file name, a
    /// non-UTF-8 file, a malformed or unknown `{{ placeholder }}`, or a
    /// directory containing no slot files at all.
    pub fn load(dir: impl AsRef<Path>) -> Result<Self> {
        let dir = dir.as_ref();
        let metadata = std::fs::metadata(dir)
            .with_context(|| format!("cannot read theme directory {}", dir.display()))?;
        if !metadata.is_dir() {
            bail!("{} is not a directory", dir.display());
        }

        let mut slots = BTreeMap::new();
        let entries = std::fs::read_dir(dir)
            .with_context(|| format!("cannot list theme directory {}", dir.display()))?;
        for entry in entries {
            let entry =
                entry.with_context(|| format!("cannot list theme directory {}", dir.display()))?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                bail!(
                    "theme directory {} contains a non-UTF-8 file name",
                    dir.display()
                );
            };
            // Editor swap files, `.git`, `.DS_Store` and friends are none of our
            // business; everything else must be something we recognize.
            if name.starts_with('.') {
                continue;
            }
            let file_type = entry
                .file_type()
                .with_context(|| format!("cannot stat {}", entry.path().display()))?;
            if file_type.is_dir() {
                bail!(
                    "theme directory {} contains subdirectory `{name}`: themes are a flat set of slot files and Inkwell serves no files from a theme directory",
                    dir.display()
                );
            }
            if IGNORED_FILES.contains(&name) {
                continue;
            }
            let Some(spec) = spec_for(name) else {
                bail!(
                    "theme directory {} contains unrecognized file `{name}`. Recognized slots: {}",
                    dir.display(),
                    SLOTS
                        .iter()
                        .map(|spec| spec.file)
                        .collect::<Vec<_>>()
                        .join(", ")
                );
            };
            let body = std::fs::read_to_string(entry.path()).with_context(|| {
                format!(
                    "cannot read theme file {} as UTF-8 text",
                    entry.path().display()
                )
            })?;
            if !spec.raw {
                validate_placeholders(name, &body, spec.vars)?;
            }
            slots.insert(spec.file, body);
        }

        if slots.is_empty() {
            bail!(
                "theme directory {} contains no recognized slot files. Expected at least one of: {}",
                dir.display(),
                SLOTS
                    .iter()
                    .map(|spec| spec.file)
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }

        Ok(Self {
            dir: dir.to_path_buf(),
            slots,
        })
    }

    /// The directory this theme was loaded from.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Whether the theme overrides `file` (a [`slot`] constant).
    pub fn has(&self, file: &str) -> bool {
        self.slots.contains_key(file)
    }

    /// Render one slot with `vars` substituted, or `None` when the theme does
    /// not override that slot (the caller then uses its built-in markup).
    pub fn render_slot(&self, file: &str, vars: &[(&str, &str)]) -> Option<String> {
        let template = self.slots.get(file)?;
        Some(render_template(template, vars))
    }

    /// The stylesheet to serve at `/assets/site.css`: `styles.css` when the
    /// theme replaces it (else `builtin`), followed by `extra.css` when present.
    pub fn stylesheet(&self, builtin: &str) -> String {
        let base = self
            .slots
            .get(slot::STYLES)
            .map(String::as_str)
            .unwrap_or(builtin);
        match self.slots.get(slot::EXTRA_STYLES) {
            Some(extra) => format!("{base}\n{extra}"),
            None => base.to_string(),
        }
    }
}

/// Reject anything a theme author would otherwise only discover as a blank
/// region on a live page: an unterminated `{{`, a name that is not a plain
/// lowercase identifier, or a variable this slot is not given.
fn validate_placeholders(file: &str, template: &str, allowed: &[&str]) -> Result<()> {
    let mut rest = template;
    while let Some(open) = rest.find("{{") {
        let tail = &rest[open + 2..];
        let close = tail.find("}}").ok_or_else(|| {
            anyhow!(
                "theme file `{file}` has an unterminated `{{{{` placeholder: add the closing `}}}}`"
            )
        })?;
        let name = tail[..close].trim();
        if name.is_empty()
            || !name
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        {
            bail!(
                "theme file `{file}` has a malformed placeholder `{{{{{}}}}}`: variable names are lowercase letters, digits, and underscores",
                &tail[..close]
            );
        }
        if !allowed.contains(&name) {
            bail!(
                "theme file `{file}` uses unknown variable `{{{{ {name} }}}}`. Available here: {}",
                allowed.join(", ")
            );
        }
        rest = &tail[close + 2..];
    }
    Ok(())
}

/// Substitute `{{ name }}` occurrences. Names are validated at load time, so an
/// unknown one here is unreachable; it expands to the empty string rather than
/// panicking mid-request.
fn render_template(template: &str, vars: &[(&str, &str)]) -> String {
    let mut out = String::with_capacity(template.len() + 512);
    let mut rest = template;
    while let Some(open) = rest.find("{{") {
        out.push_str(&rest[..open]);
        let tail = &rest[open + 2..];
        match tail.find("}}") {
            Some(close) => {
                let name = tail[..close].trim();
                if let Some((_, value)) = vars.iter().find(|(key, _)| *key == name) {
                    out.push_str(value);
                }
                rest = &tail[close + 2..];
            }
            // Unterminated: rejected at load time, so just pass it through.
            None => {
                out.push_str(&rest[open..]);
                return out;
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn substitution_fills_known_names_and_leaves_other_braces_alone() {
        let rendered = render_template(
            "<b>{{ title }}</b> .a { color: red } {{missing}}",
            &[("title", "Hi")],
        );
        assert_eq!(rendered, "<b>Hi</b> .a { color: red } ");
    }

    #[test]
    fn validation_rejects_unknown_variables_and_unterminated_placeholders() {
        let unknown = validate_placeholders("index.html", "{{ nope }}", INDEX_VARS).unwrap_err();
        assert!(unknown.to_string().contains("unknown variable"));
        assert!(
            unknown.to_string().contains("documents"),
            "the error lists what IS available: {unknown}"
        );

        let unterminated =
            validate_placeholders("layout.html", "{{ head", LAYOUT_VARS).unwrap_err();
        assert!(unterminated.to_string().contains("unterminated"));

        let malformed =
            validate_placeholders("layout.html", "{{ Head-Tag }}", LAYOUT_VARS).unwrap_err();
        assert!(malformed.to_string().contains("malformed placeholder"));

        validate_placeholders("layout.html", "{{head}} {{ main }}", LAYOUT_VARS).unwrap();
    }
}
