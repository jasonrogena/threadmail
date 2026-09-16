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
    /// Address (host:port) the HTTP server listens on.
    pub bind_address: String,
}

/// How this deployment presents comments to and from the mailing list.
#[derive(Debug, Deserialize)]
pub struct ListConfig {
    /// The bot account's own address, used as the `From:` sender when
    /// relaying a web-submitted comment.
    pub bot_address: String,
    /// The mailing list's posting address (e.g. a Google Group address).
    pub posting_address: String,
    /// Whether the no-JS web form relays comments into the mailing list.
    /// When `false`, the rendered page omits the comment form and posting
    /// to the comment endpoint directly is rejected. Useful for running
    /// read-only, or for sites that want email-native comments only.
    /// Defaults to enabled.
    #[serde(default = "enabled")]
    pub relay_comments: bool,
    /// Whether the rendered page includes the `mailto:` "comment by email"
    /// link. Independent of `relay_comments` — a site can offer the web
    /// form without advertising the list's raw posting address, or vice
    /// versa. Defaults to enabled.
    #[serde(default = "enabled")]
    pub show_email_link: bool,
}

/// Caps on how many IMAP searches / SMTP submissions this process will let
/// run at once, so a traffic spike can't hammer the mail provider. Separate
/// limits so a burst of comment submissions can't starve people just
/// reading threads, and vice versa. Optional in the config file; the
/// defaults are reasonable for a small-to-medium site.
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

/// Credentials for the bot account's own mailbox, which the list delivers a
/// copy of every message to since the bot is a normal subscribed member.
#[derive(Debug, Deserialize)]
pub struct ImapConfig {
    pub host: String,
    pub port: u16,
    pub username: String,
    /// Optional here: can instead be supplied via `THREADMAIL_IMAP_PASSWORD`,
    /// which takes precedence when set (see `resolve_secret`). Left as an
    /// empty string, not `Option`, so the rest of the config surface (and
    /// every existing caller) doesn't need to change.
    #[serde(default)]
    pub password: String,
}

/// Credentials used to submit a web-relayed comment to the list.
#[derive(Debug, Deserialize)]
pub struct SmtpConfig {
    pub host: String,
    pub port: u16,
    pub username: String,
    /// Optional here: can instead be supplied via `THREADMAIL_SMTP_PASSWORD`,
    /// which takes precedence when set. See `ImapConfig::password`.
    #[serde(default)]
    pub password: String,
}

impl Config {
    pub fn load(path: &str) -> Result<Config, Error> {
        let contents = fs::read_to_string(path)?;
        Ok(toml::from_str(&contents)?)
    }
}

/// Resolves a secret that may come from the config file or an environment
/// variable, with the environment variable taking precedence when set and
/// non-empty. Deliberately pure (the env value is passed in, not read here)
/// so callers can test it without mutating real process environment state;
/// the one real `std::env::var` call lives in `main.rs`, right where the
/// password is actually needed.
pub fn resolve_secret(from_file: &str, from_env: Option<String>) -> Option<String> {
    from_env
        .filter(|v| !v.is_empty())
        .or_else(|| Some(from_file.to_string()).filter(|v| !v.is_empty()))
}
