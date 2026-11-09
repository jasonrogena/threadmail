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
        from_moderator: false,
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

fn moderation_reply(
    id: &str,
    in_reply_to: &str,
    from_moderator: bool,
    body: &str,
    sent_at: i64,
) -> Message {
    let mut message = msg(id, "Re: my-post", Some(in_reply_to), sent_at);
    message.from_moderator = from_moderator;
    message.body = body.to_string();
    message
}

fn resolve_and_hide(messages: Vec<Message>) -> Thread {
    let mut thread = Thread::resolve(&subject("my-post", "", ""), messages).unwrap();
    thread.hide_moderated("DELETE");
    thread
}

#[test]
fn a_moderation_reply_hides_the_moderated_comment_and_its_replies_but_not_its_siblings() {
    let top = msg("top", "my-post", None, 1);
    let moderated = msg("moderated", "Re: my-post", Some("top"), 2);
    let under_moderated = msg("under-moderated", "Re: my-post", Some("moderated"), 3);
    let sibling = msg("sibling", "Re: my-post", Some("top"), 4);
    let moderation = moderation_reply("moderation", "moderated", true, "DELETE", 5);

    let thread = resolve_and_hide(vec![top, moderated, under_moderated, sibling, moderation]);

    let replies = &thread.top_level_messages[0].replies;
    assert_eq!(replies.len(), 1);
    assert_eq!(replies[0].message.message_id, "sibling");
}

#[test]
fn a_moderation_reply_to_a_top_level_comment_hides_only_that_comment() {
    let moderated = msg("moderated", "my-post", None, 1);
    let other = msg("other", "my-post", None, 2);
    let moderation = moderation_reply("moderation", "moderated", true, "DELETE", 3);

    let thread = resolve_and_hide(vec![moderated, other, moderation]);

    assert_eq!(thread.top_level_messages.len(), 1);
    assert_eq!(thread.top_level_messages[0].message.message_id, "other");
}

#[test]
fn a_keyword_reply_from_a_non_moderator_hides_nothing() {
    let top = msg("top", "my-post", None, 1);
    let moderation = moderation_reply("moderation", "top", false, "DELETE", 2);

    let thread = resolve_and_hide(vec![top, moderation]);

    assert_eq!(thread.top_level_messages.len(), 1);
    assert_eq!(thread.top_level_messages[0].replies.len(), 1);
}

#[test]
fn a_moderator_reply_without_the_keyword_hides_nothing() {
    let top = msg("top", "my-post", None, 1);
    let moderation = moderation_reply(
        "moderation",
        "top",
        true,
        "Thanks, this is a great comment",
        2,
    );

    let thread = resolve_and_hide(vec![top, moderation]);

    assert_eq!(thread.top_level_messages[0].replies.len(), 1);
}

#[test]
fn the_keyword_matches_any_unquoted_line() {
    let top = msg("top", "my-post", None, 1);
    let moderation = moderation_reply(
        "moderation",
        "top",
        true,
        "On Fri, someone wrote:\n> buy things\n\n  DELETE  ",
        2,
    );

    let thread = resolve_and_hide(vec![top, moderation]);

    assert!(thread.top_level_messages.is_empty());
}

#[test]
fn the_keyword_must_match_case_exactly() {
    let top = msg("top", "my-post", None, 1);
    let moderation = moderation_reply("moderation", "top", true, "delete", 2);

    let thread = resolve_and_hide(vec![top, moderation]);

    assert_eq!(thread.top_level_messages.len(), 1);
}

#[test]
fn the_keyword_inside_quoted_text_does_not_count() {
    let top = msg("top", "my-post", None, 1);
    let moderation = moderation_reply("moderation", "top", true, "Agreed.\n> DELETE", 2);

    let thread = resolve_and_hide(vec![top, moderation]);

    assert_eq!(thread.top_level_messages.len(), 1);
}

#[test]
fn a_moderation_reply_carrying_the_full_references_chain_hides_only_the_moderated_comment() {
    let good = msg("good", "my-post", None, 1);
    let moderated = msg("moderated", "Re: my-post", Some("good"), 2);
    let mut moderation = moderation_reply("moderation", "moderated", true, "DELETE", 3);
    moderation.references = vec!["good".to_string(), "moderated".to_string()];

    let thread = resolve_and_hide(vec![good, moderated, moderation]);

    assert_eq!(thread.top_level_messages.len(), 1);
    assert_eq!(thread.top_level_messages[0].message.message_id, "good");
    assert!(thread.top_level_messages[0].replies.is_empty());
}

#[test]
fn a_reply_carrying_the_full_references_chain_nests_under_its_direct_parent() {
    let top = msg("top", "my-post", None, 1);
    let reply = msg("reply", "Re: my-post", Some("top"), 2);
    let mut nested = msg("nested", "Re: my-post", Some("reply"), 3);
    nested.references = vec!["top".to_string(), "reply".to_string()];

    let thread = Thread::resolve(&subject("my-post", "", ""), vec![top, reply, nested]).unwrap();

    let replies = &thread.top_level_messages[0].replies;
    assert_eq!(replies.len(), 1);
    assert_eq!(replies[0].message.message_id, "reply");
    assert_eq!(replies[0].replies[0].message.message_id, "nested");
}

#[test]
fn a_reply_whose_parent_is_missing_nests_under_its_nearest_present_ancestor() {
    let top = msg("top", "my-post", None, 1);
    let mut nested = msg("nested", "Re: my-post", Some("deleted"), 3);
    nested.references = vec!["top".to_string(), "deleted".to_string()];

    let thread = Thread::resolve(&subject("my-post", "", ""), vec![top, nested]).unwrap();

    assert_eq!(
        thread.top_level_messages[0].replies[0].message.message_id,
        "nested"
    );
}

#[test]
fn a_top_level_moderation_reply_is_not_shown_and_hides_nothing() {
    let other = msg("other", "my-post", None, 1);
    let mut moderation = msg("moderation", "my-post", None, 2);
    moderation.from_moderator = true;
    moderation.body = "DELETE".to_string();

    let thread = resolve_and_hide(vec![other, moderation]);

    assert_eq!(thread.top_level_messages.len(), 1);
    assert_eq!(thread.top_level_messages[0].message.message_id, "other");
}

#[test]
fn a_moderation_reply_whose_target_is_missing_is_not_shown_and_spares_the_ancestor() {
    let good = msg("good", "my-post", None, 1);
    let mut moderation = moderation_reply("moderation", "deleted", true, "DELETE", 3);
    moderation.references = vec!["good".to_string(), "deleted".to_string()];

    let thread = resolve_and_hide(vec![good, moderation]);

    assert_eq!(thread.top_level_messages.len(), 1);
    assert_eq!(thread.top_level_messages[0].message.message_id, "good");
    assert!(thread.top_level_messages[0].replies.is_empty());
}
