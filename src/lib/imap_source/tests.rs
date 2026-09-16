use super::*;

#[test]
fn escapes_quotes_and_backslashes_in_the_search_term() {
    assert_eq!(
        escape_search_term(r#"a "quoted" \slug"#),
        r#"a \"quoted\" \\slug"#
    );
}
