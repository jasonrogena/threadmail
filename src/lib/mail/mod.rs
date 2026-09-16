//! Pure parsing of raw RFC 5322 messages into the plain data this crate needs.
//!
//! Nothing in this module touches the network or the filesystem: it is a
//! straight function from bytes to a `Message`, which is what keeps it
//! trivially unit-testable with literal fixtures.

use mail_parser::MessageParser;

#[cfg(test)]
mod tests;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("the given bytes could not be parsed as an RFC 5322 message")]
    Malformed,
}

/// The fields of an email this crate actually needs. Never carries the
/// sender's address: only the header/body content that is safe to render.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub message_id: String,
    pub in_reply_to: Option<String>,
    pub references: Vec<String>,
    pub subject: String,
    pub display_name: String,
    pub body: String,
    pub sent_at: Option<i64>,
}

impl Message {
    /// A message with no `In-Reply-To`/`References` is a candidate thread
    /// root; whether it is one for a particular slug is up to the caller.
    pub fn is_top_level(&self) -> bool {
        self.in_reply_to.is_none() && self.references.is_empty()
    }
}

/// `In-Reply-To` normally holds a single message-id but is technically a
/// text-or-list header; this takes whichever variant mail-parser produced.
fn first_text(value: &mail_parser::HeaderValue) -> Option<String> {
    value.as_text().map(|s| s.to_string()).or_else(|| {
        value
            .as_text_list()
            .and_then(|list| list.first())
            .map(|s| s.to_string())
    })
}

pub fn parse(raw: &[u8]) -> Result<Message, Error> {
    let parsed = MessageParser::default()
        .parse(raw)
        .ok_or(Error::Malformed)?;

    let message_id = parsed.message_id().ok_or(Error::Malformed)?.to_string();

    let in_reply_to = first_text(parsed.in_reply_to());

    let references = parsed
        .references()
        .as_text_list()
        .map(|list| list.iter().map(|s| s.to_string()).collect())
        .unwrap_or_default();

    let subject = parsed.subject().unwrap_or_default().trim().to_string();

    let display_name = parsed
        .from()
        .and_then(|addrs| addrs.first())
        .and_then(|addr| addr.name())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "Anonymous".to_string());

    let body = parsed
        .body_text(0)
        .map(|s| s.trim().to_string())
        .unwrap_or_default();

    let sent_at = parsed.date().map(|d| d.to_timestamp());

    Ok(Message {
        message_id,
        in_reply_to,
        references,
        subject,
        display_name,
        body,
        sent_at,
    })
}
