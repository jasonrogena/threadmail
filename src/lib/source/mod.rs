// Semantic-level ports, not protocol-level, so a JMAP adapter could replace
// imap_source without changing either trait.
pub type BoxError = Box<dyn std::error::Error + Send + Sync>;

pub trait MailSource: Send + Sync {
    fn search_subject(&self, subject: &str) -> Result<Vec<Vec<u8>>, BoxError>;
}

pub trait MailSink: Send + Sync {
    fn submit(&self, envelope: &lettre::address::Envelope, raw: &[u8]) -> Result<(), BoxError>;
}
