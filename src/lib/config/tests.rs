use super::*;

#[test]
fn loads_a_well_formed_config() {
    let config = Config::load("tests/configs/good.toml").unwrap();
    assert_eq!(config.server.bind_address, "127.0.0.1:8080");
    assert_eq!(config.list.posting_address, "group@googlegroups.com");
    assert_eq!(config.imap.port, 993);
    assert_eq!(config.smtp.port, 587);
}

#[test]
fn defaults_the_toggles_and_limits_when_omitted() {
    let config = Config::load("tests/configs/good.toml").unwrap();
    assert!(config.list.relay_comments);
    assert!(config.list.show_email_link);
    assert_eq!(config.limits.max_concurrent_searches, 8);
    assert_eq!(config.limits.max_concurrent_submits, 4);
}

#[test]
fn honors_explicit_limits_when_given() {
    let config = Config::load("tests/configs/good-with-limits.toml").unwrap();
    assert_eq!(config.limits.max_concurrent_searches, 20);
    assert_eq!(config.limits.max_concurrent_submits, 2);
}

#[test]
fn errors_on_a_config_missing_required_fields() {
    let err = Config::load("tests/configs/bad-missing-fields.toml").unwrap_err();
    assert!(matches!(err, Error::Toml(_)));
}

#[test]
fn errors_when_the_file_does_not_exist() {
    let err = Config::load("tests/configs/does-not-exist.toml").unwrap_err();
    assert!(matches!(err, Error::Io(_)));
}
