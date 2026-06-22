//! `inkwell seed [<vault>]` — populate an empty garden with a sample set of
//! interlinked notes (card T8).
//!
//! This is the "one command to a populated garden" payoff: `docker compose up`
//! runs `migrate -> seed -> serve`, and `seed` plants a handful of bundled,
//! cross-linked notes so the demo garden is browsable, searchable, and
//! backlinked out of the box — instead of an empty page.
//!
//! Two properties make it safe to wire into startup:
//!   - **Idempotent.** It no-ops when the `documents` table is non-empty, so a
//!     restart never duplicates the seed and never clobbers a user's content.
//!   - **Direct + published.** Unlike `inkwell author`/`import`, which speak the
//!     HTTP API and create drafts, `seed` writes through the db layer and marks
//!     notes `published`, because drafts are invisible to the public site,
//!     backlinks, and search. It reuses [`garden::render_and_resolve`] so
//!     `[[wikilinks]]` render and edges persist, then [`garden::backfill_after_change`]
//!     so stubs between the seeded notes light up regardless of insert order.
//!
//! Notes are read from a vault directory (default: the bundled
//! `examples/garden`, overridable so the Docker image can point at its baked-in
//! copy) using the same front-matter parser as `inkwell author`.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use sqlx::PgPool;

use crate::cli::author::{ParsedDocument, parse_markdown};
use crate::db::documents::{create_document, set_rendered_html};
use crate::domain::document::{DocumentStatus, NewDocument, StatusFilter};
use crate::garden;

/// Where the bundled sample vault lives inside the Docker image. The compose
/// app copies `examples/garden` here; `inkwell seed` with no argument uses it.
const DEFAULT_VAULT: &str = "examples/garden";

/// Dispatch `inkwell seed [<vault>]`. The optional positional argument is a
/// directory of front-mattered Markdown notes; it defaults to the bundled
/// sample vault.
pub async fn run(pool: &PgPool, mut args: impl Iterator<Item = String>) -> Result<()> {
    let vault = match args.next() {
        Some(arg) if arg.starts_with('-') => {
            bail!("usage: inkwell seed [<vault>]")
        }
        Some(arg) => PathBuf::from(arg),
        None => PathBuf::from(DEFAULT_VAULT),
    };
    if args.next().is_some() {
        bail!("usage: inkwell seed [<vault>]");
    }
    seed(pool, &vault).await
}

/// Seed `pool` from the notes under `vault`, unless the garden already has
/// content. Idempotent: returns `Ok(())` after a no-op when documents exist.
pub async fn seed(pool: &PgPool, vault: &Path) -> Result<()> {
    let existing = crate::db::documents::count_documents(pool, StatusFilter::default())
        .await
        .context("counting existing documents")?;
    if existing > 0 {
        println!("Garden already has {existing} document(s); skipping seed.");
        return Ok(());
    }

    let notes = load_notes(vault)?;
    if notes.is_empty() {
        println!(
            "No sample notes found under {}; nothing to seed.",
            vault.display()
        );
        return Ok(());
    }

    let count = notes.len();
    let mut seeded = Vec::with_capacity(count);
    for note in &notes {
        seeded.push(plant(pool, note).await?);
    }

    // Re-render so wikilinks between seeded notes that were planted *before*
    // their target resolve from stubs into real links and gain backlinks,
    // independent of insertion order.
    for note in &seeded {
        garden::backfill_after_change(pool, note.id, &note.slug).await;
    }

    println!(
        "Seeded {count} published note(s) into the garden from {}.",
        vault.display()
    );
    Ok(())
}

/// A seeded note's identity after insertion, used to drive the backlink
/// re-render fan-out.
struct Seeded {
    id: uuid::Uuid,
    slug: String,
}

/// Insert one parsed note as a published document, rendering its wikilinks and
/// persisting its outbound edges. Returns the row's id/slug for the backfill.
async fn plant(pool: &PgPool, doc: &ParsedDocument) -> Result<Seeded> {
    let slug = doc.effective_slug()?;
    let (rendered_html, refs) = garden::render_and_resolve(pool, &doc.body)
        .await
        .with_context(|| format!("rendering note {slug:?}"))?;

    let created = create_document(
        pool,
        NewDocument {
            slug: slug.clone(),
            title: doc.title.clone(),
            body_markdown: doc.body.clone(),
            rendered_html,
            // Seeded notes MUST be published: drafts are invisible to the public
            // site, backlinks, and search, which would leave the demo empty.
            status: Some(DocumentStatus::Published),
            tags: doc.tags.clone(),
        },
    )
    .await
    .with_context(|| format!("creating note {slug:?}"))?;

    garden::persist_source_edges(pool, created.id, &refs)
        .await
        .with_context(|| format!("persisting edges for {slug:?}"))?;

    // `render_and_resolve` already produced the stored HTML; keep it in sync in
    // case `create_document` defaulting ever diverges (cheap, idempotent).
    set_rendered_html(pool, created.id, &created.rendered_html)
        .await
        .ok();

    Ok(Seeded {
        id: created.id,
        slug: created.slug,
    })
}

/// Read and parse every `*.md` note directly under `vault`, sorted by path so a
/// run is deterministic. Each file must carry author-style front matter.
fn load_notes(vault: &Path) -> Result<Vec<ParsedDocument>> {
    if !vault.exists() {
        bail!(
            "sample vault {} does not exist; pass a vault directory or run from the repo root.",
            vault.display()
        );
    }
    let mut paths: Vec<PathBuf> = std::fs::read_dir(vault)
        .with_context(|| format!("reading vault {}", vault.display()))?
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|p| {
            p.is_file()
                && p.extension()
                    .and_then(|e| e.to_str())
                    .is_some_and(|e| e.eq_ignore_ascii_case("md"))
        })
        .collect();
    paths.sort();

    let mut notes = Vec::with_capacity(paths.len());
    for path in paths {
        let source = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        let doc = parse_markdown(&source).with_context(|| format!("parsing {}", path.display()))?;
        notes.push(doc);
    }
    Ok(notes)
}
