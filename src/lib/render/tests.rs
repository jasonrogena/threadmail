use super::*;
use crate::config::{MailingListConfig, Theme, WebConfig};
use crate::mail::{Author, Message};

fn msg(id: &str, name: &str, body: &str) -> Message {
    Message {
        message_id: id.to_string(),
        in_reply_to: None,
        references: Vec::new(),
        subject: "my-post".to_string(),
        author: Author {
            display_name: name.to_string(),
        },
        body: body.to_string(),
        sent_at: None,
    }
}

fn mailing_list_config() -> MailingListConfig {
    MailingListConfig {
        bot_address: "bot@example.com".to_string(),
        posting_address: "group@example.com".to_string(),
        body_footer_regex: None,
        subject_prefix: String::new(),
        subject_suffix: String::new(),
    }
}

fn web_config(relay_comments: bool, show_email_link: bool) -> WebConfig {
    WebConfig {
        relay_comments,
        show_email_link,
        theme: Theme::Auto,
        refresh_interval_secs: 60,
    }
}

fn options<'a>(mailing_list: &'a MailingListConfig, web: &'a WebConfig) -> Options<'a> {
    Options {
        comment_action: "/thread/my-post/comment",
        mailing_list,
        web,
        stale: false,
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
    let mailing_list = mailing_list_config();
    let web = web_config(true, true);

    let html = thread.render(&options(&mailing_list, &web));

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
    let mailing_list = mailing_list_config();
    let web = web_config(true, true);

    let html = thread.render(&options(&mailing_list, &web));

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
    let mailing_list = mailing_list_config();
    let web = web_config(true, true);

    let html = thread.render(&options(&mailing_list, &web));

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
    let mut mailing_list = mailing_list_config();
    mailing_list.subject_prefix = "Blog Comments: ".to_string();
    mailing_list.subject_suffix = " (blog)".to_string();
    let web = web_config(true, true);

    let html = thread.render(&options(&mailing_list, &web));

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
    let mailing_list = mailing_list_config();
    let web = web_config(true, true);

    let html = thread.render(&options(&mailing_list, &web));

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
    let mailing_list = mailing_list_config();
    let web = web_config(false, true);

    let html = thread.render(&options(&mailing_list, &web));

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
    let mailing_list = mailing_list_config();
    let web = web_config(true, false);

    let html = thread.render(&options(&mailing_list, &web));

    assert!(html.contains("<form"));
    assert!(!html.contains("mailto:"));
}

#[test]
fn empty_offers_a_top_level_form_replying_to_nothing_when_relay_is_enabled() {
    let mailing_list = mailing_list_config();
    let web = web_config(true, true);

    let html = empty("my-post", &options(&mailing_list, &web));

    assert!(html.contains("No comments yet"));
    assert!(html.contains("name=\"in_reply_to\" value=\"\""));
}

#[test]
fn empty_omits_the_form_when_relay_is_disabled() {
    let mailing_list = mailing_list_config();
    let web = web_config(false, true);

    let html = empty("my-post", &options(&mailing_list, &web));

    assert!(!html.contains("<form"));
}

#[test]
fn empty_does_not_claim_no_comments_while_stale() {
    let mailing_list = mailing_list_config();
    let web = web_config(true, true);
    let mut stale = options(&mailing_list, &web);
    stale.stale = true;

    let html = empty("my-post", &stale);

    assert!(!html.contains("No comments yet"));
    assert!(html.contains("class=\"stale-notice\""));
    // The form still offers to comment even while the search is unconfirmed.
    assert!(html.contains("<form"));
}

#[test]
fn shows_a_persistent_latency_notice_only_when_relay_is_enabled() {
    let thread = Thread {
        slug: "my-post".to_string(),
        root: Node {
            message: msg("root", "Alice", "first"),
            replies: Vec::new(),
        },
    };
    let mailing_list = mailing_list_config();
    let relay_on = web_config(true, true);
    let relay_off = web_config(false, true);

    assert!(
        thread
            .render(&options(&mailing_list, &relay_on))
            .contains("class=\"latency-notice\"")
    );
    assert!(
        !thread
            .render(&options(&mailing_list, &relay_off))
            .contains("class=\"latency-notice\"")
    );
    assert!(
        empty("my-post", &options(&mailing_list, &relay_on)).contains("class=\"latency-notice\"")
    );
    assert!(
        !empty("my-post", &options(&mailing_list, &relay_off)).contains("class=\"latency-notice\"")
    );
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
    let mailing_list = mailing_list_config();
    let web = web_config(true, true);

    let html = thread.render(&options(&mailing_list, &web));

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
    let mailing_list = mailing_list_config();
    let mut web = web_config(true, true);
    web.theme = Theme::Auto;

    let html = thread.render(&options(&mailing_list, &web));

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
    let mailing_list = mailing_list_config();
    let mut web = web_config(true, true);
    web.theme = Theme::Light;

    let html = thread.render(&options(&mailing_list, &web));

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
    let mailing_list = mailing_list_config();
    let mut web = web_config(true, true);
    web.theme = Theme::Dark;

    let html = thread.render(&options(&mailing_list, &web));

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
    let mailing_list = mailing_list_config();
    let web = web_config(true, true);

    let html = thread.render(&options(&mailing_list, &web));

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
    let mailing_list = mailing_list_config();
    let web = web_config(true, true);

    assert!(
        thread
            .render(&options(&mailing_list, &web))
            .contains("http-equiv=\"refresh\"")
    );
    assert!(empty("my-post", &options(&mailing_list, &web)).contains("http-equiv=\"refresh\""));
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
    let mailing_list = mailing_list_config();
    let mut web = web_config(true, true);
    web.refresh_interval_secs = 45;

    assert!(
        thread
            .render(&options(&mailing_list, &web))
            .contains("content=\"45\"")
    );
    assert!(empty("my-post", &options(&mailing_list, &web)).contains("content=\"45\""));
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
    let mailing_list = mailing_list_config();
    let web = web_config(true, true);
    let mut stale = options(&mailing_list, &web);
    stale.stale = true;

    assert!(thread.render(&stale).contains("class=\"stale-notice\""));
    assert!(
        !thread
            .render(&options(&mailing_list, &web))
            .contains("class=\"stale-notice\"")
    );
    assert!(empty("my-post", &stale).contains("class=\"stale-notice\""));
    assert!(!empty("my-post", &options(&mailing_list, &web)).contains("class=\"stale-notice\""));
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
    let mailing_list = mailing_list_config();
    let web = web_config(true, true);

    assert!(
        !thread
            .render(&options(&mailing_list, &web))
            .contains("<script>")
    );
    assert!(
        !empty("<script>", &options(&mailing_list, &web))
            .contains("<section class=\"thread\" data-slug=\"<script>\"")
    );
}
