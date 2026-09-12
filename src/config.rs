use std::{env, net::SocketAddr};

/// Everything the process needs to know, read once at boot.
///
/// Defaults are chosen so that `cargo run` against a local Postgres works with
/// no environment at all — the only variable you are ever *required* to set is
/// `DATABASE_URL`, and only in production.
#[derive(Debug, Clone)]
pub struct Config {
    pub database_url: String,
    pub bind: SocketAddr,
    pub base_url: String,
    /// Below this many confirmed signups we don't show a counter on the page.
    /// A waiting list is a thing people join, not a scoreboard we perform on.
    pub counter_threshold: i64,
    /// Set to `false` to keep the outbox worker parked (useful in tests and
    /// when a separate process drains it).
    pub drain_outbox: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("{0} is not set and has no default")]
    Missing(&'static str),
    #[error("{key} is not a valid {what}: {value}")]
    Invalid {
        key: &'static str,
        what: &'static str,
        value: String,
    },
}

impl Config {
    pub fn from_env() -> Result<Self, ConfigError> {
        let database_url = match env::var("DATABASE_URL") {
            Ok(url) if !url.trim().is_empty() => url,
            _ => return Err(ConfigError::Missing("DATABASE_URL")),
        };

        let bind: SocketAddr = {
            let raw = env::var("BIND").unwrap_or_else(|_| "0.0.0.0:8080".to_owned());
            raw.parse().map_err(|_| ConfigError::Invalid {
                key: "BIND",
                what: "socket address",
                value: raw,
            })?
        };

        let base_url = env::var("BASE_URL")
            .unwrap_or_else(|_| format!("http://{bind}"))
            .trim_end_matches('/')
            .to_owned();

        let counter_threshold = match env::var("COUNTER_THRESHOLD") {
            Ok(raw) => raw.parse().map_err(|_| ConfigError::Invalid {
                key: "COUNTER_THRESHOLD",
                what: "whole number",
                value: raw,
            })?,
            Err(_) => 25,
        };

        let drain_outbox = !matches!(
            env::var("DRAIN_OUTBOX").as_deref(),
            Ok("0") | Ok("false") | Ok("no")
        );

        Ok(Self {
            database_url,
            bind,
            base_url,
            counter_threshold,
            drain_outbox,
        })
    }
}
