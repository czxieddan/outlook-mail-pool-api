use std::{env, net::SocketAddr, path::PathBuf};

use anyhow::{Context, Result};

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub bind: SocketAddr,
    pub database_url: String,
    pub api_key: String,
    pub lease_ttl_seconds: i64,
    pub refresh_cooldown_seconds: i64,
    pub max_imap_workers: usize,
}

impl AppConfig {
    pub fn from_env() -> Result<Self> {
        let host = env::var("MAIL_POOL_HOST").unwrap_or_else(|_| "0.0.0.0".to_string());
        let port = env::var("MAIL_POOL_PORT").unwrap_or_else(|_| "8098".to_string());
        let bind = format!("{host}:{port}")
            .parse::<SocketAddr>()
            .with_context(|| format!("invalid MAIL_POOL_HOST/MAIL_POOL_PORT: {host}:{port}"))?;
        let api_key = env::var("MAIL_POOL_API_KEY").unwrap_or_else(|_| "change-me".to_string());
        let database_url = env::var("DATABASE_URL").unwrap_or_else(|_| {
            let path = PathBuf::from("data").join("mail_pool.sqlite");
            format!(
                "sqlite://{}?mode=rwc",
                path.to_string_lossy().replace('\\', "/")
            )
        });

        Ok(Self {
            bind,
            database_url,
            api_key,
            lease_ttl_seconds: env_i64("MAIL_POOL_LEASE_TTL_SECONDS", 900),
            refresh_cooldown_seconds: env_i64("MAIL_POOL_REFRESH_COOLDOWN_SECONDS", 8),
            max_imap_workers: env_usize("MAIL_POOL_MAX_IMAP_WORKERS", 16),
        })
    }
}

fn env_i64(key: &str, default: i64) -> i64 {
    env::var(key)
        .ok()
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or(default)
}

fn env_usize(key: &str, default: usize) -> usize {
    env::var(key)
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(default)
}
