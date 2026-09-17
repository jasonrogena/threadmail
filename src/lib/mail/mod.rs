use mail_parser::MessageParser;
use regex::Regex;

#[cfg(test)]
mod tests;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("the given bytes could not be parsed as an RFC 5322 message")]
    Malformed,
}

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
    pub fn is_top_level(&self) -> bool {
        self.in_reply_to.is_none() && self.references.is_empty()
    }

    pub fn replies_to(&self, parent: &Message) -> bool {
        self.in_reply_to.as_deref() == Some(parent.message_id.as_str())
            || self.references.iter().any(|r| r == &parent.message_id)
    }

    pub fn parse(raw: &[u8], footer_regex: Option<&Regex>) -> Result<Message, Error> {
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
            .map(|s| strip_footer(s.trim(), footer_regex))
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
}

// mail-parser can produce a Text or a TextList for In-Reply-To.
fn first_text(value: &mail_parser::HeaderValue) -> Option<String> {
    value.as_text().map(|s| s.to_string()).or_else(|| {
        value
            .as_text_list()
            .and_then(|list| list.first())
            .map(|s| s.to_string())
    })
}

fn strip_footer(body: &str, footer_regex: Option<&Regex>) -> String {
    match footer_regex.and_then(|re| re.find(body)) {
        Some(m) => body[..m.start()].trim_end().to_string(),
        None => body.to_string(),
    }
}
