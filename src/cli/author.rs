//! `inkwell author` — the first human authoring surface (ADR 0008, Option A).
//!
//! These commands speak the existing authenticated HTTP write API rather than
//! touching the database directly, so the CLI behaves exactly like any other
//! API client. Authors write Markdown files with a small YAML-ish front matter
//! block and round-trip them through `new` -> `push` -> `publish`.

use std::path::Path;

use anyhow::{Context, Result, anyhow, bail};

use crate::client::{DocumentInput, InkwellClient};
use crate::config::AuthorConfig;
use crate::domain::slug::{is_valid_slug, slugify};

const USAGE: &str = "usage: inkwell author <new|push|publish|unpublish> ...
  inkwell author new <title> [--slug <slug>] [--status draft|published] [--tag <tag>]... [-o <file>] [--force]
  inkwell author push <file> [--server <url>]
  inkwell author publish <slug> [--server <url>]
  inkwell author unpublish <slug> [--server <url>]";

/// A document parsed from a local Markdown file: front matter plus body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedDocument {
    pub title: String,
    pub slug: Option<String>,
    pub status: Option<String>,
    pub tags: Vec<String>,
    pub body: String,
}

impl ParsedDocument {
    /// The slug this document will be pushed under: the explicit front-matter
    /// slug when present and valid, otherwise one derived from the title using
    /// the same slugification the server applies.
    pub fn effective_slug(&self) -> Result<String> {
        match self.slug.as_deref() {
            Some(slug) => {
                if is_valid_slug(slug) {
                    Ok(slug.to_string())
                } else {
                    bail!(
                        "Front matter \"slug\" must be lowercase alphanumerics separated by single hyphens: {slug:?}"
                    )
                }
            }
            None => {
                let slug = slugify(&self.title);
                if slug.is_empty() {
                    bail!(
                        "Could not derive a slug from title {:?}; add an explicit \"slug\" to the front matter.",
                        self.title
                    );
                }
                Ok(slug)
            }
        }
    }

    /// Resolve the authoring policy (slug derivation) and build the client's
    /// transport-facing [`DocumentInput`]. Keeps slug policy in the CLI while
    /// the client stays unaware of `ParsedDocument`.
    pub fn to_input(&self) -> Result<DocumentInput> {
        Ok(DocumentInput {
            title: self.title.clone(),
            slug: self.effective_slug()?,
            body: self.body.clone(),
            tags: self.tags.clone(),
        })
    }
}

/// Options for scaffolding a new document with `inkwell author new`.
#[derive(Debug, Clone)]
pub struct NewOptions {
    pub title: String,
    pub slug: Option<String>,
    pub status: String,
    pub tags: Vec<String>,
}

// ---------------------------------------------------------------------------
// File format: YAML-ish front matter + Markdown body
// ---------------------------------------------------------------------------

/// Parse a Markdown authoring file: a `---` delimited front matter block
/// followed by the Markdown body. Supports scalar `key: value` lines and a
/// `tags:` block list — the subset documented in ADR 0008.
pub fn parse_markdown(input: &str) -> Result<ParsedDocument> {
    let (front_matter, body) = split_front_matter(input)?;
    let mut title: Option<String> = None;
    let mut slug: Option<String> = None;
    let mut status: Option<String> = None;
    let mut tags: Vec<String> = Vec::new();
    let mut list_key: Option<String> = None;

    for raw in front_matter.lines() {
        let line = raw.trim_end();
        if line.trim().is_empty() {
            continue;
        }
        let trimmed = line.trim_start();
        if trimmed.starts_with('#') {
            continue;
        }

        // List item belonging to the most recent empty-valued key.
        if let Some(rest) = trimmed.strip_prefix('-') {
            let key = list_key.as_deref().ok_or_else(|| {
                anyhow!("Unexpected list item in front matter (no preceding key): {line:?}")
            })?;
            if key != "tags" {
                bail!("Only \"tags\" accepts a list value in front matter; saw it under {key:?}.");
            }
            let value = unquote(rest.trim());
            if !value.is_empty() {
                tags.push(value);
            }
            continue;
        }

        let (key, value) = trimmed.split_once(':').ok_or_else(|| {
            anyhow!("Invalid front matter line (expected \"key: value\"): {line:?}")
        })?;
        let key = key.trim().to_ascii_lowercase();
        let value = unquote(value.trim());

        if value.is_empty() {
            // Begin a block list (e.g. `tags:` on its own line).
            list_key = Some(key);
            continue;
        }
        list_key = None;
        match key.as_str() {
            "title" => title = Some(value),
            "slug" => slug = Some(value),
            "status" => status = Some(value),
            // Inline list form: `tags: rust, notes`.
            "tags" => tags.extend(
                value
                    .split(',')
                    .map(|t| t.trim().to_string())
                    .filter(|t| !t.is_empty()),
            ),
            _ => { /* ignore unknown keys for forward compatibility */ }
        }
    }

    let title = title
        .filter(|t| !t.trim().is_empty())
        .ok_or_else(|| anyhow!("Front matter must include a non-empty \"title\"."))?;

    if let Some(status) = status.as_deref()
        && status != "draft"
        && status != "published"
    {
        bail!("Front matter \"status\" must be \"draft\" or \"published\", saw {status:?}.");
    }

    Ok(ParsedDocument {
        title,
        slug,
        status,
        tags,
        body: body.trim().to_string(),
    })
}

/// Split a document into its front matter block and body. The file must begin
/// with a `---` line and close the block with another `---` line.
fn split_front_matter(input: &str) -> Result<(String, String)> {
    let text = input.strip_prefix('\u{feff}').unwrap_or(input);
    let mut lines = text.split_inclusive('\n');
    let first = lines
        .next()
        .ok_or_else(|| anyhow!("Markdown file is empty; expected a \"---\" front matter block."))?;
    if first.trim_end_matches(['\r', '\n']) != "---" {
        bail!("Markdown file must begin with a \"---\" front matter delimiter.");
    }

    let mut front_matter = String::new();
    let mut closed = false;
    for line in lines.by_ref() {
        if line.trim_end_matches(['\r', '\n']) == "---" {
            closed = true;
            break;
        }
        front_matter.push_str(line);
    }
    if !closed {
        bail!("Front matter block is not closed with a \"---\" line.");
    }

    let body: String = lines.collect();
    Ok((front_matter, body))
}

/// Strip a single pair of matching surrounding quotes, if present.
fn unquote(value: &str) -> String {
    let bytes = value.as_bytes();
    if value.len() >= 2
        && ((bytes[0] == b'"' && bytes[bytes.len() - 1] == b'"')
            || (bytes[0] == b'\'' && bytes[bytes.len() - 1] == b'\''))
    {
        value[1..value.len() - 1].to_string()
    } else {
        value.to_string()
    }
}

/// Render a starter Markdown document from scaffolding options.
pub fn render_new_document(opts: &NewOptions) -> Result<String> {
    let slug = match opts.slug.as_deref() {
        Some(slug) => {
            if !is_valid_slug(slug) {
                bail!(
                    "--slug must be lowercase alphanumerics separated by single hyphens: {slug:?}"
                );
            }
            slug.to_string()
        }
        None => {
            let slug = slugify(&opts.title);
            if slug.is_empty() {
                bail!(
                    "Could not derive a slug from title {:?}; pass --slug explicitly.",
                    opts.title
                );
            }
            slug
        }
    };
    if opts.status != "draft" && opts.status != "published" {
        bail!(
            "--status must be \"draft\" or \"published\", saw {:?}.",
            opts.status
        );
    }

    let mut out = String::new();
    out.push_str("---\n");
    out.push_str(&format!("title: {}\n", yaml_scalar(&opts.title)));
    out.push_str(&format!("slug: {slug}\n"));
    out.push_str(&format!("status: {}\n", opts.status));
    if !opts.tags.is_empty() {
        out.push_str("tags:\n");
        for tag in &opts.tags {
            out.push_str(&format!("  - {}\n", yaml_scalar(tag)));
        }
    }
    out.push_str("---\n\n");
    out.push_str(&format!("# {}\n\nWrite your content here.\n", opts.title));
    Ok(out)
}

/// Quote a scalar value if it contains characters that would confuse the
/// minimal front matter parser (a colon, a leading list/quote marker, ...).
fn yaml_scalar(value: &str) -> String {
    let needs_quote = value.is_empty()
        || value.contains(':')
        || value.contains('#')
        || value.starts_with(['-', '"', '\'', ' ']);
    if needs_quote {
        format!("\"{}\"", value.replace('"', "\\\""))
    } else {
        value.to_string()
    }
}

// ---------------------------------------------------------------------------
// CLI entry points
// ---------------------------------------------------------------------------

/// Dispatch `inkwell author <subcommand>`.
pub async fn run(mut args: impl Iterator<Item = String>) -> Result<()> {
    match args.next().as_deref() {
        Some("new") => cmd_new(args),
        Some("push") => cmd_push(args).await,
        Some("publish") => cmd_publish(args).await,
        Some("unpublish") => cmd_unpublish(args).await,
        Some(other) => bail!("unknown author subcommand {other:?}\n{USAGE}"),
        None => bail!("{USAGE}"),
    }
}

fn cmd_new(args: impl Iterator<Item = String>) -> Result<()> {
    let mut title: Option<String> = None;
    let mut slug: Option<String> = None;
    let mut status = "draft".to_string();
    let mut tags: Vec<String> = Vec::new();
    let mut output: Option<String> = None;
    let mut force = false;

    let mut args = args.peekable();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--slug" => slug = Some(take_value(&mut args, "--slug")?),
            "--status" => status = take_value(&mut args, "--status")?,
            "--tag" => tags.push(take_value(&mut args, "--tag")?),
            "--title" => title = Some(take_value(&mut args, "--title")?),
            "-o" | "--output" => output = Some(take_value(&mut args, "--output")?),
            "--force" => force = true,
            other if other.starts_with('-') => bail!("unknown flag {other:?}\n{USAGE}"),
            _ => {
                if title.is_some() {
                    bail!("unexpected extra argument {arg:?}\n{USAGE}");
                }
                title = Some(arg);
            }
        }
    }

    let title = title.ok_or_else(|| anyhow!("a title is required\n{USAGE}"))?;
    let opts = NewOptions {
        title,
        slug,
        status,
        tags,
    };
    let rendered = render_new_document(&opts)?;

    // Default the output path to "<slug>.md".
    let path = match output {
        Some(path) => path,
        None => {
            // Re-derive the slug exactly as render_new_document resolved it.
            let parsed = parse_markdown(&rendered)?;
            format!("{}.md", parsed.effective_slug()?)
        }
    };
    if !force && Path::new(&path).exists() {
        bail!("refusing to overwrite existing file {path:?}; pass --force to replace it.");
    }
    std::fs::write(&path, rendered).with_context(|| format!("writing {path:?}"))?;
    println!("Wrote {path}");
    Ok(())
}

async fn cmd_push(args: impl Iterator<Item = String>) -> Result<()> {
    let (path, server) = parse_target_args(args, "push <file>")?;
    let source = std::fs::read_to_string(&path).with_context(|| format!("reading {path:?}"))?;
    let doc = parse_markdown(&source)?;
    let input = doc.to_input()?;
    let client = build_client(server.as_deref())?;
    let (action, summary) = client.push(&input).await?;
    println!(
        "{} {} (status: {})",
        action.label(),
        summary.slug,
        summary.status
    );
    Ok(())
}

async fn cmd_publish(args: impl Iterator<Item = String>) -> Result<()> {
    let (slug, server) = parse_target_args(args, "publish <slug>")?;
    let client = build_client(server.as_deref())?;
    let summary = client.publish(&slug).await?;
    println!("Published {} (status: {})", summary.slug, summary.status);
    Ok(())
}

async fn cmd_unpublish(args: impl Iterator<Item = String>) -> Result<()> {
    let (slug, server) = parse_target_args(args, "unpublish <slug>")?;
    let client = build_client(server.as_deref())?;
    let summary = client.unpublish(&slug).await?;
    println!("Unpublished {} (status: {})", summary.slug, summary.status);
    Ok(())
}

/// Parse a single positional argument plus an optional `--server <url>` flag.
fn parse_target_args(
    args: impl Iterator<Item = String>,
    usage_tail: &str,
) -> Result<(String, Option<String>)> {
    let mut positional: Option<String> = None;
    let mut server: Option<String> = None;
    let mut args = args.peekable();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--server" => server = Some(take_value(&mut args, "--server")?),
            other if other.starts_with('-') => bail!("unknown flag {other:?}\n{USAGE}"),
            _ => {
                if positional.is_some() {
                    bail!("unexpected extra argument {arg:?}\n{USAGE}");
                }
                positional = Some(arg);
            }
        }
    }
    let positional =
        positional.ok_or_else(|| anyhow!("usage: inkwell author {usage_tail} [--server <url>]"))?;
    Ok((positional, server))
}

fn take_value<I: Iterator<Item = String>>(
    args: &mut std::iter::Peekable<I>,
    flag: &str,
) -> Result<String> {
    args.next()
        .ok_or_else(|| anyhow!("flag {flag} requires a value"))
}

/// Build an [`InkwellClient`] from the environment-derived config plus an
/// optional `--server` override.
fn build_client(server: Option<&str>) -> Result<InkwellClient> {
    let config = AuthorConfig::from_env()?;
    let base_url = config.resolve_base_url(server);
    let api_key = config.api_key.clone().ok_or_else(|| {
        anyhow!("INKWELL_API_KEY is not set; the authoring API requires an API key.")
    })?;
    InkwellClient::new(base_url, api_key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_full_front_matter_with_block_tags() {
        let input = "---\n\
title: Hello World\n\
slug: hello-world\n\
status: draft\n\
tags:\n\
  - rust\n\
  - notes\n\
---\n\n# Heading\n\nBody text.\n";
        let doc = parse_markdown(input).unwrap();
        assert_eq!(doc.title, "Hello World");
        assert_eq!(doc.slug.as_deref(), Some("hello-world"));
        assert_eq!(doc.status.as_deref(), Some("draft"));
        assert_eq!(doc.tags, vec!["rust", "notes"]);
        assert_eq!(doc.body, "# Heading\n\nBody text.");
    }

    #[test]
    fn parses_inline_tags_and_quoted_title() {
        let input = "---\ntitle: \"Colons: allowed\"\ntags: rust, notes\n---\nBody\n";
        let doc = parse_markdown(input).unwrap();
        assert_eq!(doc.title, "Colons: allowed");
        assert_eq!(doc.tags, vec!["rust", "notes"]);
        assert!(doc.slug.is_none());
    }

    #[test]
    fn effective_slug_derives_from_title_when_absent() {
        let doc = ParsedDocument {
            title: "My First Post!".to_string(),
            slug: None,
            status: None,
            tags: vec![],
            body: "x".to_string(),
        };
        assert_eq!(doc.effective_slug().unwrap(), "my-first-post");
    }

    #[test]
    fn rejects_missing_title() {
        let input = "---\nslug: x\n---\nBody\n";
        assert!(parse_markdown(input).is_err());
    }

    #[test]
    fn rejects_missing_front_matter() {
        assert!(parse_markdown("# Just markdown\n").is_err());
    }

    #[test]
    fn rejects_unclosed_front_matter() {
        assert!(parse_markdown("---\ntitle: x\nBody\n").is_err());
    }

    #[test]
    fn rejects_invalid_status() {
        let input = "---\ntitle: x\nstatus: live\n---\nBody\n";
        assert!(parse_markdown(input).is_err());
    }

    #[test]
    fn scaffold_round_trips_through_parser() {
        let opts = NewOptions {
            title: "Draft: A New Idea".to_string(),
            slug: None,
            status: "draft".to_string(),
            tags: vec!["ideas".to_string()],
        };
        let rendered = render_new_document(&opts).unwrap();
        let doc = parse_markdown(&rendered).unwrap();
        assert_eq!(doc.title, "Draft: A New Idea");
        assert_eq!(doc.slug.as_deref(), Some("draft-a-new-idea"));
        assert_eq!(doc.status.as_deref(), Some("draft"));
        assert_eq!(doc.tags, vec!["ideas"]);
    }
}
