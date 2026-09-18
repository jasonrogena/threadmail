use crate::mail::Author;

use super::*;

fn msg(id: &str, subject: &str, in_reply_to: Option<&str>, sent_at: i64) -> Message {
    Message {
        message_id: id.to_string(),
        in_reply_to: in_reply_to.map(|s| s.to_string()),
        references: in_reply_to.map(|s| vec![s.to_string()]).unwrap_or_default(),
        subject: subject.to_string(),
        author: Author {
            display_name: "Someone".to_string(),
        },
        body: "body".to_string(),
        sent_at: Some(sent_at),
    }
}

fn subject<'a>(slug: &'a str, prefix: &'a str, suffix: &'a str) -> Subject<'a> {
    Subject {
        slug,
        prefix,
        suffix,
    }
}

#[test]
fn builds_a_nested_tree_from_references() {
    let top = msg("top", "my-post", None, 1);
    let reply = msg("reply", "Re: my-post", Some("top"), 2);
    let nested = msg("nested", "Re: my-post", Some("reply"), 3);

    let thread = Thread::resolve(&subject("my-post", "", ""), vec![nested, top, reply]).unwrap();

    assert_eq!(thread.top_level_messages.len(), 1);
    assert_eq!(thread.top_level_messages[0].message.message_id, "top");
    assert_eq!(thread.top_level_messages[0].replies.len(), 1);
    assert_eq!(
        thread.top_level_messages[0].replies[0].message.message_id,
        "reply"
    );
    assert_eq!(
        thread.top_level_messages[0].replies[0].replies[0]
            .message
            .message_id,
        "nested"
    );
}

#[test]
fn top_level_messages_are_ordered_by_time() {
    let earlier = msg("earlier", "my-post", None, 1);
    let later = msg("later", "my-post", None, 2);

    let thread = Thread::resolve(&subject("my-post", "", ""), vec![later, earlier]).unwrap();

    assert_eq!(thread.top_level_messages.len(), 2);
    assert_eq!(thread.top_level_messages[0].message.message_id, "earlier");
    assert_eq!(thread.top_level_messages[1].message.message_id, "later");
}

#[test]
fn a_non_reply_message_matching_the_subject_is_also_a_top_level_message() {
    let first = msg("first", "my-post", None, 1);
    let mut second = msg("second", "Re: my-post", None, 2);
    second.in_reply_to = None;
    second.references = Vec::new();

    let thread = Thread::resolve(&subject("my-post", "", ""), vec![first, second]).unwrap();

    assert_eq!(thread.top_level_messages.len(), 2);
    assert_eq!(thread.top_level_messages[0].replies.len(), 0);
    assert_eq!(thread.top_level_messages[1].message.message_id, "second");
    assert_eq!(thread.top_level_messages[1].replies.len(), 0);
}

#[test]
fn errors_when_no_message_matches_the_slug() {
    let unrelated = msg("x", "some-other-post", None, 1);
    assert!(Thread::resolve(&subject("my-post", "", ""), vec![unrelated]).is_err());
}

#[test]
fn a_configured_prefix_is_required_on_the_exact_subject() {
    let top = msg("top", "Blog Comments: my-post", None, 1);

    let thread = Thread::resolve(&subject("my-post", "Blog Comments: ", ""), vec![top]).unwrap();

    assert_eq!(thread.top_level_messages[0].message.message_id, "top");
}

#[test]
fn a_message_missing_the_configured_prefix_is_not_matched() {
    let top = msg("top", "my-post", None, 1);

    assert!(Thread::resolve(&subject("my-post", "Blog Comments: ", ""), vec![top]).is_err());
}

#[test]
fn a_configured_suffix_is_required_on_the_exact_subject() {
    let top = msg("top", "my-post (blog)", None, 1);

    let thread = Thread::resolve(&subject("my-post", "", " (blog)"), vec![top]).unwrap();

    assert_eq!(thread.top_level_messages[0].message.message_id, "top");
}

#[test]
fn a_message_missing_the_configured_suffix_is_not_matched() {
    let top = msg("top", "my-post", None, 1);

    assert!(Thread::resolve(&subject("my-post", "", " (blog)"), vec![top]).is_err());
}

#[test]
fn a_non_reply_message_still_matches_on_the_bare_slug_regardless_of_prefix_and_suffix() {
    let top = msg("top", "Blog Comments: my-post (blog)", None, 1);
    let mut other = msg("other", "Re: Blog Comments: my-post (blog)", None, 2);
    other.in_reply_to = None;
    other.references = Vec::new();

    let thread = Thread::resolve(
        &subject("my-post", "Blog Comments: ", " (blog)"),
        vec![top, other],
    )
    .unwrap();

    assert_eq!(thread.top_level_messages.len(), 2);
    assert_eq!(thread.top_level_messages[1].message.message_id, "other");
}
