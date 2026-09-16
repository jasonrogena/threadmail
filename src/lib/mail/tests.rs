use super::*;

const TOP_LEVEL: &[u8] = b"From: \"Alice\" <alice@example.com>\r\n\
Subject: my-first-post\r\n\
Message-ID: <root@example.com>\r\n\
Date: Wed, 16 Sep 2026 09:00:00 +0000\r\n\
Content-Type: text/plain\r\n\
\r\n\
Great post!\r\n";

const REPLY: &[u8] = b"From: \"Bob\" <bob@example.com>\r\n\
Subject: Re: my-first-post\r\n\
Message-ID: <reply@example.com>\r\n\
In-Reply-To: <root@example.com>\r\n\
References: <root@example.com>\r\n\
Date: Wed, 16 Sep 2026 10:00:00 +0000\r\n\
Content-Type: text/plain\r\n\
\r\n\
I agree!\r\n";

#[test]
fn parses_a_top_level_message() {
    let msg = parse(TOP_LEVEL, None).unwrap();
    assert_eq!(msg.message_id, "root@example.com");
    assert_eq!(msg.subject, "my-first-post");
    assert_eq!(msg.display_name, "Alice");
    assert_eq!(msg.body, "Great post!");
    assert!(msg.is_top_level());
}

#[test]
fn parses_a_reply_with_threading_headers() {
    let msg = parse(REPLY, None).unwrap();
    assert_eq!(msg.in_reply_to.as_deref(), Some("root@example.com"));
    assert_eq!(msg.references, vec!["root@example.com".to_string()]);
    assert!(!msg.is_top_level());
}

#[test]
fn never_exposes_a_real_address() {
    let msg = parse(TOP_LEVEL, None).unwrap();
    assert!(!msg.display_name.contains('@'));
    assert!(!msg.body.contains("alice@example.com"));
}

#[test]
fn falls_back_to_anonymous_when_no_display_name_is_set() {
    let raw = b"From: bare@example.com\r\n\
Subject: my-first-post\r\n\
Message-ID: <bare@example.com>\r\n\
Content-Type: text/plain\r\n\
\r\n\
Hi.\r\n";
    let msg = parse(raw, None).unwrap();
    assert_eq!(msg.display_name, "Anonymous");
}

#[test]
fn rejects_malformed_input() {
    assert!(parse(b"not an email at all", None).is_err() || parse(b"", None).is_err());
}

#[test]
fn strips_a_footer_matching_the_configured_regex() {
    let raw = b"From: \"Alice\" <alice@example.com>\r\n\
Subject: my-first-post\r\n\
Message-ID: <root@example.com>\r\n\
Content-Type: text/plain\r\n\
\r\n\
Great post!\r\n\
\r\n\
-- \r\n\
You received this message because you are subscribed to the Google Groups \"blog-comments\" group.\r\n\
To unsubscribe from this group, send an email to blog-comments+unsubscribe@googlegroups.com.\r\n";
    let footer = Regex::new(r"(?s)--\s*\nYou received this message.*").unwrap();
    let msg = parse(raw, Some(&footer)).unwrap();
    assert_eq!(msg.body, "Great post!");
}

#[test]
fn leaves_the_body_alone_when_no_footer_regex_is_configured() {
    let raw = b"From: \"Alice\" <alice@example.com>\r\n\
Subject: my-first-post\r\n\
Message-ID: <root@example.com>\r\n\
Content-Type: text/plain\r\n\
\r\n\
Great post!\r\n\
\r\n\
-- \r\n\
Sent from my phone\r\n";
    let msg = parse(raw, None).unwrap();
    assert!(msg.body.contains("Sent from my phone"));
}

#[test]
fn leaves_the_body_alone_when_the_footer_regex_does_not_match() {
    let raw = b"From: \"Alice\" <alice@example.com>\r\n\
Subject: my-first-post\r\n\
Message-ID: <root@example.com>\r\n\
Content-Type: text/plain\r\n\
\r\n\
Great post!\r\n";
    let footer = Regex::new(r"(?s)--\s*\nYou received this message.*").unwrap();
    let msg = parse(raw, Some(&footer)).unwrap();
    assert_eq!(msg.body, "Great post!");
}
