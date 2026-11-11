use std::{env, net::SocketAddr, time::Duration};

use thiserror::Error;
use url::Url;

#[derive(Clone)]
pub struct Config {
    pub bind_addr: SocketAddr,
    pub database_url: String,
    pub base_url: String,
    pub allowed_origins: Vec<String>,
    pub max_content_bytes: usize,
    pub database_max_connections: u32,
    pub cleanup_interval: Duration,
    pub api_token: Option<String>,
    pub rust_log: String,
}

#[derive(Debug, Error)]
#[error("invalid environment variable {key}: {reason}")]
pub struct ConfigError {
    key: &'static str,
    reason: &'static str,
}

impl Config {
    pub fn share_url(&self, id: &str) -> String {
        format!("{}/{id}", self.base_url)
    }

    pub fn from_env() -> Result<Self, ConfigError> {
        Self::from_lookup(|key| match env::var(key) {
            Ok(value) => Ok(Some(value)),
            Err(env::VarError::NotPresent) => Ok(None),
            Err(env::VarError::NotUnicode(_)) => Err(ConfigError {
                key,
                reason: "must be valid Unicode",
            }),
        })
    }

    fn from_lookup(
        lookup: impl Fn(&'static str) -> Result<Option<String>, ConfigError>,
    ) -> Result<Self, ConfigError> {
        let value = |key, default: &str| -> Result<String, ConfigError> {
            Ok(lookup(key)?.unwrap_or_else(|| default.to_owned()))
        };
        let bind_addr = value("BIND_ADDR", "127.0.0.1:3000")?
            .parse()
            .map_err(|_| ConfigError {
                key: "BIND_ADDR",
                reason: "must be an IP address and port",
            })?;
        let database_url = value("DATABASE_URL", "sqlite://data/pastelet.db")?;
        if !database_url.starts_with("sqlite:") {
            return Err(ConfigError {
                key: "DATABASE_URL",
                reason: "must be a SQLite URL",
            });
        }
        let base_url = normalize_origin(&value("BASE_URL", "http://localhost:3000")?, "BASE_URL")?;
        let mut allowed_origins = Vec::new();
        let configured = value("ALLOWED_ORIGINS", "")?;
        if !configured.trim().is_empty() {
            for entry in configured.split(',') {
                let origin = normalize_origin(entry.trim(), "ALLOWED_ORIGINS")?;
                if origin != base_url && !allowed_origins.contains(&origin) {
                    allowed_origins.push(origin);
                }
            }
        }
        let max_content_bytes =
            positive("MAX_CONTENT_BYTES", &value("MAX_CONTENT_BYTES", "1048576")?)?;
        let database_max_connections = positive(
            "DATABASE_MAX_CONNECTIONS",
            &value("DATABASE_MAX_CONNECTIONS", "5")?,
        )?;
        let cleanup_seconds = positive(
            "CLEANUP_INTERVAL_SECONDS",
            &value("CLEANUP_INTERVAL_SECONDS", "60")?,
        )?;
        if std::time::Instant::now()
            .checked_add(Duration::from_secs(cleanup_seconds))
            .is_none()
        {
            return Err(ConfigError {
                key: "CLEANUP_INTERVAL_SECONDS",
                reason: "interval exceeds the monotonic clock range",
            });
        }
        let api_token = lookup("API_TOKEN")?.filter(|token| !token.is_empty());
        if api_token
            .as_ref()
            .is_some_and(|token| !token.bytes().all(|byte| byte.is_ascii_graphic()))
        {
            return Err(ConfigError {
                key: "API_TOKEN",
                reason: "must contain only visible ASCII characters without spaces",
            });
        }
        Ok(Self {
            bind_addr,
            database_url,
            base_url,
            allowed_origins,
            max_content_bytes,
            database_max_connections,
            cleanup_interval: Duration::from_secs(cleanup_seconds),
            api_token,
            rust_log: value("RUST_LOG", "pastelet=info,tower_http=info")?,
        })
    }
}

pub(crate) fn normalize_origin(value: &str, key: &'static str) -> Result<String, ConfigError> {
    let invalid = || ConfigError {
        key,
        reason: "must be an explicit HTTP(S) origin without wildcards, credentials, path, query or fragment",
    };
    let parsed = Url::parse(value).map_err(|_| invalid())?;
    if !matches!(parsed.scheme(), "http" | "https")
        || parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
        || parsed.path() != "/"
        || value.contains('*')
        || value.chars().any(char::is_whitespace)
    {
        return Err(invalid());
    }
    Ok(parsed.origin().ascii_serialization())
}

fn positive<T>(key: &'static str, value: &str) -> Result<T, ConfigError>
where
    T: std::str::FromStr + PartialOrd + From<u8>,
{
    value
        .parse::<T>()
        .ok()
        .filter(|parsed| *parsed > T::from(0))
        .ok_or(ConfigError {
            key,
            reason: "must be a positive integer within range",
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(entries: &[(&str, &str)]) -> Result<Config, ConfigError> {
        Config::from_lookup(|key| {
            Ok(entries
                .iter()
                .find(|(name, _)| *name == key)
                .map(|(_, value)| (*value).to_owned()))
        })
    }

    #[test]
    fn defaults() {
        let config = config(&[]).expect("valid defaults");
        assert_eq!(config.max_content_bytes, 1024 * 1024);
        assert_eq!(config.cleanup_interval, Duration::from_secs(60));
        assert!(config.api_token.is_none());
        assert!(config.allowed_origins.is_empty());
    }

    #[test]
    fn overrides_and_empty_token() {
        let config = config(&[
            ("BASE_URL", "https://paste.example/"),
            ("API_TOKEN", ""),
            ("MAX_CONTENT_BYTES", "1024"),
        ])
        .expect("valid overrides");
        assert_eq!(config.base_url, "https://paste.example");
        assert_eq!(config.max_content_bytes, 1024);
        assert!(config.api_token.is_none());
    }

    #[test]
    fn allowed_origins_are_normalized_deduplicated_and_optional() {
        let settings = config(&[
            ("BASE_URL", "https://paste.example/"),
            ("ALLOWED_ORIGINS", " https://PASTE.example:443/, http://100.64.0.10:8007, http://my-node:8007/, http://my-node:8007 "),
        ]).expect("origins");
        assert_eq!(settings.base_url, "https://paste.example");
        assert_eq!(
            settings.allowed_origins,
            ["http://100.64.0.10:8007", "http://my-node:8007"]
        );
        assert!(
            config(&[("ALLOWED_ORIGINS", "  ")])
                .expect("empty list")
                .allowed_origins
                .is_empty()
        );
    }

    #[test]
    fn invalid_configuration_is_rejected_without_echoing_values() {
        for (key, value) in [
            ("BIND_ADDR", "localhost:3000"),
            ("DATABASE_URL", "postgres://localhost/db"),
            ("BASE_URL", "javascript:alert(1)"),
            ("BASE_URL", "https://user:secret@example.com"),
            ("BASE_URL", "https://example.com/subpath"),
            ("BASE_URL", "https://example.com/?secret=token"),
            ("ALLOWED_ORIGINS", "*"),
            ("ALLOWED_ORIGINS", "https://*.example.com"),
            ("ALLOWED_ORIGINS", "null"),
            ("ALLOWED_ORIGINS", "https://example.com/path"),
            ("ALLOWED_ORIGINS", "https://user:password@example.com"),
            ("ALLOWED_ORIGINS", "http://node:8007,"),
            ("ALLOWED_ORIGINS", "http://node:8007 https://example.com"),
            ("MAX_CONTENT_BYTES", "0"),
            ("MAX_CONTENT_BYTES", "-1"),
            ("DATABASE_MAX_CONNECTIONS", "4294967296"),
            ("CLEANUP_INTERVAL_SECONDS", "0"),
            ("CLEANUP_INTERVAL_SECONDS", "18446744073709551615"),
            ("API_TOKEN", "secret\nvalue"),
        ] {
            let error = config(&[(key, value)]).err().expect("invalid config");
            assert_eq!(error.key, key);
            assert!(!error.to_string().contains(value));
        }
    }
}
