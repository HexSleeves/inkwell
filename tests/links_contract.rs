//! Database-backed contract tests for the P0 link-graph foundation.
//!
//! Skipped unless `DATABASE_URL` is set (or forced via `INKWELL_REQUIRE_DB_TESTS=1`),
//! matching the rest of the db-backed suite.

mod common;

use inkwell::db::documents::{
    create_document, get_document_by_slug, set_rendered_html, update_document_by_slug,
};
use inkwell::db::links::{
    Backlink, LinkType, MAX_GRAPH_NODES, NewLink, TargetKind, Visibility, backlinks, garden_graph,
    insert_link, note_neighborhood, notes_to_rerender, resolve_existing_slugs,
};
use inkwell::domain::document::{
    DocumentPatch, DocumentStatus, GrowthStage, NewDocument, StatusFilter,
};
use inkwell::garden::{
    affected_sources, backfill_after_change, persist_source_edges, render_and_resolve,
    rerender_sources,
};
use sqlx::Postgres;
use std::sync::LazyLock;
use tokio::sync::{Mutex, MutexGuard};

/// These tests share one database and `maybe_pool` truncates it on entry, so
/// they must not run concurrently (libtest runs a binary's tests on parallel
/// threads). Hold this lock for the whole test to serialize them. Cargo already
/// runs separate test binaries sequentially, so this is sufficient.
static DB_TEST_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

async fn db_guard() -> MutexGuard<'static, ()> {
    DB_TEST_LOCK.lock().await
}

fn new_doc(slug: &str) -> NewDocument {
    NewDocument {
        slug: slug.to_string(),
        title: format!("Title {slug}"),
        body_markdown: format!("# {slug}"),
        rendered_html: format!("<h1>{slug}</h1>"),
        status: Some(DocumentStatus::Published),
        growth: None,
        tags: Vec::new(),
        owner_id: None,
    }
}

/// A published document whose body is `body` (so re-render reflects its real
/// content). Stored HTML starts as a placeholder; tests set it explicitly.
fn doc_with_body(slug: &str, body: &str) -> NewDocument {
    NewDocument {
        slug: slug.to_string(),
        title: format!("Title {slug}"),
        body_markdown: body.to_string(),
        rendered_html: "<p>placeholder</p>".to_string(),
        status: Some(DocumentStatus::Published),
        growth: None,
        tags: Vec::new(),
        owner_id: None,
    }
}

#[tokio::test]
async fn notes_to_rerender_returns_resolved_and_matching_stub_sources() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };

    // Target note plus three potential sources.
    let target = create_document(&pool, new_doc("target")).await?;
    let resolved_source = create_document(&pool, new_doc("resolved-source")).await?;
    let stub_source = create_document(&pool, new_doc("stub-source")).await?;
    let unrelated_source = create_document(&pool, new_doc("unrelated-source")).await?;

    // resolved-source resolves directly to target.
    insert_link(
        &pool,
        NewLink {
            source_note_id: resolved_source.id,
            target_kind: TargetKind::Internal,
            target_note_id: Some(target.id),
            target_url: None,
            target_text: Some("target".to_string()),
            link_type: LinkType::Wikilink,
            context_snippet: Some("see [[target]]".to_string()),
            resolved: true,
        },
    )
    .await?;

    // stub-source has an unresolved [[target]] stub: a future create/rename of
    // "target" must re-render it.
    insert_link(
        &pool,
        NewLink {
            source_note_id: stub_source.id,
            target_kind: TargetKind::Internal,
            target_note_id: None,
            target_url: None,
            target_text: Some("target".to_string()),
            link_type: LinkType::Wikilink,
            context_snippet: Some("a [[target]] that does not exist yet".to_string()),
            resolved: false,
        },
    )
    .await?;

    // unrelated-source points at a different note and must NOT be in the set.
    insert_link(
        &pool,
        NewLink {
            source_note_id: unrelated_source.id,
            target_kind: TargetKind::Internal,
            target_note_id: Some(resolved_source.id),
            target_url: None,
            target_text: Some("resolved-source".to_string()),
            link_type: LinkType::Wikilink,
            context_snippet: None,
            resolved: true,
        },
    )
    .await?;

    let mut affected = notes_to_rerender(&pool, target.id, "target").await?;
    affected.sort();
    let mut expected = vec![resolved_source.id, stub_source.id];
    expected.sort();

    assert_eq!(
        affected, expected,
        "re-render set must be exactly the resolved source and the matching stub source"
    );
    assert!(
        !affected.contains(&unrelated_source.id),
        "an unrelated source must never be re-rendered"
    );

    Ok(())
}

#[tokio::test]
async fn deleting_a_note_cascades_to_its_outbound_edges() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };

    let source = create_document(&pool, new_doc("cascade-source")).await?;
    let target = create_document(&pool, new_doc("cascade-target")).await?;
    insert_link(
        &pool,
        NewLink {
            source_note_id: source.id,
            target_kind: TargetKind::Internal,
            target_note_id: Some(target.id),
            target_url: None,
            target_text: Some("cascade-target".to_string()),
            link_type: LinkType::Wikilink,
            context_snippet: None,
            resolved: true,
        },
    )
    .await?;

    sqlx::query("DELETE FROM documents WHERE id = $1")
        .bind(source.id)
        .execute(&pool)
        .await?;

    let remaining: i64 = sqlx::query_scalar::<Postgres, i64>(
        "SELECT count(*)::bigint FROM links WHERE source_note_id = $1",
    )
    .bind(source.id)
    .fetch_one(&pool)
    .await?;
    assert_eq!(
        remaining, 0,
        "deleting a source note must cascade-delete its edges"
    );

    Ok(())
}

#[tokio::test]
async fn resolve_existing_slugs_respects_visibility() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };

    create_document(&pool, new_doc("pub-note")).await?;
    let mut draft = new_doc("draft-note");
    draft.status = Some(DocumentStatus::Draft);
    create_document(&pool, draft).await?;

    let requested = vec![
        "pub-note".to_string(),
        "draft-note".to_string(),
        "missing".to_string(),
    ];

    let public = resolve_existing_slugs(&pool, &requested, Visibility::Public).await?;
    assert!(public.contains("pub-note"), "published note resolves");
    assert!(
        !public.contains("draft-note"),
        "drafts must not resolve for public callers"
    );
    assert!(!public.contains("missing"), "absent slug never resolves");

    let all = resolve_existing_slugs(&pool, &requested, Visibility::All).await?;
    assert!(
        all.contains("pub-note") && all.contains("draft-note"),
        "owner visibility resolves drafts too"
    );
    assert!(!all.contains("missing"));

    Ok(())
}

#[tokio::test]
async fn render_and_resolve_renders_links_and_persists_edges() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };

    let target = create_document(&pool, new_doc("target")).await?;
    let source = create_document(&pool, new_doc("source")).await?;

    let (html, refs) = render_and_resolve(&pool, "see [[target]] and [[Missing One]] here").await?;

    // Resolved target renders a plain anchor; the missing one renders a stub.
    assert!(html.contains("href=\"/target\""), "resolved link renders");
    assert!(
        html.contains("class=\"stub\""),
        "missing link renders a stub"
    );
    assert!(
        html.contains("href=\"/missing-one\""),
        "stub slug is normalized"
    );
    assert_eq!(refs.len(), 2);

    persist_source_edges(&pool, source.id, &refs).await?;

    let rows: Vec<(String, bool)> = sqlx::query_as::<Postgres, (String, bool)>(
        "SELECT target_text, resolved FROM links WHERE source_note_id = $1 ORDER BY target_text",
    )
    .bind(source.id)
    .fetch_all(&pool)
    .await?;
    assert_eq!(
        rows,
        vec![
            ("missing-one".to_string(), false),
            ("target".to_string(), true),
        ]
    );

    // The source now resolves to the target, so renaming/changing target must
    // re-render the source.
    let affected = notes_to_rerender(&pool, target.id, "target").await?;
    assert!(affected.contains(&source.id));

    Ok(())
}

#[tokio::test]
async fn backfill_lights_up_stub_when_target_appears() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };

    // Source links to a target that does not exist yet → stub.
    let source = create_document(&pool, doc_with_body("src", "see [[target]]")).await?;
    let (html0, refs0) = render_and_resolve(&pool, "see [[target]]").await?;
    set_rendered_html(&pool, source.id, &html0).await?;
    persist_source_edges(&pool, source.id, &refs0).await?;
    assert!(html0.contains("class=\"stub\""), "starts as a stub");

    // Target appears; the create-path backfill lights up the stub.
    let target = create_document(&pool, new_doc("target")).await?;
    backfill_after_change(&pool, target.id, &target.slug).await;

    let html1: String =
        sqlx::query_scalar::<Postgres, String>("SELECT rendered_html FROM documents WHERE id = $1")
            .bind(source.id)
            .fetch_one(&pool)
            .await?;
    assert!(
        html1.contains("href=\"/target\"") && !html1.contains("class=\"stub\""),
        "stub upgraded to a real link"
    );

    let resolved: bool = sqlx::query_scalar::<Postgres, bool>(
        "SELECT resolved FROM links WHERE source_note_id = $1 AND target_text = 'target'",
    )
    .bind(source.id)
    .fetch_one(&pool)
    .await?;
    assert!(resolved, "edge is now resolved");

    Ok(())
}

#[tokio::test]
async fn deleting_a_target_restubs_inbound_links() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };

    let target = create_document(&pool, new_doc("target")).await?;
    let source = create_document(&pool, doc_with_body("src", "see [[target]]")).await?;
    let (html0, refs0) = render_and_resolve(&pool, "see [[target]]").await?;
    set_rendered_html(&pool, source.id, &html0).await?;
    persist_source_edges(&pool, source.id, &refs0).await?;
    assert!(html0.contains("href=\"/target\"") && !html0.contains("class=\"stub\""));

    // Delete the target the way the handler does: capture inbound sources first,
    // delete, then re-render those sources.
    let affected = affected_sources(&pool, target.id, &target.slug).await;
    sqlx::query("DELETE FROM documents WHERE id = $1")
        .bind(target.id)
        .execute(&pool)
        .await?;
    rerender_sources(&pool, &affected).await;

    let html1: String =
        sqlx::query_scalar::<Postgres, String>("SELECT rendered_html FROM documents WHERE id = $1")
            .bind(source.id)
            .fetch_one(&pool)
            .await?;
    assert!(
        html1.contains("class=\"stub\""),
        "link falls back to a stub after the target is deleted"
    );

    let resolved: bool = sqlx::query_scalar::<Postgres, bool>(
        "SELECT resolved FROM links WHERE source_note_id = $1 AND target_text = 'target'",
    )
    .bind(source.id)
    .fetch_one(&pool)
    .await?;
    assert!(!resolved, "edge is unresolved after the target is deleted");

    Ok(())
}

/// Insert one resolved internal wikilink edge from `source` to `target`.
async fn link(
    pool: &sqlx::PgPool,
    source_id: uuid::Uuid,
    target_id: uuid::Uuid,
    target_slug: &str,
    context: &str,
) -> anyhow::Result<()> {
    insert_link(
        pool,
        NewLink {
            source_note_id: source_id,
            target_kind: TargetKind::Internal,
            target_note_id: Some(target_id),
            target_url: None,
            target_text: Some(target_slug.to_string()),
            link_type: LinkType::Wikilink,
            context_snippet: Some(context.to_string()),
            resolved: true,
        },
    )
    .await?;
    Ok(())
}

#[tokio::test]
async fn backlinks_returns_each_linking_source_deduped_and_ordered() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };

    let target = create_document(&pool, new_doc("target")).await?;
    let alpha = create_document(&pool, new_doc("alpha-source")).await?;
    let beta = create_document(&pool, new_doc("beta-source")).await?;

    // beta links once; alpha links TWICE (must dedup to a single backlink).
    link(&pool, beta.id, target.id, "target", "from beta [[target]]").await?;
    link(
        &pool,
        alpha.id,
        target.id,
        "target",
        "first [[target]] mention",
    )
    .await?;
    link(
        &pool,
        alpha.id,
        target.id,
        "target",
        "second [[target]] mention",
    )
    .await?;

    let got = backlinks(&pool, target.id, Visibility::Public).await?;

    // Deduped to one per source, ordered by slug (alpha before beta).
    let slugs: Vec<&str> = got.iter().map(|b| b.source_slug.as_str()).collect();
    assert_eq!(
        slugs,
        vec!["alpha-source", "beta-source"],
        "each source appears once, ordered by slug"
    );
    assert_eq!(got[0].source_title, "Title alpha-source");

    Ok(())
}

#[tokio::test]
async fn backlinks_is_empty_when_no_one_links_to_the_target() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };

    let target = create_document(&pool, new_doc("lonely-target")).await?;
    // A document that links to something ELSE must not appear as a backlink.
    let other = create_document(&pool, new_doc("other")).await?;
    let elsewhere = create_document(&pool, new_doc("elsewhere")).await?;
    link(&pool, other.id, elsewhere.id, "elsewhere", "[[elsewhere]]").await?;

    let got = backlinks(&pool, target.id, Visibility::Public).await?;
    assert_eq!(got, Vec::<Backlink>::new(), "no inbound links ⇒ empty vec");

    Ok(())
}

#[tokio::test]
async fn backlinks_never_leak_a_draft_source_to_public_callers() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };

    // A published target linked-to by one published source and one DRAFT source.
    let target = create_document(&pool, new_doc("target")).await?;
    let published_source = create_document(&pool, new_doc("published-source")).await?;
    let mut draft = new_doc("draft-source");
    draft.status = Some(DocumentStatus::Draft);
    let draft_source = create_document(&pool, draft).await?;

    link(
        &pool,
        published_source.id,
        target.id,
        "target",
        "pub [[target]]",
    )
    .await?;
    link(
        &pool,
        draft_source.id,
        target.id,
        "target",
        "draft [[target]]",
    )
    .await?;

    // Public scope: ONLY the published source — the draft must never leak.
    let public = backlinks(&pool, target.id, Visibility::Public).await?;
    let public_slugs: Vec<&str> = public.iter().map(|b| b.source_slug.as_str()).collect();
    assert_eq!(
        public_slugs,
        vec!["published-source"],
        "public backlinks must omit the draft source (no-draft-leak invariant)"
    );

    // Owner scope: both sources are visible.
    let all = backlinks(&pool, target.id, Visibility::All).await?;
    let all_slugs: Vec<&str> = all.iter().map(|b| b.source_slug.as_str()).collect();
    assert_eq!(
        all_slugs,
        vec!["draft-source", "published-source"],
        "owner backlinks include the draft source too"
    );

    Ok(())
}

#[tokio::test]
async fn updating_a_document_bumps_its_version() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };

    let doc = create_document(&pool, new_doc("bump")).await?;
    update_document_by_slug(
        &pool,
        "bump",
        DocumentPatch {
            title: Some("Bumped".to_string()),
            ..Default::default()
        },
    )
    .await?
    .expect("document exists");

    let version: i64 =
        sqlx::query_scalar::<Postgres, i64>("SELECT version FROM documents WHERE id = $1")
            .bind(doc.id)
            .fetch_one(&pool)
            .await?;
    assert_eq!(version, 2, "an update bumps version from 1 to 2");

    Ok(())
}

#[tokio::test]
async fn documents_carry_a_version_defaulting_to_one() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };

    let doc = create_document(&pool, new_doc("versioned")).await?;
    let version: i64 =
        sqlx::query_scalar::<Postgres, i64>("SELECT version FROM documents WHERE id = $1")
            .bind(doc.id)
            .fetch_one(&pool)
            .await?;
    assert_eq!(version, 1, "new documents start at version 1");

    Ok(())
}

// ---------------------------------------------------------------------------
// T9: bounded link graph
// ---------------------------------------------------------------------------

/// A draft document whose body is `body`.
fn draft_with_body(slug: &str, body: &str) -> NewDocument {
    NewDocument {
        slug: slug.to_string(),
        title: format!("Title {slug}"),
        body_markdown: body.to_string(),
        rendered_html: "<p>placeholder</p>".to_string(),
        status: Some(DocumentStatus::Draft),
        growth: None,
        tags: Vec::new(),
        owner_id: None,
    }
}

#[tokio::test]
async fn garden_graph_never_leaks_a_draft_node_or_edge_to_public() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };

    // pub-a → pub-b (both published) AND pub-a → draft-c (target is a draft).
    let pub_a = create_document(&pool, doc_with_body("pub-a", "[[pub-b]] and [[draft-c]]")).await?;
    let pub_b = create_document(&pool, new_doc("pub-b")).await?;
    let draft_c = create_document(&pool, draft_with_body("draft-c", "lonely draft")).await?;

    link(&pool, pub_a.id, pub_b.id, "pub-b", "[[pub-b]]").await?;
    // An edge whose target is a draft: must never appear at PUBLIC.
    link(&pool, pub_a.id, draft_c.id, "draft-c", "[[draft-c]]").await?;

    let public = garden_graph(&pool, Visibility::Public).await?;
    let node_slugs: Vec<&str> = public.nodes.iter().map(|n| n.slug.as_str()).collect();
    assert!(
        node_slugs.contains(&"pub-a") && node_slugs.contains(&"pub-b"),
        "published nodes are present"
    );
    assert!(
        !node_slugs.contains(&"draft-c"),
        "a draft node must never appear in the public graph"
    );
    // Exactly one edge (pub-a → pub-b); the draft-targeted edge is filtered out.
    assert_eq!(
        public.edges.len(),
        1,
        "an edge touching a draft must never appear in the public graph"
    );
    assert_eq!(public.edges[0].source_slug, "pub-a");
    assert_eq!(public.edges[0].target_slug, "pub-b");
    assert!(
        !public
            .edges
            .iter()
            .any(|e| e.source_slug == "draft-c" || e.target_slug == "draft-c"),
        "no edge may reference the draft"
    );

    // Owner scope (All) sees the draft node AND the draft-targeted edge.
    let all = garden_graph(&pool, Visibility::All).await?;
    let all_slugs: Vec<&str> = all.nodes.iter().map(|n| n.slug.as_str()).collect();
    assert!(
        all_slugs.contains(&"draft-c"),
        "owner visibility includes the draft node"
    );
    assert_eq!(all.edges.len(), 2, "owner visibility includes both edges");

    Ok(())
}

#[tokio::test]
async fn garden_graph_bounds_the_node_count() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };

    // The cap is large; assert the contract holds rather than inserting 500+ rows:
    // a modest published set is fully returned and never exceeds the cap.
    for i in 0..5 {
        create_document(&pool, new_doc(&format!("node-{i:02}"))).await?;
    }
    let graph = garden_graph(&pool, Visibility::Public).await?;
    assert_eq!(graph.nodes.len(), 5, "all published nodes are returned");
    assert!(
        (graph.nodes.len() as i64) <= MAX_GRAPH_NODES,
        "node count never exceeds the hard cap"
    );
    // Deterministic slug ordering.
    let mut sorted = graph.nodes.clone();
    sorted.sort_by(|a, b| a.slug.cmp(&b.slug));
    assert_eq!(graph.nodes, sorted, "nodes are ordered by slug");

    Ok(())
}

#[tokio::test]
async fn garden_graph_truncates_at_the_node_cap() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };

    // Insert one more than the cap so the LIMIT actually fires. Bulk-insert via
    // generate_series to keep this exact-bound test cheap (no per-row fan-out).
    sqlx::query(
        "INSERT INTO documents (slug, title, body_markdown, rendered_html, status) \
         SELECT 'cap-' || to_char(g, 'FM0000'), 'Cap ' || g, '# x', '<h1>x</h1>', 'published' \
         FROM generate_series(1, $1) AS g",
    )
    .bind(MAX_GRAPH_NODES + 1)
    .execute(&pool)
    .await?;

    let graph = garden_graph(&pool, Visibility::Public).await?;
    assert_eq!(
        graph.nodes.len() as i64,
        MAX_GRAPH_NODES,
        "node count is hard-bounded to exactly the cap when the garden exceeds it"
    );

    Ok(())
}

#[tokio::test]
async fn note_neighborhood_is_one_hop_and_visibility_filtered() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };

    // center → neighbor (published); center → draft-neighbor (draft); far is two hops.
    let center = create_document(&pool, new_doc("center")).await?;
    let neighbor = create_document(&pool, new_doc("neighbor")).await?;
    let far = create_document(&pool, new_doc("far")).await?;
    let draft_neighbor = create_document(&pool, draft_with_body("draft-neighbor", "x")).await?;

    link(&pool, center.id, neighbor.id, "neighbor", "[[neighbor]]").await?;
    link(&pool, neighbor.id, far.id, "far", "[[far]]").await?;
    link(
        &pool,
        center.id,
        draft_neighbor.id,
        "draft-neighbor",
        "[[draft-neighbor]]",
    )
    .await?;

    let hood = note_neighborhood(&pool, "center", Visibility::Public).await?;
    let slugs: Vec<&str> = hood.nodes.iter().map(|n| n.slug.as_str()).collect();
    assert!(slugs.contains(&"center") && slugs.contains(&"neighbor"));
    assert!(
        !slugs.contains(&"far"),
        "a two-hop node is outside the one-hop neighborhood"
    );
    assert!(
        !slugs.contains(&"draft-neighbor"),
        "a draft neighbor must never leak into a public neighborhood"
    );

    Ok(())
}

// ---------------------------------------------------------------------------
// T9: note growth stages
// ---------------------------------------------------------------------------

#[tokio::test]
async fn growth_defaults_to_seedling_and_round_trips_through_create_and_update()
-> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };

    // Default: a note created without an explicit growth is a seedling.
    let created = create_document(&pool, new_doc("grows")).await?;
    assert_eq!(
        created.growth,
        GrowthStage::Seedling,
        "growth defaults to seedling"
    );

    // Create with an explicit stage round-trips.
    let mut input = new_doc("evergreen-note");
    input.growth = Some(GrowthStage::Evergreen);
    let ever = create_document(&pool, input).await?;
    assert_eq!(ever.growth, GrowthStage::Evergreen);

    // Update promotes the stage.
    let updated = update_document_by_slug(
        &pool,
        "grows",
        DocumentPatch {
            growth: Some(GrowthStage::Budding),
            ..Default::default()
        },
    )
    .await?
    .expect("document exists");
    assert_eq!(updated.growth, GrowthStage::Budding);

    // Re-read confirms persistence.
    let reread = get_document_by_slug(&pool, "grows", StatusFilter::default())
        .await?
        .expect("document exists");
    assert_eq!(reread.growth, GrowthStage::Budding);

    Ok(())
}

// ---------------------------------------------------------------------------
// T9: transclusion (![[embed]])
// ---------------------------------------------------------------------------

#[tokio::test]
async fn published_embed_expands_to_its_content() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };

    create_document(
        &pool,
        doc_with_body("embedded", "This is the **embedded** body text."),
    )
    .await?;

    let (html, _refs) = render_and_resolve(&pool, "Host note.\n\n![[embedded]]").await?;
    assert!(
        html.contains("body text"),
        "a published embed expands to its rendered content"
    );
    assert!(
        html.contains("<strong>embedded</strong>"),
        "the embed content is rendered as markdown"
    );
    assert!(
        html.contains(r#"class="embed""#),
        "the transcluded content is wrapped in an embed figure"
    );
    assert!(!html.contains("![["), "raw embed markup is consumed");

    Ok(())
}

#[tokio::test]
async fn draft_embed_renders_only_a_placeholder_no_leak() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };

    // The draft body carries a unique sentinel that must NEVER appear in output.
    create_document(
        &pool,
        draft_with_body("secret-draft", "TOP_SECRET_DRAFT_BODY do not leak"),
    )
    .await?;

    let (html, _refs) = render_and_resolve(&pool, "Host.\n\n![[secret-draft]]").await?;
    assert!(
        !html.contains("TOP_SECRET_DRAFT_BODY"),
        "a draft embed must never leak the draft's body (no-draft-leak invariant)"
    );
    assert!(
        html.contains("Embed omitted"),
        "a draft embed renders a neutral placeholder"
    );

    Ok(())
}

#[tokio::test]
async fn missing_embed_renders_only_a_placeholder() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };

    let (html, _refs) = render_and_resolve(&pool, "![[does-not-exist]]").await?;
    assert!(
        html.contains("Embed omitted"),
        "missing target → placeholder"
    );
    assert!(!html.contains("![["), "raw markup is consumed");

    Ok(())
}

#[tokio::test]
async fn cyclic_embed_terminates_with_a_placeholder() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };

    // Direct self-embed and a two-note cycle: rendering must terminate (no hang)
    // and emit a placeholder rather than looping.
    create_document(&pool, doc_with_body("loop-a", "A body. ![[loop-b]]")).await?;
    create_document(&pool, doc_with_body("loop-b", "B body. ![[loop-a]]")).await?;

    // Render loop-a's body directly: it embeds loop-b, which embeds loop-a back.
    let (html, _refs) = render_and_resolve(&pool, "Start. ![[loop-a]]").await?;
    assert!(
        html.contains("Embed omitted"),
        "a cyclic embed terminates with a placeholder"
    );
    // The first level still expands (A body, then B body) but the back-edge stops.
    assert!(html.contains("A body"), "the first expansion level renders");

    Ok(())
}

#[tokio::test]
async fn embed_inside_code_stays_literal() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };

    create_document(&pool, doc_with_body("real", "REAL_EMBED_CONTENT")).await?;

    let (html, _refs) = render_and_resolve(&pool, "`![[real]]`").await?;
    assert!(html.contains("<code>"), "the code span is preserved");
    assert!(
        !html.contains("REAL_EMBED_CONTENT"),
        "an embed inside code is never expanded"
    );

    Ok(())
}
