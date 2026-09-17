use std::time::Duration;

use super::*;

#[test]
fn an_empty_entry_is_always_stale() {
    assert!(CacheEntry::empty().is_stale(Duration::from_secs(60)));
}

#[test]
fn a_just_refreshed_entry_is_fresh_within_the_ttl() {
    let entry = CacheEntry {
        raw_messages: Vec::new(),
        refreshed_at: Some(SystemTime::now()),
    };

    assert!(!entry.is_stale(Duration::from_secs(60)));
}

#[test]
fn an_old_entry_is_stale_once_the_ttl_elapses() {
    let entry = CacheEntry {
        raw_messages: Vec::new(),
        refreshed_at: Some(SystemTime::now() - Duration::from_secs(120)),
    };

    assert!(entry.is_stale(Duration::from_secs(60)));
}
