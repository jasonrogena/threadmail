use super::*;

#[test]
fn a_fresh_slug_has_no_cached_messages_and_is_stale() {
    let db = SqliteStore::open(":memory:").unwrap();

    let entry = db.get("my-post").unwrap();

    assert!(entry.raw_messages.is_empty());
    assert!(entry.refreshed_at.is_none());
}

#[test]
fn stored_messages_come_back_and_count_as_fresh() {
    let db = SqliteStore::open(":memory:").unwrap();

    db.store("my-post", &[b"one".to_vec(), b"two".to_vec()])
        .unwrap();
    let entry = db.get("my-post").unwrap();

    assert_eq!(entry.raw_messages, vec![b"one".to_vec(), b"two".to_vec()]);
    assert!(entry.refreshed_at.is_some());
}

#[test]
fn storing_again_replaces_rather_than_appends() {
    let db = SqliteStore::open(":memory:").unwrap();

    db.store("my-post", &[b"one".to_vec()]).unwrap();
    db.store("my-post", &[b"two".to_vec()]).unwrap();
    let entry = db.get("my-post").unwrap();

    assert_eq!(entry.raw_messages, vec![b"two".to_vec()]);
}

#[test]
fn a_different_slug_is_unaffected_by_another_slugs_cache() {
    let db = SqliteStore::open(":memory:").unwrap();

    db.store("my-post", &[b"one".to_vec()]).unwrap();
    let entry = db.get("other-post").unwrap();

    assert!(entry.raw_messages.is_empty());
}

#[test]
fn mark_refresh_attempted_freshens_without_touching_messages() {
    let db = SqliteStore::open(":memory:").unwrap();

    db.mark_refresh_attempted("my-post").unwrap();
    let entry = db.get("my-post").unwrap();

    assert!(entry.raw_messages.is_empty());
    assert!(entry.refreshed_at.is_some());
}

#[test]
fn invalidate_clears_the_refreshed_at_without_dropping_messages() {
    let db = SqliteStore::open(":memory:").unwrap();

    db.store("my-post", &[b"one".to_vec()]).unwrap();
    db.invalidate("my-post").unwrap();
    let entry = db.get("my-post").unwrap();

    assert_eq!(entry.raw_messages, vec![b"one".to_vec()]);
    assert!(entry.refreshed_at.is_none());
}

#[test]
fn enqueuing_returns_a_pending_message_that_shows_up_in_the_queue() {
    let db = SqliteStore::open(":memory:").unwrap();

    let queued = db
        .enqueue("my-post", "bot@example.com", "group@example.com", b"raw")
        .unwrap();

    assert_eq!(db.pending().unwrap(), vec![queued]);
}

#[test]
fn pending_is_empty_when_nothing_has_been_queued() {
    let db = SqliteStore::open(":memory:").unwrap();
    assert!(db.pending().unwrap().is_empty());
}

#[test]
fn removing_a_message_takes_it_out_of_the_queue() {
    let db = SqliteStore::open(":memory:").unwrap();
    let queued = db
        .enqueue("my-post", "bot@example.com", "group@example.com", b"raw")
        .unwrap();

    db.remove(queued.id).unwrap();

    assert!(db.pending().unwrap().is_empty());
}

#[test]
fn removing_an_unknown_id_is_not_an_error() {
    let db = SqliteStore::open(":memory:").unwrap();
    assert!(db.remove(999).is_ok());
}

#[test]
fn multiple_pending_messages_are_all_returned() {
    let db = SqliteStore::open(":memory:").unwrap();
    db.enqueue("post-a", "bot@example.com", "group@example.com", b"one")
        .unwrap();
    db.enqueue("post-b", "bot@example.com", "group@example.com", b"two")
        .unwrap();

    assert_eq!(db.pending().unwrap().len(), 2);
}

#[test]
fn the_cache_and_outgoing_comments_are_independent_within_the_same_file() {
    let db = SqliteStore::open(":memory:").unwrap();

    db.store("my-post", &[b"cached".to_vec()]).unwrap();
    db.enqueue("my-post", "bot@example.com", "group@example.com", b"raw")
        .unwrap();

    assert_eq!(
        db.get("my-post").unwrap().raw_messages,
        vec![b"cached".to_vec()]
    );
    assert_eq!(db.pending().unwrap().len(), 1);
}
