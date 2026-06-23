mod common;

use inkwell::cli::author::{self, NewOptions};
use inkwell::client::{InkwellClient, PushAction};
use inkwell::http::router::build_router;

/// Returns true if the API list envelope contains a document with `slug`.
fn list_contains(list: &serde_json::Value, slug: &str) -> bool {
    list["documents"]
        .as_array()
        .map(|docs| docs.iter().any(|doc| doc["slug"] == slug))
        .unwrap_or(false)
}

/// Exercises the full authoring round trip against a real local server:
/// scaffold (`new`) -> create (`push`) -> `publish` -> appears in the public,
/// unauthenticated list.
#[tokio::test]
async fn author_new_push_publish_appears_in_public_list() -> anyhow::Result<()> {
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };
    let config = common::test_config(std::env::var("DATABASE_URL")?);
    let router = build_router(config, pool);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let server = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    let base = format!("http://{addr}");
    let public = reqwest::Client::new();

    // `author new`: scaffold a Markdown file and parse it back.
    let opts = NewOptions {
        title: "Author Flow Demo".to_string(),
        slug: None,
        status: "draft".to_string(),
        tags: vec!["cli".to_string()],
    };
    let rendered = author::render_new_document(&opts)?;
    let doc = author::parse_markdown(&rendered)?;
    let slug = doc.effective_slug()?;
    assert_eq!(slug, "author-flow-demo");
    let input = doc.to_input()?;

    let client = InkwellClient::new(base.clone(), "test-secret-key")?;

    // `author push`: first push creates the document as a draft.
    let (action, summary) = client.push(&input).await?;
    assert_eq!(action, PushAction::Created);
    assert_eq!(summary.slug, slug);
    assert_eq!(summary.status, "draft");

    // A draft must NOT appear in the public, unauthenticated list.
    let before = public
        .get(format!("{base}/documents"))
        .send()
        .await?
        .json::<serde_json::Value>()
        .await?;
    assert!(
        !list_contains(&before, &slug),
        "draft leaked into public list"
    );

    // `author publish`: flips status to published.
    let summary = client.publish(&slug).await?;
    assert_eq!(summary.status, "published");

    // Now it appears in the public list.
    let after = public
        .get(format!("{base}/documents"))
        .send()
        .await?
        .json::<serde_json::Value>()
        .await?;
    assert!(
        list_contains(&after, &slug),
        "published document missing from public list"
    );

    // Pushing the same file again updates the existing document.
    let (action, _) = client.push(&input).await?;
    assert_eq!(action, PushAction::Updated);

    // `author unpublish`: returns it to draft and drops it from the public list.
    let summary = client.unpublish(&slug).await?;
    assert_eq!(summary.status, "draft");
    let final_list = public
        .get(format!("{base}/documents"))
        .send()
        .await?
        .json::<serde_json::Value>()
        .await?;
    assert!(!list_contains(&final_list, &slug));

    server.abort();
    Ok(())
}

/// A missing slug yields a clear, non-panicking error (404 path).
#[tokio::test]
async fn publish_unknown_slug_errors_cleanly() -> anyhow::Result<()> {
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };
    let config = common::test_config(std::env::var("DATABASE_URL")?);
    let router = build_router(config, pool);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let server = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });

    let client = InkwellClient::new(format!("http://{addr}"), "test-secret-key")?;
    let err = client.publish("does-not-exist").await.unwrap_err();
    assert!(err.to_string().contains("404"), "unexpected error: {err}");

    server.abort();
    Ok(())
}

/// A bad API key produces a clear 401 message rather than a panic.
#[tokio::test]
async fn wrong_api_key_reports_unauthorized() -> anyhow::Result<()> {
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };
    let config = common::test_config(std::env::var("DATABASE_URL")?);
    let router = build_router(config, pool);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let server = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });

    let client = InkwellClient::new(format!("http://{addr}"), "wrong-key")?;
    let doc = author::ParsedDocument {
        title: "Nope".to_string(),
        slug: Some("nope".to_string()),
        status: None,
        growth: None,
        tags: vec![],
        body: "x".to_string(),
    };
    let err = client.push(&doc.to_input()?).await.unwrap_err();
    assert!(err.to_string().contains("401"), "unexpected error: {err}");

    server.abort();
    Ok(())
}
