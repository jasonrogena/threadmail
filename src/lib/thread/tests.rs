use super::*;

fn msg(id: &str, subject: &str, in_reply_to: Option<&str>, sent_at: i64) -> Message {
    Message {
        message_id: id.to_string(),
        in_reply_to: in_reply_to.map(|s| s.to_string()),
        references: in_reply_to.map(|s| vec![s.to_string()]).unwrap_or_default(),
        subject: subject.to_string(),
        display_name: "Someone".to_string(),
        body: "body".to_string(),
        sent_at: Some(sent_at),
    }
}

#[test]
fn builds_a_nested_tree_from_references() {
    let root = msg("root", "my-post", None, 1);
    let reply = msg("reply", "Re: my-post", Some("root"), 2);
    let nested = msg("nested", "Re: my-post", Some("reply"), 3);

    let thread = resolve("my-post", vec![nested, root, reply]).unwrap();

    assert_eq!(thread.root.message.message_id, "root");
    assert_eq!(thread.root.replies.len(), 1);
    assert_eq!(thread.root.replies[0].message.message_id, "reply");
    assert_eq!(
        thread.root.replies[0].replies[0].message.message_id,
        "nested"
    );
}

#[test]
fn picks_the_earliest_top_level_message_as_root_on_duplicates() {
    let early_root = msg("early", "my-post", None, 1);
    let late_duplicate = msg("late", "my-post", None, 2);

    let thread = resolve("my-post", vec![late_duplicate, early_root]).unwrap();

    assert_eq!(thread.root.message.message_id, "early");
}

#[test]
fn attaches_stray_messages_missing_threading_headers_under_the_root() {
    let root = msg("root", "my-post", None, 1);
    let mut stray = msg("stray", "Re: my-post", None, 2);
    stray.in_reply_to = None;
    stray.references = Vec::new();

    let thread = resolve("my-post", vec![root, stray]).unwrap();

    assert_eq!(thread.root.replies.len(), 1);
    assert_eq!(thread.root.replies[0].message.message_id, "stray");
}

#[test]
fn errors_when_no_message_matches_the_slug() {
    let unrelated = msg("x", "some-other-post", None, 1);
    assert!(resolve("my-post", vec![unrelated]).is_err());
}
