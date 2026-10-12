use super::*;

#[test]
fn escapes_quotes_and_backslashes_in_the_search_term() {
    assert_eq!(
        escape_search_term(r#"a "quoted" \slug"#),
        r#"a \"quoted\" \\slug"#
    );
}

#[test]
fn root_store_errors_when_the_configured_ca_file_does_not_exist() {
    let source = ImapSource {
        host: "imap.example.com".to_string(),
        port: 993,
        username: String::new(),
        password: String::new(),
        mailbox: "INBOX".to_string(),
        ca_file: Some("/nonexistent/ca.pem".to_string()),
    };
    assert!(matches!(source.root_store(), Err(Error::CaFile(_))));
}

#[test]
fn root_store_falls_back_to_the_public_roots_when_no_ca_file_is_configured() {
    let source = ImapSource {
        host: "imap.example.com".to_string(),
        port: 993,
        username: String::new(),
        password: String::new(),
        mailbox: "INBOX".to_string(),
        ca_file: None,
    };
    assert!(!source.root_store().unwrap().roots.is_empty());
}
