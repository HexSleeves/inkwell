//! Compose env-forwarding contract (CYP-50).
//!
//! `docker-compose.yml` forwards a *fixed* list of variables into the `app`
//! container. A flag the binary reads but Compose does not list is unreachable
//! on the documented deploy path — and, worse, setting it in `.env` looks like
//! it should work. That is exactly how the v0.2 authoring UI (`INKWELL_BROWSER_LOGIN`)
//! and the `/metrics` endpoint (`INKWELL_METRICS_ENABLED`) shipped unreachable
//! on Compose. These are plain text assertions (no DB, no container) so the gap
//! is caught by `cargo test` rather than by a release-eve QA sweep.

const COMPOSE: &str = include_str!("../docker-compose.yml");
const ENV_EXAMPLE: &str = include_str!("../.env.example");

/// Variables the `app` service must forward from the shell/`.env`.
const REQUIRED_FORWARDED: &[&str] = &[
    "INKWELL_API_KEY",
    "INKWELL_SITE_URL",
    "INKWELL_BROWSER_LOGIN",
    "INKWELL_METRICS_ENABLED",
    "INKWELL_METRICS_TOKEN",
    "INKWELL_MEDIA_BACKEND",
    "INKWELL_MEDIA_DIR",
    "INKWELL_MEDIA_MAX_BYTES",
];

/// Feature flags that must stay off in a stock stack: the Compose default has to
/// be the same "off" the binary itself defaults to, so adding the passthrough
/// never changes an existing deployment's behavior.
const OFF_BY_DEFAULT: &[(&str, &str)] = &[
    ("INKWELL_BROWSER_LOGIN", "${INKWELL_BROWSER_LOGIN:-false}"),
    (
        "INKWELL_METRICS_ENABLED",
        "${INKWELL_METRICS_ENABLED:-false}",
    ),
    ("INKWELL_METRICS_TOKEN", "${INKWELL_METRICS_TOKEN:-}"),
];

#[test]
fn compose_forwards_every_required_variable() {
    for key in REQUIRED_FORWARDED {
        assert!(
            COMPOSE.contains(&format!("{key}:")),
            "docker-compose.yml does not forward {key} to the app service, so setting it in .env has no effect"
        );
    }
}

#[test]
fn compose_feature_flags_default_to_off() {
    for (key, expected) in OFF_BY_DEFAULT {
        assert!(
            COMPOSE.contains(expected),
            "{key} must be forwarded as `{expected}` so a stock Compose stack keeps the binary's off-by-default behavior"
        );
    }
}

#[test]
fn env_example_documents_the_forwarded_feature_flags() {
    for key in [
        "INKWELL_BROWSER_LOGIN",
        "INKWELL_METRICS_ENABLED",
        "INKWELL_MEDIA_BACKEND",
    ] {
        assert!(
            ENV_EXAMPLE.contains(key),
            ".env.example does not mention {key}, so operators have no way to discover it"
        );
    }
}
