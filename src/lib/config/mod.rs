use std::fs;
use std::io;

use serde::Deserialize;

#[cfg(test)]
mod tests;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("could not read the config file")]
    Io(#[from] io::Error),
    #[error("could not parse the config file as TOML")]
    Toml(#[from] toml::de::Error),
}

#[derive(Debug, Deserialize)]
pub struct Config {
    pub server: ServerConfig,
    pub list: ListConfig,
    pub imap: ImapConfig,
    pub smtp: SmtpConfig,
    pub storage: StorageConfig,
    #[serde(default)]
    pub outbox: OutboxConfig,
    #[serde(default)]
    pub limits: Limits,
}

fn enabled() -> bool {
    true
}

#[derive(Debug, Deserialize)]
pub struct ServerConfig {
    pub bind_address: String,
}

#[derive(Debug, Deserialize)]
pub struct ListConfig {
    pub bot_address: String,
    pub posting_address: String,
    #[serde(default = "enabled")]
    pub relay_comments: bool,
    #[serde(default = "enabled")]
    pub show_email_link: bool,
    // Everything from the first match onward is stripped from bodies; empty disables it.
    #[serde(default)]
    pub body_footer_regex: String,
    // Prepended/appended to the slug when building a Subject; empty disables.
    #[serde(default)]
    pub subject_prefix: String,
    #[serde(default)]
    pub subject_suffix: String,
    // "auto" (follow the visitor's device), "light", or "dark".
    #[serde(default = "auto_theme")]
    pub theme: String,
}

fn auto_theme() -> String {
    "auto".to_string()
}

#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct Limits {
    pub max_concurrent_searches: usize,
    pub max_concurrent_submits: usize,
    // How long a cached search result is served before a fresh IMAP search
    // is triggered in the background.
    pub cache_ttl_secs: u64,
    // How often the rendered page reloads itself to check for new comments.
    pub refresh_interval_secs: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_concurrent_searches: 8,
            max_concurrent_submits: 4,
            cache_ttl_secs: 300,
            refresh_interval_secs: 30,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct StorageConfig {
    // Backs both the comment cache and the outgoing-comment outbox; must be
    // a writable, persistent location (e.g. a mounted volume).
    pub path: String,
}

#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct OutboxConfig {
    pub ttl_secs: u64,
    pub sweep_interval_secs: u64,
}

impl Default for OutboxConfig {
    fn default() -> Self {
        Self {
            ttl_secs: 3 * 60 * 60,
            sweep_interval_secs: 10,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct ImapConfig {
    pub host: String,
    pub port: u16,
    // Can also come from THREADMAIL_IMAP_USERNAME/_PASSWORD; see resolve_secret.
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub password: String,
}

#[derive(Debug, Deserialize)]
pub struct SmtpConfig {
    pub host: String,
    pub port: u16,
    // Can also come from THREADMAIL_SMTP_USERNAME/_PASSWORD; see resolve_secret.
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub password: String,
}

impl Config {
    pub fn load(path: &str) -> Result<Config, Error> {
        let contents = fs::read_to_string(path)?;
        Ok(toml::from_str(&contents)?)
    }
}

// Pure so it's testable without mutating real env state; the real
// std::env::var call lives in main.rs. Env wins over the file when set.
pub fn resolve_secret(from_file: &str, from_env: Option<String>) -> Option<String> {
    from_env
        .filter(|v| !v.is_empty())
        .or_else(|| Some(from_file.to_string()).filter(|v| !v.is_empty()))
}
