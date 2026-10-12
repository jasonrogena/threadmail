use std::sync::Arc;

// Semantic-level ports, not protocol-level, so a JMAP adapter could replace
// imap_source or smtp_sink without changing either trait.
pub type BoxError = Box<dyn std::error::Error + Send + Sync>;

pub trait MailSource: Send + Sync {
    fn search_subject(&self, subject: &str) -> Result<Vec<Vec<u8>>, BoxError>;
}

// Called with a thread's slug once a message submitted for it has actually been delivered.
pub type OnDelivered = Arc<dyn Fn(&str) + Send + Sync>;

pub trait MailSink: Send + Sync {
    // Called once at startup, inside the async runtime.
    fn boot(&self, on_delivered: OnDelivered);
    // Ok means accepted for delivery, which may still happen later; on_delivered reports when it does.
    fn submit(
        &self,
        slug: &str,
        envelope: &lettre::address::Envelope,
        raw: &[u8],
    ) -> Result<(), BoxError>;
    fn health(&self) -> Result<(), BoxError>;
}
