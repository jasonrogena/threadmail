use std::time::{Duration, SystemTime};

use crate::source::BoxError;

#[cfg(test)]
mod tests;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IncomingComment {
    pub raw_messages: Vec<Vec<u8>>,
    pub refreshed_at: Option<SystemTime>,
}

impl IncomingComment {
    pub fn empty() -> IncomingComment {
        IncomingComment {
            raw_messages: Vec::new(),
            refreshed_at: None,
        }
    }

    pub fn is_stale(&self, ttl: Duration) -> bool {
        match self.refreshed_at {
            None => true,
            Some(at) => at.elapsed().unwrap_or(Duration::MAX) >= ttl,
        }
    }
}

// A local, ephemeral record of the last search result per slug, kept only so
// a page can render instantly while a fresh IMAP search runs in the
// background; never authoritative, so any backing store can implement this.
pub trait IncomingCommentStore: Send + Sync {
    fn get(&self, slug: &str) -> Result<IncomingComment, BoxError>;
    fn store(&self, slug: &str, raw_messages: &[Vec<u8>]) -> Result<(), BoxError>;
    fn mark_refresh_attempted(&self, slug: &str) -> Result<(), BoxError>;
    fn invalidate(&self, slug: &str) -> Result<(), BoxError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutgoingComment {
    pub id: i64,
    pub slug: String,
    pub from_address: String,
    pub to_address: String,
    pub raw: Vec<u8>,
    pub created_at: SystemTime,
}

impl OutgoingComment {
    pub fn is_expired(&self, ttl: Duration) -> bool {
        self.created_at.elapsed().unwrap_or(Duration::MAX) >= ttl
    }
}

// A durable queue of comments accepted from the web form but not yet
// relayed by SMTP, so a submission can be acknowledged immediately and
// delivered in the background, surviving a restart in between.
pub trait OutgoingCommentStore: Send + Sync {
    fn enqueue(
        &self,
        slug: &str,
        from_address: &str,
        to_address: &str,
        raw: &[u8],
    ) -> Result<OutgoingComment, BoxError>;
    fn pending(&self) -> Result<Vec<OutgoingComment>, BoxError>;
    fn remove(&self, id: i64) -> Result<(), BoxError>;
}
