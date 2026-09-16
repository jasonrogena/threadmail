//! The only two I/O boundaries the pure core depends on.
//!
//! Both are defined at the semantic level ("get messages matching this
//! subject", "send this message") rather than the protocol level, so a
//! JMAP adapter could stand in for `imap_source` later without changing
//! either trait or anything upstream of them. Errors are boxed rather than
//! an associated type so both traits stay object-safe and can be stored as
//! `Arc<dyn MailSource>` / `Arc<dyn MailSink>` in shared server state.

pub type BoxError = Box<dyn std::error::Error + Send + Sync>;

/// Fetches raw RFC 5322 messages relevant to one post's thread.
pub trait MailSource: Send + Sync {
    /// Returns the raw bytes of every message whose `Subject` header
    /// matches `subject` exactly or contains it (mirroring how a mail
    /// client's `Re:` prefixing behaves), so the caller can resolve a full
    /// thread out of the result via `crate::thread::resolve`.
    fn search_subject(&self, subject: &str) -> Result<Vec<Vec<u8>>, BoxError>;
}

/// Submits a composed message to the mailing list.
pub trait MailSink: Send + Sync {
    fn submit(&self, message: &lettre::Message) -> Result<(), BoxError>;
}
