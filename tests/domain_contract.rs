use inkwell::domain::document::DocumentStatus;
use inkwell::domain::slug::{is_valid_slug, slugify};
use inkwell::domain::tags::normalize_tags;

#[test]
fn slugify_matches_expected_behavior() {
    assert_eq!(slugify("Hello World"), "hello-world");
    assert_eq!(slugify("Crème brûlée"), "creme-brulee");
    assert_eq!(slugify("!!!"), "");
    assert!(is_valid_slug("hello-world"));
    assert!(!is_valid_slug("Not Valid"));
}

#[test]
fn tags_normalize_and_dedupe() {
    let tags = normalize_tags(&[
        "Rust".to_string(),
        "rust".to_string(),
        " Postgres ".to_string(),
    ])
    .unwrap();
    assert_eq!(tags, vec!["rust", "postgres"]);
}

#[test]
fn status_serializes_lowercase() {
    assert_eq!(
        serde_json::to_string(&DocumentStatus::Draft).unwrap(),
        "\"draft\""
    );
    assert_eq!(
        serde_json::to_string(&DocumentStatus::Published).unwrap(),
        "\"published\""
    );
}
