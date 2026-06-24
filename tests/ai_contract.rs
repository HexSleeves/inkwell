//! Database-backed contract + eval tests for the semantic layer (card T10).
//!
//! Skipped unless `DATABASE_URL` is set (or forced via `INKWELL_REQUIRE_DB_TESTS=1`),
//! matching the rest of the db-backed suite. The eval tests use the deterministic
//! mock embedder + mock LLM (no API keys), so they are fully reproducible in CI.

mod common;

use axum::body::{Body, to_bytes};
use http::{Method, Request, StatusCode};
use serde_json::Value;
use std::sync::LazyLock;
use tokio::sync::{Mutex, MutexGuard};
use tower::ServiceExt;

/// These tests share one database and `maybe_pool` truncates it on entry, so
/// they must not run concurrently. Cargo runs separate test binaries
/// sequentially; this serializes the tests within this binary.
static DB_TEST_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

async fn db_guard() -> MutexGuard<'static, ()> {
    DB_TEST_LOCK.lock().await
}

/// Create a note via the write API. `published` controls whether it is then
/// published (notes are created as drafts). Returns the created envelope.
async fn create_note(
    router: &axum::Router,
    title: &str,
    body: &str,
    published: bool,
) -> anyhow::Result<Value> {
    let payload = serde_json::json!({ "title": title, "bodyMarkdown": body });
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/documents")
                .header("content-type", "application/json")
                .header("x-api-key", "test-secret-key")
                .body(Body::from(payload.to_string()))?,
        )
        .await?;
    assert_eq!(
        response.status(),
        StatusCode::CREATED,
        "create should succeed"
    );
    let body_bytes = to_bytes(response.into_body(), usize::MAX).await?;
    let envelope: Value = serde_json::from_slice(&body_bytes)?;
    let slug = envelope["slug"].as_str().unwrap().to_string();

    if published {
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri(format!("/documents/{slug}/publish"))
                    .header("x-api-key", "test-secret-key")
                    .body(Body::empty())?,
            )
            .await?;
        assert_eq!(response.status(), StatusCode::OK, "publish should succeed");
    }
    Ok(envelope)
}

async fn get_json(router: &axum::Router, uri: &str) -> anyhow::Result<(StatusCode, Value)> {
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(uri)
                .body(Body::empty())?,
        )
        .await?;
    let status = response.status();
    let body = to_bytes(response.into_body(), usize::MAX).await?;
    let json: Value = if body.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&body)?
    };
    Ok((status, json))
}

// --- FTS search (no keys; the cheapest recall win) --------------------------

#[tokio::test]
async fn fts_search_matches_body_and_ranks_title_first() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(router) = common::maybe_router().await? else {
        return Ok(());
    };

    // Title carries the term (weight A); another note has it only in the body
    // (weight B); a third is unrelated and must not match.
    create_note(
        &router,
        "Postgres Indexing",
        "How relational stores work.",
        true,
    )
    .await?;
    create_note(
        &router,
        "Database Tips",
        "Tuning postgres for write-heavy loads.",
        true,
    )
    .await?;
    create_note(&router, "Sourdough", "Baking bread at home.", true).await?;

    let (status, json) = get_json(&router, "/search?q=postgres&format=json").await?;
    assert_eq!(status, StatusCode::OK);
    let results = json["results"].as_array().unwrap();
    assert_eq!(results.len(), 2, "only the two postgres notes match");
    // Title match (weight A) ranks above the body-only match.
    assert_eq!(results[0]["slug"], "postgres-indexing");
    assert_eq!(json["total"], 2);
    Ok(())
}

#[tokio::test]
async fn fts_search_excludes_drafts_from_public_results() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(router) = common::maybe_router().await? else {
        return Ok(());
    };

    create_note(&router, "Public Widgets", "All about widgets.", true).await?;
    create_note(&router, "Draft Widgets", "Secret widget plans.", false).await?;

    let (status, json) = get_json(&router, "/search?q=widgets&format=json").await?;
    assert_eq!(status, StatusCode::OK);
    let results = json["results"].as_array().unwrap();
    assert_eq!(results.len(), 1, "draft must not surface to public search");
    assert_eq!(results[0]["slug"], "public-widgets");
    Ok(())
}

#[tokio::test]
async fn fts_search_tolerates_punctuation_in_query() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(router) = common::maybe_router().await? else {
        return Ok(());
    };
    create_note(&router, "C++ Notes", "Thoughts on memory safety.", true).await?;

    // websearch_to_tsquery never errors on punctuation; this must 200, not 500.
    let (status, _json) = get_json(&router, "/search?q=memory%20%26%20safety!&format=json").await?;
    assert_eq!(status, StatusCode::OK);
    Ok(())
}

// --- Related notes (mock embedder) ------------------------------------------

#[tokio::test]
async fn related_returns_nearest_published_notes() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(router) = common::maybe_router_with_ai().await? else {
        return Ok(());
    };

    create_note(
        &router,
        "Rust Ownership",
        "Ownership and borrowing rules govern memory in rust.",
        true,
    )
    .await?;
    create_note(
        &router,
        "Borrow Checker",
        "The borrow checker enforces ownership and borrowing in rust.",
        true,
    )
    .await?;
    create_note(
        &router,
        "Pasta Recipes",
        "Boiling pasta with salt and water.",
        true,
    )
    .await?;

    let (status, json) = get_json(&router, "/documents/rust-ownership/related").await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["slug"], "rust-ownership");
    let related = json["related"].as_array().unwrap();
    assert!(!related.is_empty(), "expected at least one related note");
    // The closest note shares the most words; the recipe must not be first.
    assert_eq!(related[0]["slug"], "borrow-checker");
    // The origin note is never in its own related set.
    assert!(related.iter().all(|n| n["slug"] != "rust-ownership"));
    Ok(())
}

#[tokio::test]
async fn related_hides_drafts_from_public_callers() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(router) = common::maybe_router_with_ai().await? else {
        return Ok(());
    };

    create_note(
        &router,
        "Gardening Basics",
        "Soil, water, and sunlight grow healthy plants.",
        true,
    )
    .await?;
    // A draft note that is textually similar must never appear for a public caller.
    create_note(
        &router,
        "Secret Garden Plan",
        "Soil, water, and sunlight plan for the secret garden plants.",
        false,
    )
    .await?;

    let (status, json) = get_json(&router, "/documents/gardening-basics/related").await?;
    assert_eq!(status, StatusCode::OK);
    let related = json["related"].as_array().unwrap();
    assert!(
        related.iter().all(|n| n["slug"] != "secret-garden-plan"),
        "draft must never appear in public related results (no-draft-leak)"
    );
    Ok(())
}

#[tokio::test]
async fn related_404s_for_unknown_or_draft_slug() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(router) = common::maybe_router_with_ai().await? else {
        return Ok(());
    };
    create_note(&router, "Hidden", "A draft note.", false).await?;

    let (status, _json) = get_json(&router, "/documents/nope/related").await?;
    assert_eq!(status, StatusCode::NOT_FOUND);
    // A draft is invisible to a public caller, so /related 404s rather than leaking it.
    let (status, _json) = get_json(&router, "/documents/hidden/related").await?;
    assert_eq!(status, StatusCode::NOT_FOUND);
    Ok(())
}

// --- ask-your-garden eval suite (mock embedder + mock LLM) ------------------

#[tokio::test]
async fn ask_known_answer_retrieves_and_cites_the_right_note() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(router) = common::maybe_router_with_ai().await? else {
        return Ok(());
    };

    create_note(
        &router,
        "Compost Guide",
        "Composting turns kitchen scraps into rich garden soil over several weeks.",
        true,
    )
    .await?;
    create_note(
        &router,
        "Bicycle Maintenance",
        "Oiling a bicycle chain keeps the drivetrain smooth.",
        true,
    )
    .await?;

    let (status, json) = get_json(&router, "/ask?q=how%20does%20composting%20make%20soil").await?;
    assert_eq!(status, StatusCode::OK);
    let answer = json["answer"].as_str().unwrap();
    assert!(
        !answer.contains(inkwell::ai::NO_ANSWER_MARKER),
        "a relevant note exists, so the model should answer"
    );
    let citations = json["citations"].as_array().unwrap();
    assert!(
        !citations.is_empty(),
        "an answer must cite its source notes"
    );
    assert_eq!(
        citations[0]["slug"], "compost-guide",
        "the compost note should be the top citation, not the bicycle note"
    );
    Ok(())
}

#[tokio::test]
async fn ask_no_answer_refuses_cleanly_without_relevant_notes() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(router) = common::maybe_router_with_ai().await? else {
        return Ok(());
    };

    // No notes at all → retrieval is empty → the model must refuse, not hallucinate.
    let (status, json) =
        get_json(&router, "/ask?q=what%20is%20the%20capital%20of%20france").await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["answer"], inkwell::ai::NO_ANSWER_MARKER);
    assert!(
        json["citations"].as_array().unwrap().is_empty(),
        "a clean refusal cites nothing"
    );
    Ok(())
}

#[tokio::test]
async fn ask_never_retrieves_or_cites_a_draft_note() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(router) = common::maybe_router_with_ai().await? else {
        return Ok(());
    };

    // Only a DRAFT note matches the question. A public ask must not retrieve it,
    // so it refuses cleanly and cites nothing (no-draft-leak).
    create_note(
        &router,
        "Unreleased Feature",
        "The quantum flux capacitor ships next quarter with warp coils.",
        false,
    )
    .await?;

    let (status, json) = get_json(
        &router,
        "/ask?q=tell%20me%20about%20the%20quantum%20flux%20capacitor",
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    let answer = json["answer"].as_str().unwrap();
    assert_eq!(
        answer,
        inkwell::ai::NO_ANSWER_MARKER,
        "a draft must not ground a public answer"
    );
    assert!(
        json["citations"].as_array().unwrap().is_empty(),
        "a draft must never appear as a citation"
    );
    Ok(())
}

#[tokio::test]
async fn ask_empty_query_is_a_bad_request() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(router) = common::maybe_router_with_ai().await? else {
        return Ok(());
    };
    let (status, _json) = get_json(&router, "/ask?q=").await?;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    Ok(())
}

// MAX_ASK_QUERY_CHARS in src/http/ai.rs is 1_000 (not public). These assert the
// guard rejects an over-cap question with a 400 BEFORE any provider work — so no
// notes need to exist for retrieval/synthesis to run.
#[tokio::test]
async fn ask_rejects_overlong_get_query_before_provider_work() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(router) = common::maybe_router_with_ai().await? else {
        return Ok(());
    };
    let overlong = "a".repeat(1_001);
    let (status, _json) = get_json(&router, &format!("/ask?q={overlong}")).await?;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    Ok(())
}

#[tokio::test]
async fn ask_rejects_overlong_post_query_before_provider_work() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(router) = common::maybe_router_with_ai().await? else {
        return Ok(());
    };
    let payload = serde_json::json!({ "q": "a".repeat(1_001) });
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/ask")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&payload)?))?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    Ok(())
}

#[tokio::test]
async fn ask_reports_not_configured_without_anthropic_key() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    // The default test router has no LLM wired in (mirrors no ANTHROPIC_API_KEY).
    let Some(router) = common::maybe_router().await? else {
        return Ok(());
    };
    create_note(&router, "Anything", "Some published content.", true).await?;

    let (status, json) = get_json(&router, "/ask?q=anything").await?;
    assert_eq!(
        status,
        StatusCode::OK,
        "must not 500 when AI is unconfigured"
    );
    assert!(
        json["answer"].as_str().unwrap().contains("not configured"),
        "expected a clear not-configured message, got: {}",
        json["answer"]
    );
    Ok(())
}

#[tokio::test]
async fn ask_accepts_post_with_json_body() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(router) = common::maybe_router_with_ai().await? else {
        return Ok(());
    };
    create_note(
        &router,
        "Tea Brewing",
        "Steep green tea for two minutes in hot water.",
        true,
    )
    .await?;

    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/ask")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"q":"how long to steep tea"}"#))?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await?;
    let json: Value = serde_json::from_slice(&body)?;
    assert_eq!(json["query"], "how long to steep tea");
    Ok(())
}
