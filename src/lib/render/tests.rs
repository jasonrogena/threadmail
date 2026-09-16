use super::*;
use crate::mail::Message;

fn msg(id: &str, name: &str, body: &str) -> Message {
    Message {
        message_id: id.to_string(),
        in_reply_to: None,
        references: Vec::new(),
        subject: "my-post".to_string(),
        display_name: name.to_string(),
        body: body.to_string(),
        sent_at: None,
    }
}

fn options(allow_relay: bool, show_email_link: bool) -> Options<'static> {
    Options {
        comment_action: "/thread/my-post/comment",
        mailto_address: "group@example.com",
        allow_relay,
        show_email_link,
    }
}

#[test]
fn escapes_attacker_controlled_content() {
    let thread = Thread {
        slug: "my-post".to_string(),
        root: Node {
            message: msg("root", "<script>alert(1)</script>", "hello \"world\""),
            replies: Vec::new(),
        },
    };

    let html = render(&thread, &options(true, true));

    assert!(!html.contains("<script>"));
    assert!(html.contains("&lt;script&gt;"));
    assert!(html.contains("&quot;world&quot;"));
}

#[test]
fn nests_replies_and_carries_message_id_for_threaded_replies() {
    let thread = Thread {
        slug: "my-post".to_string(),
        root: Node {
            message: msg("root", "Alice", "first"),
            replies: vec![Node {
                message: msg("reply", "Bob", "second"),
                replies: Vec::new(),
            }],
        },
    };

    let html = render(&thread, &options(true, true));

    assert!(html.contains("<ol class=\"replies\">"));
    assert!(html.contains("value=\"root\""));
    assert!(html.contains("value=\"reply\""));
}

#[test]
fn includes_a_mailto_hint_with_the_slug_as_subject() {
    let thread = Thread {
        slug: "my-post".to_string(),
        root: Node {
            message: msg("root", "Alice", "first"),
            replies: Vec::new(),
        },
    };

    let html = render(&thread, &options(true, true));

    assert!(html.contains("mailto:group@example.com?subject=my-post"));
}

#[test]
fn never_renders_a_real_email_address_for_a_commenter() {
    let thread = Thread {
        slug: "my-post".to_string(),
        root: Node {
            message: msg("root", "Alice", "reach me at alice@example.com"),
            replies: Vec::new(),
        },
    };

    let html = render(&thread, &options(true, true));

    assert!(html.contains("Alice"));
}

#[test]
fn omits_comment_forms_when_relay_is_disabled() {
    let thread = Thread {
        slug: "my-post".to_string(),
        root: Node {
            message: msg("root", "Alice", "first"),
            replies: Vec::new(),
        },
    };

    let html = render(&thread, &options(false, true));

    assert!(!html.contains("<form"));
    assert!(html.contains("mailto:"));
}

#[test]
fn omits_the_mailto_hint_when_show_email_link_is_disabled() {
    let thread = Thread {
        slug: "my-post".to_string(),
        root: Node {
            message: msg("root", "Alice", "first"),
            replies: Vec::new(),
        },
    };

    let html = render(&thread, &options(true, false));

    assert!(html.contains("<form"));
    assert!(!html.contains("mailto:"));
}

#[test]
fn empty_offers_a_top_level_form_replying_to_nothing_when_relay_is_enabled() {
    let html = empty("my-post", &options(true, true));

    assert!(html.contains("No comments yet"));
    assert!(html.contains("name=\"in_reply_to\" value=\"\""));
}

#[test]
fn empty_omits_the_form_when_relay_is_disabled() {
    let html = empty("my-post", &options(false, true));

    assert!(!html.contains("<form"));
}

#[test]
fn escapes_the_slug_in_both_render_and_empty() {
    let thread = Thread {
        slug: "<script>".to_string(),
        root: Node {
            message: msg("root", "Alice", "first"),
            replies: Vec::new(),
        },
    };

    assert!(!render(&thread, &options(true, true)).contains("<script>"));
    assert!(
        !empty("<script>", &options(true, true))
            .contains("<section class=\"thread\" data-slug=\"<script>\"")
    );
}
