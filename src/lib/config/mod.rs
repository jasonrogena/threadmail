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
}

#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct Limits {
    pub max_concurrent_searches: usize,
    pub max_concurrent_submits: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_concurrent_searches: 8,
            max_concurrent_submits: 4,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct ImapConfig {
    pub host: String,
    pub port: u16,
    pub username: String,
    // Can also come from THREADMAIL_IMAP_PASSWORD; see resolve_secret.
    #[serde(default)]
    pub password: String,
}

#[derive(Debug, Deserialize)]
pub struct SmtpConfig {
    pub host: String,
    pub port: u16,
    pub username: String,
    // Can also come from THREADMAIL_SMTP_PASSWORD; see resolve_secret.
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
