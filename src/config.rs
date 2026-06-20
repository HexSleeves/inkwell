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
}
