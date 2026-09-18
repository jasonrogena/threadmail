use std::fs;
use std::io;

use regex::Regex;
use serde::Deserialize;

#[cfg(test)]
mod tests;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("could not read the config file")]
    Io(#[from] io::Error),
    #[error("could not parse the config file as TOML")]
    Toml(#[from] toml::de::Error),
    #[error("no value set for {field}: set it in the config file or {env_var}")]
    MissingSecret {
        field: &'static str,
        env_var: &'static str,
    },
}

#[derive(Debug, Deserialize)]
pub struct Config {
    pub mailing_list: MailingListConfig,
    pub web: WebConfig,
    pub imap: ImapConfig,
    pub smtp: SmtpConfig,
    pub storage: StorageConfig,
}

fn enabled() -> bool {
    true
}

#[derive(Debug, Deserialize)]
pub struct MailingListConfig {
    pub bot_address: String,
    pub posting_address: String,
    // Everything from the first match onward is stripped from bodies; empty
    // (or omitted) disables it. Compiled at load time so a bad pattern
    // fails fast instead of surfacing lazily on first use.
    #[serde(default, deserialize_with = "deserialize_body_footer_regex")]
    pub body_footer_regex: Option<Regex>,
    // Prepended/appended to the slug when building a Subject; empty disables.
    #[serde(default)]
    pub subject_prefix: String,
    #[serde(default)]
    pub subject_suffix: String,
}

fn deserialize_body_footer_regex<'de, D>(deserializer: D) -> Result<Option<Regex>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let raw = String::deserialize(deserializer)?;
    if raw.is_empty() {
        Ok(None)
    } else {
        Regex::new(&raw).map(Some).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Deserialize)]
pub struct WebConfig {
    // The address (host:port) the HTTP server listens on.
    pub bind_address: String,
    #[serde(default = "enabled")]
    pub relay_comments: bool,
    #[serde(default = "enabled")]
    pub show_email_link: bool,
    #[serde(default)]
    pub theme: Theme,
    // How often the rendered page reloads itself to check for new comments.
    #[serde(default = "default_refresh_interval_secs")]
    pub refresh_interval_secs: u64,
}

fn default_refresh_interval_secs() -> u64 {
    60
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    #[default]
    Auto,
    Light,
    Dark,
}

impl Theme {
    pub fn as_str(&self) -> &'static str {
        match self {
            Theme::Auto => "auto",
            Theme::Light => "light",
            Theme::Dark => "dark",
        }
    }
}

impl std::fmt::Display for Theme {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Deserialize)]
pub struct StorageConfig {
    // Backs both the incoming comment cache and the outgoing comment queue;
    // must be a writable, persistent location (e.g. a mounted volume).
    pub path: String,
    // How long a cached incoming search result is served before a fresh
    // IMAP search is triggered in the background.
    #[serde(default = "default_incoming_message_ttl_secs")]
    pub incoming_message_ttl_secs: u64,
    // How long an outgoing (submitted) comment stays queued for retry
    // before being dropped undelivered.
    #[serde(default = "default_outgoing_message_ttl_secs")]
    pub outgoing_message_ttl_secs: u64,
    // How often the outgoing comment worker retries whatever's still queued.
    #[serde(default = "default_outgoing_comment_sweep_interval_secs")]
    pub outgoing_comment_sweep_interval_secs: u64,
}

fn default_incoming_message_ttl_secs() -> u64 {
    60
}

fn default_outgoing_message_ttl_secs() -> u64 {
    3 * 60 * 60
}

fn default_outgoing_comment_sweep_interval_secs() -> u64 {
    10
}

#[derive(Debug, Deserialize)]
pub struct ImapConfig {
    pub host: String,
    pub port: u16,
    // Can also come from THREADMAIL_IMAP_USERNAME/_PASSWORD; see username()/password().
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub password: String,
    #[serde(default = "default_max_concurrent_searches")]
    pub max_concurrent_searches: usize,
}

fn default_max_concurrent_searches() -> usize {
    8
}

impl ImapConfig {
    pub fn username(&self) -> Result<String, Error> {
        resolve_secret(&self.username, env("THREADMAIL_IMAP_USERNAME")).ok_or(
            Error::MissingSecret {
                field: "imap.username",
                env_var: "THREADMAIL_IMAP_USERNAME",
            },
        )
    }

    pub fn password(&self) -> Result<String, Error> {
        resolve_secret(&self.password, env("THREADMAIL_IMAP_PASSWORD")).ok_or(
            Error::MissingSecret {
                field: "imap.password",
                env_var: "THREADMAIL_IMAP_PASSWORD",
            },
        )
    }
}

#[derive(Debug, Deserialize)]
pub struct SmtpConfig {
    pub host: String,
    pub port: u16,
    // Can also come from THREADMAIL_SMTP_USERNAME/_PASSWORD; see username()/password().
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub password: String,
    #[serde(default = "default_max_concurrent_submits")]
    pub max_concurrent_submits: usize,
}

fn default_max_concurrent_submits() -> usize {
    4
}

impl SmtpConfig {
    pub fn username(&self) -> Result<String, Error> {
        resolve_secret(&self.username, env("THREADMAIL_SMTP_USERNAME")).ok_or(
            Error::MissingSecret {
                field: "smtp.username",
                env_var: "THREADMAIL_SMTP_USERNAME",
            },
        )
    }

    pub fn password(&self) -> Result<String, Error> {
        resolve_secret(&self.password, env("THREADMAIL_SMTP_PASSWORD")).ok_or(
            Error::MissingSecret {
                field: "smtp.password",
                env_var: "THREADMAIL_SMTP_PASSWORD",
            },
        )
    }
}

impl Config {
    pub fn load(path: &str) -> Result<Config, Error> {
        let contents = fs::read_to_string(path)?;
        Ok(toml::from_str(&contents)?)
    }
}

fn env(var: &str) -> Option<String> {
    std::env::var(var).ok()
}

// Pure so it's directly testable; env() is the one impure caller.
// Env wins over the file when set.
fn resolve_secret(from_file: &str, from_env: Option<String>) -> Option<String> {
    from_env
        .filter(|v| !v.is_empty())
        .or_else(|| Some(from_file.to_string()).filter(|v| !v.is_empty()))
}
