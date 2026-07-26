use anyhow::{Result, anyhow};

/// Default Claude model for ask-your-garden answer synthesis. Configurable via
/// `INKWELL_LLM_MODEL`. Per the project's claude-api guidance, `claude-sonnet-4-6`
/// uses adaptive thinking and rejects `temperature`/`top_p`/`top_k`/`budget_tokens`
/// — the LLM client never sends those.
pub const DEFAULT_LLM_MODEL: &str = "claude-sonnet-4-6";

/// Default site title (brand name) when `INKWELL_SITE_TITLE` is unset.
pub const DEFAULT_SITE_TITLE: &str = "Inkwell";

/// Default per-principal (or per-IP) write rate limit, in requests per minute,
/// when `INKWELL_WRITE_RATE_LIMIT` is unset. Generous enough for a human author
/// or an MCP agent doing bulk edits, low enough to blunt abusive write floods.
/// Set the env var to `0` to disable rate limiting entirely.
pub const DEFAULT_WRITE_RATE_LIMIT: u32 = 60;

/// Default directory for the local media backend when `INKWELL_MEDIA_DIR` is
/// unset. Relative to the process working directory; compose mounts a volume
/// here (see `docker-compose.yml` and ADR 0013).
pub const DEFAULT_MEDIA_DIR: &str = "./data/media";

/// Default maximum upload size for `POST /media`: 5 MiB.
pub const DEFAULT_MEDIA_MAX_BYTES: usize = 5 * 1024 * 1024;

/// Hard ceiling for `INKWELL_MEDIA_MAX_BYTES` (256 MiB), matching the
/// `media_size_check` constraint in migration 0025. Uploads are buffered in
/// memory, so an unbounded cap would be a trivial memory-exhaustion lever.
pub const MEDIA_MAX_BYTES_CEILING: usize = 256 * 1024 * 1024;

/// Maximum outbound webhook endpoints (`INKWELL_WEBHOOK_URLS`). One publish
/// fans out to every endpoint, so the list is capped exactly like every other
/// fan-out surface. Raise deliberately, not by accident.
pub const MAX_WEBHOOK_ENDPOINTS: usize = 10;

/// Minimum accepted length for `INKWELL_WEBHOOK_SECRET`. A short shared secret
/// makes the HMAC signature brute-forceable, which defeats the point of signing,
/// so a too-short secret fails startup rather than shipping a weak signature.
pub const MIN_WEBHOOK_SECRET_LEN: usize = 16;

/// Where uploaded media bytes are stored (`INKWELL_MEDIA_BACKEND`). See ADR 0013.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MediaBackend {
    /// Local filesystem under [`Config::media_dir`] — the default.
    Local,
    /// The `media_blobs` Postgres table. For platforms with an ephemeral
    /// filesystem where mounting a volume is impractical.
    Postgres,
}

impl MediaBackend {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Postgres => "postgres",
        }
    }

    /// Parse the env value, or `None` when unrecognised (a startup error — a
    /// typo must not silently pick a backend that loses uploads).
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "local" | "fs" | "filesystem" => Some(Self::Local),
            "postgres" | "pg" | "database" => Some(Self::Postgres),
            _ => None,
        }
    }
}

#[derive(Clone)]
pub struct Config {
    pub database_url: String,
    pub host: String,
    pub port: u16,
    pub api_key: Option<String>,
    pub site_url: Option<String>,
    /// Voyage AI key (`VOYAGE_API_KEY`) for generating note embeddings. When
    /// `None`, embedding generation is a logged no-op and the site still works.
    pub voyage_api_key: Option<String>,
    /// Anthropic key (`ANTHROPIC_API_KEY`) for ask-your-garden answer synthesis.
    /// When `None`, `/ask` returns a clear "AI features not configured" response
    /// instead of 500ing.
    pub anthropic_api_key: Option<String>,
    /// Claude model (`INKWELL_LLM_MODEL`) used for synthesis. Defaults to
    /// [`DEFAULT_LLM_MODEL`].
    pub llm_model: String,
    /// Minimum cosine similarity (`INKWELL_MIN_SIMILARITY`) for vector ANN
    /// search in `/ask`. `0.0` disables filtering and preserves the historical
    /// top-K behavior.
    pub min_similarity: f32,
    /// Whether to SEND Webmentions when a published note links out
    /// (`INKWELL_WEBMENTION_SEND`). Conservative default: **off**. Receiving is
    /// always on; sending is opt-in and, when off, the send code path is fully
    /// inert (no outbound requests). Not a secret, but a behavior toggle.
    pub webmention_send: bool,
    /// Whether to enable browser session login (`INKWELL_BROWSER_LOGIN`).
    /// Conservative default: **off**. When false, the `/auth/login` and
    /// `/auth/logout` routes are not registered (404), no cookie is ever
    /// consulted during authentication, and the existing auth paths are
    /// byte-for-byte unchanged. See ADR 0010.
    pub browser_login: bool,
    /// Write rate limit (`INKWELL_WRITE_RATE_LIMIT`) in requests per minute,
    /// applied per authenticated principal/token (or per client IP when
    /// anonymous) to mutation routes and `/ask`. Reads and the public HTML site
    /// are never throttled. `0` disables limiting. Defaults to
    /// [`DEFAULT_WRITE_RATE_LIMIT`]. See CIL-128 and `src/http/rate_limit.rs`.
    pub write_rate_limit: u32,
    /// Trust `X-Forwarded-For` / `X-Real-IP` when keying the rate limiter by
    /// client IP (`INKWELL_TRUST_FORWARDED_HEADERS`). Conservative default:
    /// **off** — those headers are client-controllable and only safe behind a
    /// proxy that overwrites them (e.g. Railway). When off, IP keying uses the
    /// real peer address, so a directly-exposed instance can't be spoofed.
    pub trust_forwarded_headers: bool,
    /// Human-readable site title used as the brand name in the HTML header,
    /// feed title, and page `<title>` suffix (`INKWELL_SITE_TITLE`). Defaults
    /// to [`DEFAULT_SITE_TITLE`] ("Inkwell"). Non-secret; shown publicly.
    pub site_title: String,
    /// Site-level description surfaced as the index page `<meta name="description">`
    /// and the Atom feed subtitle (`INKWELL_SITE_DESCRIPTION`). Optional;
    /// omitted from generated HTML when absent.
    pub site_description: Option<String>,
    /// Default author name for JSON-LD `author` and Atom feed `<author>` entries
    /// when a document does not specify one (`INKWELL_SITE_AUTHOR`). Optional.
    pub site_author: Option<String>,
    /// URL of an extra stylesheet to load on every public HTML page
    /// (`INKWELL_CUSTOM_CSS_URL`). When set, a `<link rel="stylesheet">` is
    /// injected after the built-in styles so operators can apply a custom theme
    /// without touching source code. Optional; nothing injected when absent.
    pub custom_css_url: Option<String>,
    /// Whether to register the `/metrics` Prometheus endpoint
    /// (`INKWELL_METRICS_ENABLED`). Conservative default: **off** — metrics are
    /// not publicly scrapeable on a default install; the route simply does not
    /// exist. See ADR 0012 and `docs/OBSERVABILITY.md`.
    pub metrics_enabled: bool,
    /// Optional bearer token required to scrape `/metrics`
    /// (`INKWELL_METRICS_TOKEN`). When `None`, an enabled `/metrics` is open and
    /// the operator is relying on network isolation instead. Secret: redacted in
    /// [`Debug`].
    pub metrics_token: Option<String>,
    /// Where uploaded media bytes are stored (`INKWELL_MEDIA_BACKEND`).
    /// Defaults to [`MediaBackend::Local`]. See ADR 0013.
    pub media_backend: MediaBackend,
    /// Root directory for the local media backend (`INKWELL_MEDIA_DIR`).
    /// Defaults to [`DEFAULT_MEDIA_DIR`]. Ignored by the Postgres backend.
    pub media_dir: String,
    /// Maximum accepted upload size in bytes (`INKWELL_MEDIA_MAX_BYTES`).
    /// Defaults to [`DEFAULT_MEDIA_MAX_BYTES`], capped at
    /// [`MEDIA_MAX_BYTES_CEILING`]. Also sets the router's body limit for
    /// `POST /media`, so an over-cap request is refused before it is buffered.
    pub media_max_bytes: usize,
    /// Whether to POST outbound webhooks on publish/unpublish
    /// (`INKWELL_WEBHOOKS_ENABLED`). Conservative default: **off** — with the
    /// flag off the delivery path is fully inert (no payload built, no task
    /// spawned, no outbound request). See `docs/WEBHOOKS.md` and CYP-53.
    pub webhooks_enabled: bool,
    /// Endpoints that receive webhook deliveries (`INKWELL_WEBHOOK_URLS`,
    /// comma-separated). Validated at startup; capped at
    /// [`MAX_WEBHOOK_ENDPOINTS`]. Not secret — endpoint URLs are logged with
    /// each delivery so failures are debuggable.
    pub webhook_urls: Vec<String>,
    /// Shared secret used as the HMAC-SHA256 key over each raw request body
    /// (`INKWELL_WEBHOOK_SECRET`). Secret: redacted in [`Debug`], never logged,
    /// never sent in a header or payload.
    pub webhook_secret: Option<String>,
}

impl std::fmt::Debug for Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never print secrets: `api_key` is the shared write credential and
        // `database_url` may embed a password in the DSN.
        f.debug_struct("Config")
            .field("database_url", &"<redacted>")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("api_key", &self.api_key.as_ref().map(|_| "<redacted>"))
            .field("site_url", &self.site_url)
            .field(
                "voyage_api_key",
                &self.voyage_api_key.as_ref().map(|_| "<redacted>"),
            )
            .field(
                "anthropic_api_key",
                &self.anthropic_api_key.as_ref().map(|_| "<redacted>"),
            )
            .field("llm_model", &self.llm_model)
            .field("min_similarity", &self.min_similarity)
            .field("webmention_send", &self.webmention_send)
            .field("browser_login", &self.browser_login)
            .field("write_rate_limit", &self.write_rate_limit)
            .field("trust_forwarded_headers", &self.trust_forwarded_headers)
            .field("site_title", &self.site_title)
            .field("site_description", &self.site_description)
            .field("site_author", &self.site_author)
            .field("custom_css_url", &self.custom_css_url)
            .field("metrics_enabled", &self.metrics_enabled)
            .field(
                "metrics_token",
                &self.metrics_token.as_ref().map(|_| "<redacted>"),
            )
            .field("media_backend", &self.media_backend)
            .field("media_dir", &self.media_dir)
            .field("media_max_bytes", &self.media_max_bytes)
            .field("webhooks_enabled", &self.webhooks_enabled)
            .field("webhook_urls", &self.webhook_urls)
            .field(
                "webhook_secret",
                &self.webhook_secret.as_ref().map(|_| "<redacted>"),
            )
            .finish()
    }
}

impl Config {
    pub fn from_env() -> Result<Self> {
        // Load `.env` when present; never override variables already set in the process env.
        let _ = dotenvy::dotenv();

        let database_url =
            std::env::var("DATABASE_URL").map_err(|_| anyhow!("DATABASE_URL is required"))?;
        let host = std::env::var("HOST").unwrap_or_else(|_| "0.0.0.0".to_string());
        let port = match std::env::var("PORT") {
            Ok(raw) if !raw.is_empty() => raw.parse::<u16>().map_err(|_| {
                anyhow!("Invalid PORT \"{raw}\": expected an integer between 0 and 65535.")
            })?,
            _ => 3000,
        };
        let api_key = std::env::var("INKWELL_API_KEY")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        let site_url = std::env::var("INKWELL_SITE_URL")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        let voyage_api_key = trimmed_env("VOYAGE_API_KEY");
        let anthropic_api_key = trimmed_env("ANTHROPIC_API_KEY");
        let llm_model =
            trimmed_env("INKWELL_LLM_MODEL").unwrap_or_else(|| DEFAULT_LLM_MODEL.to_string());
        let min_similarity = trimmed_env("INKWELL_MIN_SIMILARITY")
            .and_then(|value| value.parse::<f32>().ok())
            .unwrap_or(0.0);
        // Webmention send is opt-in: only the explicit string "true" (case-
        // insensitive, trimmed) enables it. Anything else — absent, empty,
        // "false", "1", garbage — leaves it off, the safe default.
        let webmention_send = trimmed_env("INKWELL_WEBMENTION_SEND")
            .is_some_and(|value| value.eq_ignore_ascii_case("true"));
        // Browser login is opt-in: same parse rule as webmention_send.
        let browser_login = trimmed_env("INKWELL_BROWSER_LOGIN")
            .is_some_and(|value| value.eq_ignore_ascii_case("true"));
        // Write rate limit (requests/minute). Only an UNSET (or blank) variable
        // falls back to the default; a present-but-malformed value (e.g. "abc",
        // "-1") fails startup rather than silently defaulting, mirroring PORT.
        // An explicit `0` is valid and disables limiting.
        let write_rate_limit = match trimmed_env("INKWELL_WRITE_RATE_LIMIT") {
            Some(raw) => raw.parse::<u32>().map_err(|_| {
                anyhow!(
                    "Invalid INKWELL_WRITE_RATE_LIMIT \"{raw}\": expected a non-negative integer (0 disables)."
                )
            })?,
            None => DEFAULT_WRITE_RATE_LIMIT,
        };
        // Trust forwarded headers is opt-in: same parse rule as the other flags.
        let trust_forwarded_headers = trimmed_env("INKWELL_TRUST_FORWARDED_HEADERS")
            .is_some_and(|value| value.eq_ignore_ascii_case("true"));
        let site_title =
            trimmed_env("INKWELL_SITE_TITLE").unwrap_or_else(|| DEFAULT_SITE_TITLE.to_string());
        let site_description = trimmed_env("INKWELL_SITE_DESCRIPTION");
        let site_author = trimmed_env("INKWELL_SITE_AUTHOR");
        let custom_css_url = trimmed_env("INKWELL_CUSTOM_CSS_URL");
        // Metrics exposure is opt-in: same strict "true"-only parse rule as the
        // other flags, so a typo leaves the endpoint unregistered rather than
        // silently publishing operational data.
        let metrics_enabled = trimmed_env("INKWELL_METRICS_ENABLED")
            .is_some_and(|value| value.eq_ignore_ascii_case("true"));
        let metrics_token = trimmed_env("INKWELL_METRICS_TOKEN");
        if metrics_enabled && metrics_token.is_none() {
            tracing::warn!(
                "INKWELL_METRICS_ENABLED is on without INKWELL_METRICS_TOKEN: /metrics is unauthenticated. Set a token or keep the port private."
            );
        }
        // A misspelled backend fails startup rather than silently defaulting:
        // picking the wrong one loses (or strands) uploaded images.
        let media_backend = match trimmed_env("INKWELL_MEDIA_BACKEND") {
            Some(raw) => MediaBackend::parse(&raw).ok_or_else(|| {
                anyhow!(
                    "Invalid INKWELL_MEDIA_BACKEND \"{raw}\": expected \"local\" or \"postgres\"."
                )
            })?,
            None => MediaBackend::Local,
        };
        let media_dir =
            trimmed_env("INKWELL_MEDIA_DIR").unwrap_or_else(|| DEFAULT_MEDIA_DIR.to_string());
        let media_max_bytes = match trimmed_env("INKWELL_MEDIA_MAX_BYTES") {
            Some(raw) => {
                let parsed = raw.parse::<usize>().map_err(|_| {
                    anyhow!(
                        "Invalid INKWELL_MEDIA_MAX_BYTES \"{raw}\": expected a positive integer number of bytes."
                    )
                })?;
                if parsed == 0 || parsed > MEDIA_MAX_BYTES_CEILING {
                    return Err(anyhow!(
                        "Invalid INKWELL_MEDIA_MAX_BYTES \"{raw}\": expected 1..={MEDIA_MAX_BYTES_CEILING}."
                    ));
                }
                parsed
            }
            None => DEFAULT_MEDIA_MAX_BYTES,
        };
        // Outbound webhooks are opt-in: same strict "true"-only parse rule as the
        // other flags, so a typo leaves the path inert instead of quietly POSTing
        // publish events off-box.
        let webhooks_enabled = trimmed_env("INKWELL_WEBHOOKS_ENABLED")
            .is_some_and(|value| value.eq_ignore_ascii_case("true"));
        let webhook_urls = trimmed_env("INKWELL_WEBHOOK_URLS")
            .map(|raw| {
                raw.split(',')
                    .map(str::trim)
                    .filter(|url| !url.is_empty())
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let webhook_secret = trimmed_env("INKWELL_WEBHOOK_SECRET");
        // Misconfiguration fails startup rather than silently never delivering
        // (or, worse, delivering unsigned). An operator who explicitly turned the
        // flag on gets told exactly what is missing.
        if webhooks_enabled {
            if webhook_urls.is_empty() {
                return Err(anyhow!(
                    "INKWELL_WEBHOOKS_ENABLED is true but INKWELL_WEBHOOK_URLS is empty: set one or more comma-separated http(s) endpoints."
                ));
            }
            if webhook_urls.len() > MAX_WEBHOOK_ENDPOINTS {
                return Err(anyhow!(
                    "INKWELL_WEBHOOK_URLS has {} endpoints: at most {MAX_WEBHOOK_ENDPOINTS} are allowed.",
                    webhook_urls.len()
                ));
            }
            for url in &webhook_urls {
                if let Some(problem) = crate::webhooks::endpoint_url_problem(url) {
                    return Err(anyhow!("Invalid INKWELL_WEBHOOK_URLS entry: {problem}."));
                }
            }
            match webhook_secret.as_deref() {
                None => {
                    return Err(anyhow!(
                        "INKWELL_WEBHOOKS_ENABLED is true but INKWELL_WEBHOOK_SECRET is unset: deliveries must be signed."
                    ));
                }
                Some(secret) if secret.len() < MIN_WEBHOOK_SECRET_LEN => {
                    return Err(anyhow!(
                        "INKWELL_WEBHOOK_SECRET is too short: use at least {MIN_WEBHOOK_SECRET_LEN} characters."
                    ));
                }
                Some(_) => {}
            }
        }

        Ok(Self {
            database_url,
            host,
            port,
            api_key,
            site_url,
            voyage_api_key,
            anthropic_api_key,
            llm_model,
            min_similarity,
            webmention_send,
            browser_login,
            write_rate_limit,
            trust_forwarded_headers,
            site_title,
            site_description,
            site_author,
            custom_css_url,
            metrics_enabled,
            metrics_token,
            media_backend,
            media_dir,
            media_max_bytes,
            webhooks_enabled,
            webhook_urls,
            webhook_secret,
        })
    }
}

/// Client-side configuration for the `inkwell author` commands.
///
/// Unlike [`Config`], this deliberately does **not** require `DATABASE_URL`:
/// the authoring CLI talks to a remote server over HTTP and never opens a
/// database connection. It reuses the same env var names and `.env` loading so
/// authors configure the client exactly like the server.
#[derive(Clone)]
pub struct AuthorConfig {
    /// Explicit API base URL (`INKWELL_API_URL`), e.g. `https://blog.example.com`.
    pub api_url: Option<String>,
    pub api_key: Option<String>,
    pub host: String,
    pub port: u16,
}

impl std::fmt::Debug for AuthorConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AuthorConfig")
            .field("api_url", &self.api_url)
            .field("api_key", &self.api_key.as_ref().map(|_| "<redacted>"))
            .field("host", &self.host)
            .field("port", &self.port)
            .finish()
    }
}

impl AuthorConfig {
    pub fn from_env() -> Result<Self> {
        // Load `.env` when present; never override variables already set.
        let _ = dotenvy::dotenv();

        let api_url = trimmed_env("INKWELL_API_URL");
        let api_key = trimmed_env("INKWELL_API_KEY");
        let host = std::env::var("HOST").unwrap_or_else(|_| "0.0.0.0".to_string());
        let port = match std::env::var("PORT") {
            Ok(raw) if !raw.is_empty() => raw.parse::<u16>().map_err(|_| {
                anyhow!("Invalid PORT \"{raw}\": expected an integer between 0 and 65535.")
            })?,
            _ => 3000,
        };

        Ok(Self {
            api_url,
            api_key,
            host,
            port,
        })
    }

    /// Resolve the base URL to target, preferring an explicit `override_url`,
    /// then `INKWELL_API_URL`, then a URL derived from `HOST`/`PORT`. Wildcard
    /// bind hosts collapse to a loopback address so local authoring works.
    pub fn resolve_base_url(&self, override_url: Option<&str>) -> String {
        if let Some(url) = override_url.map(str::trim).filter(|u| !u.is_empty()) {
            return url.to_string();
        }
        if let Some(url) = self.api_url.as_deref() {
            return url.to_string();
        }
        let host = match self.host.as_str() {
            "0.0.0.0" | "::" => "127.0.0.1",
            other => other,
        };
        format!("http://{host}:{}", self.port)
    }
}

fn trimmed_env(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A server config with every secret set to a recognisable sentinel, so a
    /// redaction assertion fails loudly if a new field starts leaking. Tests that
    /// care about one behavior clone this and mutate the fields they exercise.
    fn config_with_sentinel_secrets() -> Config {
        Config {
            database_url: "postgres://user:supersecret@localhost/db".to_string(),
            host: "0.0.0.0".to_string(),
            port: 3000,
            api_key: Some("sentinel-key-value".to_string()),
            site_url: None,
            voyage_api_key: Some("sentinel-voyage-value".to_string()),
            anthropic_api_key: Some("sentinel-anthropic-value".to_string()),
            llm_model: DEFAULT_LLM_MODEL.to_string(),
            min_similarity: 0.0,
            webmention_send: false,
            browser_login: false,
            write_rate_limit: DEFAULT_WRITE_RATE_LIMIT,
            trust_forwarded_headers: false,
            site_title: DEFAULT_SITE_TITLE.to_string(),
            site_description: None,
            site_author: None,
            custom_css_url: None,
            metrics_enabled: true,
            metrics_token: Some("sentinel-metrics-value".to_string()),
            media_backend: MediaBackend::Local,
            media_dir: DEFAULT_MEDIA_DIR.to_string(),
            media_max_bytes: DEFAULT_MEDIA_MAX_BYTES,
            webhooks_enabled: true,
            webhook_urls: vec!["https://hooks.example.com/inkwell".to_string()],
            webhook_secret: Some("sentinel-webhook-value".to_string()),
        }
    }

    #[test]
    fn debug_does_not_leak_api_key_or_dsn_password() {
        let config = config_with_sentinel_secrets();
        let rendered = format!("{config:?}");
        assert!(!rendered.contains("sentinel-key-value"));
        assert!(!rendered.contains("sentinel-webhook-value"));
        assert!(!rendered.contains("sentinel-voyage-value"));
        assert!(!rendered.contains("sentinel-anthropic-value"));
        assert!(!rendered.contains("sentinel-metrics-value"));
        assert!(!rendered.contains("supersecret"));
        assert!(rendered.contains("<redacted>"));
        // Endpoint URLs are NOT secret and stay visible for debuggability.
        assert!(rendered.contains("hooks.example.com"));
    }

    #[test]
    fn webhooks_active_requires_flag_urls_and_secret() {
        let configured = config_with_sentinel_secrets();
        assert!(configured.webhooks_active());

        // Flag off ⇒ inert even when fully configured.
        let mut off = configured.clone();
        off.webhooks_enabled = false;
        assert!(!off.webhooks_active());

        // No endpoints ⇒ nothing to deliver to.
        let mut no_urls = configured.clone();
        no_urls.webhook_urls = Vec::new();
        assert!(!no_urls.webhooks_active());

        // No secret ⇒ we would have to send unsigned; refuse instead.
        let mut no_secret = configured.clone();
        no_secret.webhook_secret = None;
        assert!(!no_secret.webhooks_active());
    }

    #[test]
    fn debug_includes_site_metadata_fields() {
        let config = Config {
            database_url: "postgres://localhost/db".to_string(),
            host: "0.0.0.0".to_string(),
            port: 3000,
            api_key: None,
            site_url: None,
            voyage_api_key: None,
            anthropic_api_key: None,
            llm_model: DEFAULT_LLM_MODEL.to_string(),
            min_similarity: 0.0,
            webmention_send: false,
            browser_login: false,
            write_rate_limit: DEFAULT_WRITE_RATE_LIMIT,
            trust_forwarded_headers: false,
            site_title: "My Garden".to_string(),
            site_description: Some("A digital garden.".to_string()),
            site_author: Some("Alice".to_string()),
            custom_css_url: Some("https://example.com/custom.css".to_string()),
            metrics_enabled: false,
            metrics_token: None,
            media_backend: MediaBackend::Postgres,
            media_dir: "/srv/media".to_string(),
            media_max_bytes: DEFAULT_MEDIA_MAX_BYTES,
            webhooks_enabled: false,
            webhook_urls: Vec::new(),
            webhook_secret: None,
        };
        let rendered = format!("{config:?}");
        assert!(rendered.contains("Postgres"));
        assert!(rendered.contains("/srv/media"));
        assert!(rendered.contains("My Garden"));
        assert!(rendered.contains("A digital garden."));
        assert!(rendered.contains("Alice"));
        assert!(rendered.contains("custom.css"));
    }

    #[test]
    fn media_backend_parses_known_aliases_and_rejects_typos() {
        assert_eq!(MediaBackend::parse("local"), Some(MediaBackend::Local));
        assert_eq!(MediaBackend::parse(" FS "), Some(MediaBackend::Local));
        assert_eq!(
            MediaBackend::parse("postgres"),
            Some(MediaBackend::Postgres)
        );
        assert_eq!(MediaBackend::parse("PG"), Some(MediaBackend::Postgres));
        // A typo must not resolve to a backend — startup fails instead.
        assert_eq!(MediaBackend::parse("localhost"), None);
        assert_eq!(MediaBackend::parse("s3"), None);
        assert_eq!(MediaBackend::parse(""), None);
    }

    #[test]
    fn media_backend_round_trips_through_its_string_form() {
        for backend in [MediaBackend::Local, MediaBackend::Postgres] {
            assert_eq!(MediaBackend::parse(backend.as_str()), Some(backend));
        }
    }

    fn author_config(api_url: Option<&str>, host: &str, port: u16) -> AuthorConfig {
        AuthorConfig {
            api_url: api_url.map(str::to_string),
            api_key: Some("k".to_string()),
            host: host.to_string(),
            port,
        }
    }

    #[test]
    fn resolve_base_url_prefers_override_then_env_then_host_port() {
        let cfg = author_config(Some("https://env.example.com"), "0.0.0.0", 3000);
        // Explicit override wins over everything.
        assert_eq!(
            cfg.resolve_base_url(Some("https://flag.example.com")),
            "https://flag.example.com"
        );
        // Falls back to INKWELL_API_URL.
        assert_eq!(cfg.resolve_base_url(None), "https://env.example.com");
        // Blank override is ignored.
        assert_eq!(cfg.resolve_base_url(Some("  ")), "https://env.example.com");
    }

    #[test]
    fn resolve_base_url_derives_loopback_from_wildcard_host() {
        let cfg = author_config(None, "0.0.0.0", 8080);
        assert_eq!(cfg.resolve_base_url(None), "http://127.0.0.1:8080");

        let cfg = author_config(None, "blog.internal", 443);
        assert_eq!(cfg.resolve_base_url(None), "http://blog.internal:443");
    }

    #[test]
    fn author_config_debug_redacts_api_key() {
        let cfg = author_config(None, "0.0.0.0", 3000);
        let rendered = format!("{cfg:?}");
        assert!(!rendered.contains("\"k\""));
        assert!(rendered.contains("<redacted>"));
    }
}
