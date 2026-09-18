use std::time::Duration;

use super::*;

#[test]
fn an_empty_entry_is_always_stale() {
    assert!(IncomingComment::empty().is_stale(Duration::from_secs(60)));
}

#[test]
fn a_just_refreshed_entry_is_fresh_within_the_ttl() {
    let entry = IncomingComment {
        raw_messages: Vec::new(),
        refreshed_at: Some(SystemTime::now()),
    };

    assert!(!entry.is_stale(Duration::from_secs(60)));
}

#[test]
fn an_old_entry_is_stale_once_the_ttl_elapses() {
    let entry = IncomingComment {
        raw_messages: Vec::new(),
        refreshed_at: Some(SystemTime::now() - Duration::from_secs(120)),
    };

    assert!(entry.is_stale(Duration::from_secs(60)));
}

fn pending(created_at: SystemTime) -> OutgoingComment {
    OutgoingComment {
        id: 1,
        slug: "my-post".to_string(),
        from_address: "bot@example.com".to_string(),
        to_address: "group@example.com".to_string(),
        raw: b"raw".to_vec(),
        created_at,
    }
}

#[test]
fn a_freshly_queued_message_is_not_expired() {
    assert!(!pending(SystemTime::now()).is_expired(Duration::from_secs(60)));
}

#[test]
fn a_message_older_than_the_ttl_is_expired() {
    let created_at = SystemTime::now() - Duration::from_secs(120);
    assert!(pending(created_at).is_expired(Duration::from_secs(60)));
}
