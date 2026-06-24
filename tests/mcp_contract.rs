//! Client-independent MCP round-trip test (card T6).
//!
//! This does NOT shell out to an external MCP client. It binds the real inkwell
//! axum router on an ephemeral port, points the MCP server's `InkwellClient` at
//! it (authenticating with the *MCP* key), and invokes the tools in-process,
//! asserting the create -> read round-trip, the stale-update (wrong
//! `expected_version`) stale-write error, and that search/list surface the note.
//!
//! Gated on `DATABASE_URL` via the shared harness, like the other db-backed
//! contract tests; it runs in CI which provisions Postgres.

mod common;

use inkwell::client::InkwellClient;
use inkwell::mcp::{
    CreateNoteArgs, InkwellMcpServer, ReadNoteArgs, SearchNotesArgs, UpdateNoteArgs,
};
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::CallToolResult;

/// Pull the structured JSON back out of a tool result. `Content::json` stores
/// the payload as a JSON string in a text content item, so we parse it back.
fn result_json(result: &CallToolResult) -> serde_json::Value {
    let text = result
        .content
        .first()
        .and_then(|c| c.as_text())
        .map(|t| t.text.as_str())
        .expect("tool result should carry a text content item");
    serde_json::from_str(text).expect("tool result content should be valid JSON")
}

#[tokio::test]
async fn mcp_round_trip_create_read_search_and_stale_update() -> anyhow::Result<()> {
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };
    let config = common::test_config(std::env::var("DATABASE_URL")?);
    let router = inkwell::http::router::build_router(config, pool);

    // Bind the real router on an ephemeral port and serve it in the background.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let _server = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    let base = format!("http://{addr}");

    // The MCP server authenticates with `INKWELL_API_KEY` (slice 4 retired the
    // separate MCP key). In production this is a scoped token; the test uses the
    // shared admin key the test router is configured with.
    let client = InkwellClient::new(base, "test-secret-key")?;
    let server = InkwellMcpServer::new(client);

    // create_note -> returns slug + version.
    let created = server
        .create_note(Parameters(CreateNoteArgs {
            title: "MCP Round Trip".to_string(),
            body: "# Hello\n\nFrom the MCP server.".to_string(),
            slug: None,
            tags: Some(vec!["mcp".to_string(), "test".to_string()]),
            growth: None,
        }))
        .await
        .expect("create_note should succeed");
    let created = result_json(&created);
    let slug = created["slug"].as_str().expect("created slug").to_string();
    assert_eq!(slug, "mcp-round-trip");
    let version = created["version"].as_i64().expect("created version");

    // read_note -> round-trips title/body/tags/version.
    let read = server
        .read_note(Parameters(ReadNoteArgs { slug: slug.clone() }))
        .await
        .expect("read_note should succeed");
    let read = result_json(&read);
    assert_eq!(read["found"], true);
    assert_eq!(read["note"]["title"], "MCP Round Trip");
    assert_eq!(
        read["note"]["bodyMarkdown"],
        "# Hello\n\nFrom the MCP server."
    );
    assert_eq!(read["note"]["version"], version);
    let read_tags: Vec<String> = serde_json::from_value(read["note"]["tags"].clone())?;
    assert!(read_tags.contains(&"mcp".to_string()));

    // read_note on a missing slug -> found=false (not an error).
    let missing = server
        .read_note(Parameters(ReadNoteArgs {
            slug: "does-not-exist".to_string(),
        }))
        .await
        .expect("read_note should succeed for a missing slug");
    assert_eq!(result_json(&missing)["found"], false);

    // list_notes -> includes the created note.
    let listed = result_json(
        &server
            .list_notes()
            .await
            .expect("list_notes should succeed"),
    );
    let list_has = listed["notes"]
        .as_array()
        .unwrap()
        .iter()
        .any(|n| n["slug"] == slug.as_str());
    assert!(list_has, "list_notes should include the created note");

    // search_notes -> finds the note by a body term.
    let searched = result_json(
        &server
            .search_notes(Parameters(SearchNotesArgs {
                query: "MCP server".to_string(),
            }))
            .await
            .expect("search_notes should succeed"),
    );
    let search_has = searched["notes"]
        .as_array()
        .unwrap()
        .iter()
        .any(|n| n["slug"] == slug.as_str());
    assert!(search_has, "search_notes should find the created note");

    // A correct-version update succeeds and bumps the version.
    let updated = result_json(
        &server
            .update_note(Parameters(UpdateNoteArgs {
                slug: slug.clone(),
                expected_version: version,
                body: Some("# Hello\n\nEdited body.".to_string()),
                title: None,
                tags: None,
            }))
            .await
            .expect("a correctly-versioned update should succeed"),
    );
    let new_version = updated["version"].as_i64().expect("updated version");
    assert!(
        new_version > version,
        "version should advance after an update"
    );

    // A STALE update (the now-outdated version) must surface a stale-write error.
    let stale = server
        .update_note(Parameters(UpdateNoteArgs {
            slug: slug.clone(),
            expected_version: version, // outdated: the note is now at new_version
            body: Some("# Hello\n\nConflicting edit.".to_string()),
            title: None,
            tags: None,
        }))
        .await;
    let err = stale.expect_err("a stale update must be rejected");
    let message = err.to_string().to_lowercase();
    assert!(
        message.contains("stale write"),
        "expected a stale-write conflict error, got: {message}"
    );

    Ok(())
}
