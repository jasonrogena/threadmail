use super::*;

// `search_subject` itself needs a live IMAP server and is exercised in the
// deployment's own smoke testing, not here; this only covers the one bit of
// pure logic in this adapter.

#[test]
fn escapes_quotes_and_backslashes_in_the_search_term() {
    assert_eq!(
        escape_search_term(r#"a "quoted" \slug"#),
        r#"a \"quoted\" \\slug"#
    );
}
