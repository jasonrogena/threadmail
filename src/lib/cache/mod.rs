use std::time::{Duration, SystemTime};

use crate::source::BoxError;

#[cfg(test)]
mod tests;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheEntry {
    pub raw_messages: Vec<Vec<u8>>,
    pub refreshed_at: Option<SystemTime>,
}

impl CacheEntry {
    pub fn empty() -> CacheEntry {
        CacheEntry {
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
pub trait CommentCache: Send + Sync {
    fn get(&self, slug: &str) -> Result<CacheEntry, BoxError>;
    fn store(&self, slug: &str, raw_messages: &[Vec<u8>]) -> Result<(), BoxError>;
    fn mark_refresh_attempted(&self, slug: &str) -> Result<(), BoxError>;
    fn invalidate(&self, slug: &str) -> Result<(), BoxError>;
}
