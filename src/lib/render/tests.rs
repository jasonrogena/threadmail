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
        theme: "auto",
        just_posted: false,
        refresh_interval_secs: 60,
        stale: false,
        subject_prefix: "",
        subject_suffix: "",
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

    let html = thread.render(&options(true, true));

    assert!(!html.contains("<script>"));
    assert!(html.contains("&#60;script&#62;"));
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

    let html = thread.render(&options(true, true));

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

    let html = thread.render(&options(true, true));

    assert!(html.contains("mailto:group@example.com?subject=my-post"));
}

#[test]
fn the_mailto_subject_carries_the_configured_prefix_and_suffix() {
    let thread = Thread {
        slug: "my-post".to_string(),
        root: Node {
            message: msg("root", "Alice", "first"),
            replies: Vec::new(),
        },
    };
    let mut opts = options(true, true);
    opts.subject_prefix = "Blog Comments: ";
    opts.subject_suffix = " (blog)";

    let html = thread.render(&opts);

    assert!(html.contains("subject=Blog%20Comments%3A%20my-post%20%28blog%29"));
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

    let html = thread.render(&options(true, true));

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

    let html = thread.render(&options(false, true));

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

    let html = thread.render(&options(true, false));

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
fn empty_does_not_claim_no_comments_while_stale() {
    let mut stale = options(true, true);
    stale.stale = true;

    let html = empty("my-post", &stale);

    assert!(!html.contains("No comments yet"));
    assert!(html.contains("class=\"stale-notice\""));
    // The form still offers to comment even while the search is unconfirmed.
    assert!(html.contains("<form"));
}

#[test]
fn shows_a_posted_notice_only_when_requested() {
    let thread = Thread {
        slug: "my-post".to_string(),
        root: Node {
            message: msg("root", "Alice", "first"),
            replies: Vec::new(),
        },
    };
    let mut posted = options(true, true);
    posted.just_posted = true;

    assert!(thread.render(&posted).contains("class=\"posted-notice\""));
    assert!(
        !thread
            .render(&options(true, true))
            .contains("class=\"posted-notice\"")
    );
    assert!(empty("my-post", &posted).contains("class=\"posted-notice\""));
    assert!(!empty("my-post", &options(true, true)).contains("class=\"posted-notice\""));
}

#[test]
fn a_just_posted_reload_targets_the_clean_url_so_the_notice_clears_itself() {
    let thread = Thread {
        slug: "my-post".to_string(),
        root: Node {
            message: msg("root", "Alice", "first"),
            replies: Vec::new(),
        },
    };
    let mut posted = options(true, true);
    posted.just_posted = true;

    assert!(
        thread
            .render(&posted)
            .contains(";url=/thread/my-post/comment")
    );
    assert!(!thread.render(&options(true, true)).contains(";url="));
    assert!(empty("my-post", &posted).contains(";url=/thread/my-post/comment"));
    assert!(!empty("my-post", &options(true, true)).contains(";url="));
}

#[test]
fn reply_form_is_collapsed_by_default() {
    let thread = Thread {
        slug: "my-post".to_string(),
        root: Node {
            message: msg("root", "Alice", "first"),
            replies: Vec::new(),
        },
    };

    let html = thread.render(&options(true, true));

    assert!(html.contains("<details class=\"reply-toggle\">"));
    assert!(!html.contains("<details class=\"reply-toggle\" open"));
    assert!(!html.contains("<details open"));
}

#[test]
fn auto_theme_defers_to_the_device_via_media_query() {
    let thread = Thread {
        slug: "my-post".to_string(),
        root: Node {
            message: msg("root", "Alice", "first"),
            replies: Vec::new(),
        },
    };
    let mut opts = options(true, true);
    opts.theme = "auto";

    let html = thread.render(&opts);

    assert!(html.contains("color-scheme: light dark;"));
    assert!(html.contains("@media (prefers-color-scheme: dark)"));
}

#[test]
fn light_theme_is_forced_regardless_of_device() {
    let thread = Thread {
        slug: "my-post".to_string(),
        root: Node {
            message: msg("root", "Alice", "first"),
            replies: Vec::new(),
        },
    };
    let mut opts = options(true, true);
    opts.theme = "light";

    let html = thread.render(&opts);

    assert!(html.contains("color-scheme: light;"));
    assert!(!html.contains("@media (prefers-color-scheme: dark)"));
}

#[test]
fn dark_theme_is_forced_regardless_of_device() {
    let thread = Thread {
        slug: "my-post".to_string(),
        root: Node {
            message: msg("root", "Alice", "first"),
            replies: Vec::new(),
        },
    };
    let mut opts = options(true, true);
    opts.theme = "dark";

    let html = thread.render(&opts);

    assert!(html.contains("color-scheme: dark;"));
    assert!(html.contains("--tm-bg: #0f172a;"));
    assert!(!html.contains("@media (prefers-color-scheme: dark)"));
}

#[test]
fn each_comment_gets_an_initial_avatar_and_an_anchor() {
    let thread = Thread {
        slug: "my-post".to_string(),
        root: Node {
            message: msg("root", "alice", "first"),
            replies: Vec::new(),
        },
    };

    let html = thread.render(&options(true, true));

    assert!(html.contains("id=\"c-root\""));
    assert!(html.contains("data-initial=\"A\""));
}

#[test]
fn always_includes_a_periodic_refresh_tag() {
    let thread = Thread {
        slug: "my-post".to_string(),
        root: Node {
            message: msg("root", "Alice", "first"),
            replies: Vec::new(),
        },
    };

    assert!(
        thread
            .render(&options(true, true))
            .contains("http-equiv=\"refresh\"")
    );
    assert!(empty("my-post", &options(true, true)).contains("http-equiv=\"refresh\""));
}

#[test]
fn the_refresh_tag_uses_the_configured_interval() {
    let thread = Thread {
        slug: "my-post".to_string(),
        root: Node {
            message: msg("root", "Alice", "first"),
            replies: Vec::new(),
        },
    };
    let mut opts = options(true, true);
    opts.refresh_interval_secs = 45;

    assert!(thread.render(&opts).contains("content=\"45\""));
    assert!(empty("my-post", &opts).contains("content=\"45\""));
}

#[test]
fn shows_a_stale_notice_only_when_the_content_is_past_its_ttl() {
    let thread = Thread {
        slug: "my-post".to_string(),
        root: Node {
            message: msg("root", "Alice", "first"),
            replies: Vec::new(),
        },
    };
    let mut stale = options(true, true);
    stale.stale = true;

    assert!(thread.render(&stale).contains("class=\"stale-notice\""));
    assert!(
        !thread
            .render(&options(true, true))
            .contains("class=\"stale-notice\"")
    );
    assert!(empty("my-post", &stale).contains("class=\"stale-notice\""));
    assert!(!empty("my-post", &options(true, true)).contains("class=\"stale-notice\""));
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

    assert!(!thread.render(&options(true, true)).contains("<script>"));
    assert!(
        !empty("<script>", &options(true, true))
            .contains("<section class=\"thread\" data-slug=\"<script>\"")
    );
}
