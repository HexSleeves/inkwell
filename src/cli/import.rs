//! `inkwell import <vault>` — minimal Obsidian vault import (card T7).
//!
//! Walks an Obsidian vault directory and pushes every Markdown note to the
//! server through the shared [`InkwellClient`], exactly like `inkwell author
//! push`. Notes flow through the normal create-or-update path, so the server's
//! own write-time link resolution turns `[[wikilinks]]` into real edges or
//! stubs — this command does **no** link rewriting itself.
//!
//! Per the design this is intentionally minimal: no attachments or images, no
//! link rewriting, no incremental sync. Each note is parsed, then either:
//!   - reuses [`author::parse_markdown`] when it begins with a `---` YAML front
//!     matter block, or
//!   - synthesizes `title` from the filename stem and `slug` from
//!     [`slugify`](crate::domain::slug::slugify) when there is no front matter
//!     (the common Obsidian case), with the whole file as the body.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};

use crate::cli::author::{ParsedDocument, parse_markdown};
use crate::client::{InkwellClient, PushAction};
use crate::config::AuthorConfig;
use crate::domain::slug::slugify;

const USAGE: &str = "usage: inkwell import <vault> [--server <url>] [--dry-run]";

/// Dispatch `inkwell import <vault>`.
pub async fn run(args: impl Iterator<Item = String>) -> Result<()> {
    let opts = ImportOptions::parse(args)?;
    let scan =
        collect_notes(&opts.vault).with_context(|| format!("scanning vault {:?}", opts.vault))?;

    if scan.notes.is_empty() && scan.load_failures.is_empty() {
        println!("No Markdown notes found under {:?}.", opts.vault);
        return Ok(());
    }

    if opts.dry_run {
        return report_dry_run(&scan);
    }

    let client = build_client(opts.server.as_deref())?;
    push_notes(&client, &scan).await
}

/// Parsed `inkwell import` invocation.
#[derive(Debug, Clone)]
struct ImportOptions {
    vault: PathBuf,
    server: Option<String>,
    dry_run: bool,
}

impl ImportOptions {
    fn parse(args: impl Iterator<Item = String>) -> Result<Self> {
        let mut vault: Option<PathBuf> = None;
        let mut server: Option<String> = None;
        let mut dry_run = false;

        let mut args = args.peekable();
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--server" => {
                    let value = args
                        .next()
                        .ok_or_else(|| anyhow!("flag --server requires a value\n{USAGE}"))?;
                    if value.starts_with('-') {
                        bail!("flag --server requires a value, got flag {value:?}\n{USAGE}");
                    }
                    server = Some(value);
                }
                "--dry-run" => dry_run = true,
                other if other.starts_with('-') => bail!("unknown flag {other:?}\n{USAGE}"),
                _ => {
                    if vault.is_some() {
                        bail!("unexpected extra argument {arg:?}\n{USAGE}");
                    }
                    vault = Some(PathBuf::from(arg));
                }
            }
        }

        let vault = vault.ok_or_else(|| anyhow!("a vault directory is required\n{USAGE}"))?;
        Ok(Self {
            vault,
            server,
            dry_run,
        })
    }
}

/// A note discovered in the vault: its source path plus the document derived
/// from it (front matter or filename fallback).
#[derive(Debug, Clone, PartialEq, Eq)]
struct Note {
    path: PathBuf,
    document: ParsedDocument,
}

/// The outcome of scanning a vault: the notes that loaded cleanly plus any
/// per-file failures (unreadable file, malformed front matter, underivable
/// slug) collected so the run can continue and report them in the summary.
#[derive(Debug, Default)]
struct Scan {
    notes: Vec<Note>,
    load_failures: Vec<(PathBuf, anyhow::Error)>,
}

/// Recursively collect every Markdown note under `vault`, skipping dotfiles and
/// dot-directories (`.obsidian`, `.trash`, ...). Notes are returned sorted by
/// path so a run is deterministic.
///
/// Per-file read/parse problems do **not** abort the scan: they are collected
/// into [`Scan::load_failures`] so the import continues over the remaining
/// notes and reports them in the created/updated/failed summary, matching the
/// command's continue-on-error contract. Only an error enumerating the vault
/// directories themselves (the caller cannot see what to import) is fatal.
fn collect_notes(vault: &Path) -> Result<Scan> {
    let mut paths = Vec::new();
    walk_markdown(vault, &mut paths)?;
    paths.sort();

    let mut scan = Scan {
        notes: Vec::with_capacity(paths.len()),
        load_failures: Vec::new(),
    };
    for path in paths {
        match load_note(&path) {
            Ok(note) => scan.notes.push(note),
            Err(err) => scan.load_failures.push((path, err)),
        }
    }
    Ok(scan)
}

/// Read and derive a single note, mapping any failure to a contextful error.
fn load_note(path: &Path) -> Result<Note> {
    let source =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let document = derive_document(path, &source)?;
    Ok(Note {
        path: path.to_path_buf(),
        document,
    })
}

/// Append the `*.md` files under `dir` to `out`, recursing into subdirectories.
/// Entries whose name begins with `.` are skipped entirely (files and dirs),
/// which excludes `.obsidian`, `.trash`, and other Obsidian metadata.
fn walk_markdown(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    let entries =
        std::fs::read_dir(dir).with_context(|| format!("reading directory {}", dir.display()))?;
    for entry in entries {
        let entry = entry.with_context(|| format!("reading an entry in {}", dir.display()))?;
        let name = entry.file_name();
        // Skip anything dot-prefixed: dotfiles and dot-dirs like `.obsidian`.
        if name.to_string_lossy().starts_with('.') {
            continue;
        }
        let path = entry.path();
        // Use the entry's own metadata so symlinked directories are treated as
        // files (we do not follow them), avoiding cycles.
        let file_type = entry
            .file_type()
            .with_context(|| format!("inspecting {}", path.display()))?;
        if file_type.is_dir() {
            walk_markdown(&path, out)?;
        } else if file_type.is_file() && has_markdown_extension(&path) {
            out.push(path);
        }
    }
    Ok(())
}

/// True when `path` has a `.md` extension (case-insensitive).
fn has_markdown_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("md"))
}

/// Derive a [`ParsedDocument`] from a note's contents.
///
/// A file that opens with a `---` front matter delimiter is parsed by
/// [`author::parse_markdown`] (reusing the exact authoring rules). Otherwise we
/// synthesize a document from the filename stem: `title = stem`, `slug =
/// slugify(stem)`, body = the whole file.
fn derive_document(path: &Path, source: &str) -> Result<ParsedDocument> {
    if has_front_matter(source) {
        return parse_markdown(source).with_context(|| format!("parsing {}", path.display()));
    }

    let stem = file_stem(path).ok_or_else(|| {
        anyhow!(
            "Could not derive a title from path {} (no file stem).",
            path.display()
        )
    })?;
    let slug = slugify(&stem);
    if slug.is_empty() {
        bail!(
            "Could not derive a slug from filename {:?}; rename the note or add front matter with an explicit \"slug\".",
            stem
        );
    }
    Ok(ParsedDocument {
        title: stem,
        slug: Some(slug),
        status: None,
        growth: None,
        tags: Vec::new(),
        body: source.trim().to_string(),
    })
}

/// True when `source` begins with a `---` front matter delimiter line. The
/// delimiter may be preceded by a UTF-8 BOM (which `parse_markdown` also
/// tolerates) and followed by `\n` or `\r\n`.
fn has_front_matter(source: &str) -> bool {
    let text = source.strip_prefix('\u{feff}').unwrap_or(source);
    let first = text.split_inclusive('\n').next().unwrap_or(text);
    first.trim_end_matches(['\r', '\n']) == "---"
}

/// The file name without its extension, as an owned `String`.
fn file_stem(path: &Path) -> Option<String> {
    path.file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
}

/// Report what a real run would push, sending nothing. Surfaces parse-time
/// problems (e.g. a derivable slug) so authors can fix them before going live,
/// including notes that failed to even load during the scan.
fn report_dry_run(scan: &Scan) -> Result<()> {
    println!("Dry run: {} note(s) would be pushed.", scan.notes.len());
    for note in &scan.notes {
        match note.document.to_input() {
            Ok(input) => println!(
                "  would push {} -> slug {:?} (title {:?})",
                note.path.display(),
                input.slug,
                input.title
            ),
            Err(err) => println!("  would FAIL {} -> {err}", note.path.display()),
        }
    }
    for (path, err) in &scan.load_failures {
        println!("  would FAIL {} -> {err}", path.display());
    }
    Ok(())
}

/// Push every note, continuing past per-file failures and collecting them, then
/// print a `N created, M updated, K failed` summary. Returns an error only when
/// at least one note failed, so the exit status reflects the outcome.
///
/// Notes that never loaded (collected in [`Scan::load_failures`] during the
/// scan) are folded into the failure count and reported alongside push
/// failures, so an unreadable or malformed note does not silently vanish.
async fn push_notes(client: &InkwellClient, scan: &Scan) -> Result<()> {
    let mut created = 0usize;
    let mut updated = 0usize;
    let mut failures: Vec<(PathBuf, anyhow::Error)> = Vec::new();

    for (path, err) in &scan.load_failures {
        eprintln!("Failed {}: {err}", path.display());
        failures.push((path.clone(), anyhow!("{err}")));
    }

    for note in &scan.notes {
        match push_one(client, note).await {
            Ok(PushAction::Created) => {
                created += 1;
                println!("Created {} ({})", note.document.title, note.path.display());
            }
            Ok(PushAction::Updated) => {
                updated += 1;
                println!("Updated {} ({})", note.document.title, note.path.display());
            }
            Err(err) => {
                eprintln!("Failed {}: {err}", note.path.display());
                failures.push((note.path.clone(), err));
            }
        }
    }

    println!(
        "\nImport summary: {created} created, {updated} updated, {} failed.",
        failures.len()
    );
    if !failures.is_empty() {
        for (path, err) in &failures {
            eprintln!("  {} — {err}", path.display());
        }
        bail!("{} note(s) failed to import.", failures.len());
    }
    Ok(())
}

/// Push a single note via the shared client (create-or-update by slug).
async fn push_one(client: &InkwellClient, note: &Note) -> Result<PushAction> {
    let input = note.document.to_input()?;
    let (action, _summary) = client.push(&input).await?;
    Ok(action)
}

/// Build an [`InkwellClient`] from the environment-derived config plus an
/// optional `--server` override, mirroring the `inkwell author` commands.
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

    /// A throwaway directory under the system temp dir, removed on drop. Keeps
    /// the vault-walk tests pure (filesystem only, no network) without pulling
    /// in a `tempfile` dev-dependency.
    struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        fn new(label: &str) -> Self {
            let unique = format!(
                "inkwell-import-test-{label}-{}-{:?}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            );
            let path = std::env::temp_dir().join(unique);
            std::fs::create_dir_all(&path).unwrap();
            TempDir { path }
        }

        fn write(&self, rel: &str, contents: &str) {
            let target = self.path.join(rel);
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::write(target, contents).unwrap();
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    #[test]
    fn detects_front_matter_only_when_first_line_is_delimiter() {
        assert!(has_front_matter("---\ntitle: x\n---\nBody\n"));
        assert!(has_front_matter("\u{feff}---\ntitle: x\n---\nBody\n"));
        assert!(has_front_matter("---\r\ntitle: x\r\n---\r\nBody\r\n"));
        assert!(!has_front_matter("# Just a heading\n\nBody.\n"));
        assert!(!has_front_matter("Some prose ---\n"));
        assert!(!has_front_matter(""));
    }

    #[test]
    fn front_mattered_note_reuses_parse_markdown() {
        let source = "---\ntitle: Hello World\nslug: hello-world\ntags: rust, notes\n---\n\n# Heading\n\nBody.\n";
        let doc = derive_document(Path::new("/vault/whatever.md"), source).unwrap();
        // Front matter wins over the filename stem.
        assert_eq!(doc.title, "Hello World");
        assert_eq!(doc.slug.as_deref(), Some("hello-world"));
        assert_eq!(doc.tags, vec!["rust", "notes"]);
        assert_eq!(doc.body, "# Heading\n\nBody.");
    }

    #[test]
    fn bare_note_derives_title_and_slug_from_filename() {
        let source = "Just some Obsidian prose with a [[wikilink]].\n";
        let doc = derive_document(Path::new("/vault/My First Note.md"), source).unwrap();
        assert_eq!(doc.title, "My First Note");
        assert_eq!(doc.slug.as_deref(), Some("my-first-note"));
        assert!(doc.status.is_none());
        assert!(doc.tags.is_empty());
        // The whole (trimmed) file becomes the body, wikilink untouched.
        assert_eq!(doc.body, "Just some Obsidian prose with a [[wikilink]].");
        // The derived input carries the filename-based slug.
        assert_eq!(doc.to_input().unwrap().slug, "my-first-note");
    }

    #[test]
    fn walk_skips_dot_dirs_and_collects_md_only() {
        let vault = TempDir::new("walk");
        // A front-mattered note at the root.
        vault.write(
            "front.md",
            "---\ntitle: Front Matter Note\nslug: front-matter-note\n---\n\nBody.\n",
        );
        // A bare Obsidian note in a normal subfolder.
        vault.write("notes/Bare Note.md", "Plain body with [[link]].\n");
        // A note inside a `.obsidian` dir that MUST be skipped.
        vault.write(".obsidian/workspace.md", "---\ntitle: Config\n---\nnope\n");
        // A dotfile that MUST be skipped even at the root.
        vault.write(".hidden.md", "---\ntitle: Hidden\n---\nnope\n");
        // A non-Markdown file that MUST be ignored.
        vault.write("notes/image-note.txt", "not markdown\n");

        let scan = collect_notes(&vault.path).unwrap();
        let notes = scan.notes;
        assert!(
            scan.load_failures.is_empty(),
            "unexpected load failures: {:?}",
            scan.load_failures
        );

        // Only the two real notes survive; sorted by path.
        assert_eq!(notes.len(), 2, "got: {notes:?}");

        let slugs: Vec<&str> = notes
            .iter()
            .map(|n| n.document.slug.as_deref().unwrap())
            .collect();
        assert!(slugs.contains(&"front-matter-note"));
        assert!(slugs.contains(&"bare-note"));

        // Nothing from the dot-dir or the dotfile leaked in.
        for note in &notes {
            let path = note.path.to_string_lossy();
            assert!(!path.contains(".obsidian"), "dot-dir leaked: {path}");
            assert!(!path.contains(".hidden"), "dotfile leaked: {path}");
            assert!(!path.ends_with(".txt"), "non-markdown leaked: {path}");
        }

        // The bare note derived its title/slug from the filename; the
        // front-mattered note kept its declared title.
        let bare = notes
            .iter()
            .find(|n| n.document.slug.as_deref() == Some("bare-note"))
            .unwrap();
        assert_eq!(bare.document.title, "Bare Note");
        assert_eq!(bare.document.body, "Plain body with [[link]].");

        let front = notes
            .iter()
            .find(|n| n.document.slug.as_deref() == Some("front-matter-note"))
            .unwrap();
        assert_eq!(front.document.title, "Front Matter Note");
    }

    #[test]
    fn parse_rejects_dry_run_value_flag() {
        // `--server` needs a value.
        let args = ["/vault".to_string(), "--server".to_string()].into_iter();
        assert!(ImportOptions::parse(args).is_err());
    }

    #[test]
    fn parse_rejects_flag_as_server_value() {
        // A following flag must not be silently consumed as the server URL.
        let args = [
            "/vault".to_string(),
            "--server".to_string(),
            "--dry-run".to_string(),
        ]
        .into_iter();
        assert!(ImportOptions::parse(args).is_err());
    }

    #[test]
    fn parse_collects_vault_server_and_dry_run() {
        let args = [
            "/vault".to_string(),
            "--server".to_string(),
            "https://example.com".to_string(),
            "--dry-run".to_string(),
        ]
        .into_iter();
        let opts = ImportOptions::parse(args).unwrap();
        assert_eq!(opts.vault, PathBuf::from("/vault"));
        assert_eq!(opts.server.as_deref(), Some("https://example.com"));
        assert!(opts.dry_run);
    }

    #[test]
    fn parse_requires_vault() {
        let args = std::iter::empty::<String>();
        assert!(ImportOptions::parse(args).is_err());
    }

    #[test]
    fn dry_run_report_is_pure() {
        let vault = TempDir::new("dry");
        vault.write("a.md", "Body A\n");
        vault.write("b.md", "---\ntitle: B\nslug: b\n---\nBody B\n");
        let scan = collect_notes(&vault.path).unwrap();
        // Sending nothing, this must succeed and touch no network.
        assert!(report_dry_run(&scan).is_ok());
    }

    #[test]
    fn scan_collects_per_file_failures_instead_of_aborting() {
        let vault = TempDir::new("collect-failures");
        // A perfectly good note.
        vault.write("good.md", "Body of a good note.\n");
        // A note whose filename slugifies to nothing AND has no front matter,
        // so `derive_document` fails — but the scan must keep the good note and
        // record this as a load failure rather than aborting the whole import.
        vault.write("---.md", "Body with an unslugifiable name.\n");

        let scan = collect_notes(&vault.path).unwrap();

        assert_eq!(scan.notes.len(), 1, "good note must survive: {scan:?}");
        assert_eq!(scan.notes[0].document.title, "good");
        assert_eq!(
            scan.load_failures.len(),
            1,
            "the bad note must be recorded as a failure: {scan:?}"
        );
        assert!(
            scan.load_failures[0].0.ends_with("---.md"),
            "failure path should point at the bad note: {scan:?}"
        );

        // A dry run over a scan with load failures still succeeds (it only
        // reports), and surfaces the failed note.
        assert!(report_dry_run(&scan).is_ok());
    }
}
