use super::*;

fn round_trip(msg: &Message) -> crate::mail::Message {
    crate::mail::Message::parse(&msg.formatted(), None).unwrap()
}

fn subject<'a>(slug: &'a str, prefix: &'a str, suffix: &'a str) -> Subject<'a> {
    Subject {
        slug,
        prefix,
        suffix,
    }
}

#[test]
fn top_level_comment_uses_the_bare_slug_as_subject() {
    let author = Author {
        display_name: "Alice".to_string(),
    };
    let comment = NewComment {
        author: &author,
        body: "Great post!",
        in_reply_to: None,
    };
    let msg = comment
        .compose(
            &subject("my-first-post", "", ""),
            "bot@ourdomain.com",
            "group@googlegroups.com",
        )
        .unwrap();
    let parsed = round_trip(&msg);

    assert_eq!(parsed.subject, "my-first-post");
    assert_eq!(parsed.author.display_name, "Alice (via web)");
    assert_eq!(parsed.body, "Great post!");
    assert!(parsed.in_reply_to.is_none());
}

#[test]
fn reply_carries_in_reply_to_and_references() {
    let author = Author {
        display_name: "Bob".to_string(),
    };
    let comment = NewComment {
        author: &author,
        body: "I agree!",
        in_reply_to: Some("root@example.com"),
    };
    let msg = comment
        .compose(
            &subject("my-first-post", "", ""),
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
    let author = Author {
        display_name: "Carol".to_string(),
    };
    let comment = NewComment {
        author: &author,
        body: "no address was ever asked for",
        in_reply_to: None,
    };
    let msg = comment
        .compose(
            &subject("my-first-post", "", ""),
            "bot@ourdomain.com",
            "group@googlegroups.com",
        )
        .unwrap();
    let raw = String::from_utf8(msg.formatted()).unwrap();

    assert!(raw.contains("bot@ourdomain.com"));
}

#[test]
fn a_configured_prefix_goes_before_the_slug_in_a_top_level_subject() {
    let author = Author {
        display_name: "Alice".to_string(),
    };
    let comment = NewComment {
        author: &author,
        body: "Great post!",
        in_reply_to: None,
    };
    let msg = comment
        .compose(
            &subject("my-first-post", "Blog Comments: ", ""),
            "bot@ourdomain.com",
            "group@googlegroups.com",
        )
        .unwrap();
    let parsed = round_trip(&msg);

    assert_eq!(parsed.subject, "Blog Comments: my-first-post");
}

#[test]
fn a_configured_prefix_stays_before_the_slug_in_a_reply_subject() {
    let author = Author {
        display_name: "Bob".to_string(),
    };
    let comment = NewComment {
        author: &author,
        body: "I agree!",
        in_reply_to: Some("root@example.com"),
    };
    let msg = comment
        .compose(
            &subject("my-first-post", "Blog Comments: ", ""),
            "bot@ourdomain.com",
            "group@googlegroups.com",
        )
        .unwrap();
    let parsed = round_trip(&msg);

    assert_eq!(parsed.subject, "Re: Blog Comments: my-first-post");
}

#[test]
fn a_configured_suffix_goes_after_the_slug_in_a_top_level_subject() {
    let author = Author {
        display_name: "Alice".to_string(),
    };
    let comment = NewComment {
        author: &author,
        body: "Great post!",
        in_reply_to: None,
    };
    let msg = comment
        .compose(
            &subject("my-first-post", "", " (blog)"),
            "bot@ourdomain.com",
            "group@googlegroups.com",
        )
        .unwrap();
    let parsed = round_trip(&msg);

    assert_eq!(parsed.subject, "my-first-post (blog)");
}

#[test]
fn a_configured_suffix_goes_after_the_slug_in_a_reply_subject() {
    let author = Author {
        display_name: "Bob".to_string(),
    };
    let comment = NewComment {
        author: &author,
        body: "I agree!",
        in_reply_to: Some("root@example.com"),
    };
    let msg = comment
        .compose(
            &subject("my-first-post", "", " (blog)"),
            "bot@ourdomain.com",
            "group@googlegroups.com",
        )
        .unwrap();
    let parsed = round_trip(&msg);

    assert_eq!(parsed.subject, "Re: my-first-post (blog)");
}
