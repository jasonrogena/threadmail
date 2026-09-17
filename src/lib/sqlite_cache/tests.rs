use super::*;

#[test]
fn a_fresh_slug_has_no_cached_messages_and_is_stale() {
    let cache = SqliteCache::open_in_memory().unwrap();

    let entry = cache.get("my-post").unwrap();

    assert!(entry.raw_messages.is_empty());
    assert!(entry.refreshed_at.is_none());
}

#[test]
fn stored_messages_come_back_and_count_as_fresh() {
    let cache = SqliteCache::open_in_memory().unwrap();

    cache
        .store("my-post", &[b"one".to_vec(), b"two".to_vec()])
        .unwrap();
    let entry = cache.get("my-post").unwrap();

    assert_eq!(entry.raw_messages, vec![b"one".to_vec(), b"two".to_vec()]);
    assert!(entry.refreshed_at.is_some());
}

#[test]
fn storing_again_replaces_rather_than_appends() {
    let cache = SqliteCache::open_in_memory().unwrap();

    cache.store("my-post", &[b"one".to_vec()]).unwrap();
    cache.store("my-post", &[b"two".to_vec()]).unwrap();
    let entry = cache.get("my-post").unwrap();

    assert_eq!(entry.raw_messages, vec![b"two".to_vec()]);
}

#[test]
fn a_different_slug_is_unaffected_by_another_slugs_cache() {
    let cache = SqliteCache::open_in_memory().unwrap();

    cache.store("my-post", &[b"one".to_vec()]).unwrap();
    let entry = cache.get("other-post").unwrap();

    assert!(entry.raw_messages.is_empty());
}

#[test]
fn mark_refresh_attempted_freshens_without_touching_messages() {
    let cache = SqliteCache::open_in_memory().unwrap();

    cache.mark_refresh_attempted("my-post").unwrap();
    let entry = cache.get("my-post").unwrap();

    assert!(entry.raw_messages.is_empty());
    assert!(entry.refreshed_at.is_some());
}

#[test]
fn invalidate_clears_the_refreshed_at_without_dropping_messages() {
    let cache = SqliteCache::open_in_memory().unwrap();

    cache.store("my-post", &[b"one".to_vec()]).unwrap();
    cache.invalidate("my-post").unwrap();
    let entry = cache.get("my-post").unwrap();

    assert_eq!(entry.raw_messages, vec![b"one".to_vec()]);
    assert!(entry.refreshed_at.is_none());
}
