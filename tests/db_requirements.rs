mod common;

#[tokio::test]
async fn require_db_tests_errors_without_database_url() {
    unsafe {
        std::env::set_var("INKWELL_REQUIRE_DB_TESTS", "1");
        std::env::remove_var("DATABASE_URL");
    }

    let error = common::maybe_router()
        .await
        .expect_err("required DB mode should fail without DATABASE_URL");

    assert!(
        error
            .to_string()
            .contains("DATABASE_URL is required for database-backed contract tests"),
        "unexpected error: {error:#}"
    );
}
