use anyhow::{Result, anyhow};

#[derive(Clone, Debug)]
pub struct Config {
    pub database_url: String,
    pub host: String,
    pub port: u16,
    pub api_key: Option<String>,
    pub site_url: Option<String>,
}

impl Config {
    pub fn from_env() -> Result<Self> {
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
