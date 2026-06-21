use anyhow::{Result, anyhow};

#[derive(Clone)]
pub struct Config {
    pub database_url: String,
    pub host: String,
    pub port: u16,
    pub api_key: Option<String>,
    pub site_url: Option<String>,
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

        Ok(Self {
            database_url,
            host,
            port,
            api_key,
            site_url,
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

    #[test]
    fn debug_does_not_leak_api_key_or_dsn_password() {
        let config = Config {
            database_url: "postgres://user:supersecret@localhost/db".to_string(),
            host: "0.0.0.0".to_string(),
            port: 3000,
            api_key: Some("sentinel-key-value".to_string()),
            site_url: None,
        };
        let rendered = format!("{config:?}");
        assert!(!rendered.contains("sentinel-key-value"));
        assert!(!rendered.contains("supersecret"));
        assert!(rendered.contains("<redacted>"));
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
