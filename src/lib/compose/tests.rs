use super::*;

fn round_trip(msg: &Message) -> crate::mail::Message {
    crate::mail::Message::parse(&msg.formatted(), None).unwrap()
}

#[test]
fn top_level_comment_uses_the_bare_slug_as_subject() {
    let comment = NewComment {
        name: "Alice",
        body: "Great post!",
        in_reply_to: None,
    };
    let msg = comment
        .compose(
            "my-first-post",
            "bot@ourdomain.com",
            "group@googlegroups.com",
        )
        .unwrap();
    let parsed = round_trip(&msg);

    assert_eq!(parsed.subject, "my-first-post");
    assert_eq!(parsed.display_name, "Alice (via web)");
    assert_eq!(parsed.body, "Great post!");
    assert!(parsed.in_reply_to.is_none());
}

#[test]
fn reply_carries_in_reply_to_and_references() {
    let comment = NewComment {
        name: "Bob",
        body: "I agree!",
        in_reply_to: Some("root@example.com"),
    };
    let msg = comment
        .compose(
            "my-first-post",
            "bot@ourdomain.com",
            "group@googlegroups.com",
        )
        .unwrap();
    let parsed = round_trip(&msg);

    assert_eq!(parsed.subject, "Re: my-first-post");
    assert_eq!(parsed.in_reply_to.as_deref(), Some("root@example.com"));
    assert_eq!(parsed.references, vec!["root@example.com".to_string()]);
}

#[test]
fn never_includes_a_real_commenter_address_because_none_is_collected() {
    let comment = NewComment {
        name: "Carol",
        body: "no address was ever asked for",
        in_reply_to: None,
    };
    let msg = comment
        .compose(
            "my-first-post",
            "bot@ourdomain.com",
            "group@googlegroups.com",
        )
        .unwrap();
    let raw = String::from_utf8(msg.formatted()).unwrap();

    assert!(raw.contains("bot@ourdomain.com"));
}
