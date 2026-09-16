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
    let msg = parse(TOP_LEVEL).unwrap();
    assert_eq!(msg.message_id, "root@example.com");
    assert_eq!(msg.subject, "my-first-post");
    assert_eq!(msg.display_name, "Alice");
    assert_eq!(msg.body, "Great post!");
    assert!(msg.is_top_level());
}

#[test]
fn parses_a_reply_with_threading_headers() {
    let msg = parse(REPLY).unwrap();
    assert_eq!(msg.in_reply_to.as_deref(), Some("root@example.com"));
    assert_eq!(msg.references, vec!["root@example.com".to_string()]);
    assert!(!msg.is_top_level());
}

#[test]
fn never_exposes_a_real_address() {
    let msg = parse(TOP_LEVEL).unwrap();
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
    let msg = parse(raw).unwrap();
    assert_eq!(msg.display_name, "Anonymous");
}

#[test]
fn rejects_malformed_input() {
    assert!(parse(b"not an email at all").is_err() || parse(b"").is_err());
}
