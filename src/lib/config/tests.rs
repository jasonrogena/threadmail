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
    assert_eq!(config.list.theme, "auto");
    assert_eq!(config.limits.max_concurrent_searches, 8);
    assert_eq!(config.limits.max_concurrent_submits, 4);
    assert_eq!(config.limits.cache_ttl_secs, 300);
    assert_eq!(config.limits.refresh_interval_secs, 30);
}

#[test]
fn honors_explicit_limits_when_given() {
    let config = Config::load("tests/configs/good-with-limits.toml").unwrap();
    assert_eq!(config.limits.max_concurrent_searches, 20);
    assert_eq!(config.limits.max_concurrent_submits, 2);
    assert_eq!(config.limits.cache_ttl_secs, 30);
    assert_eq!(config.limits.refresh_interval_secs, 90);
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

#[test]
fn username_and_password_default_to_empty_string_when_omitted() {
    let toml_str = r#"
        [server]
        bind_address = "127.0.0.1:8080"
        [list]
        bot_address = "bot@example.com"
        posting_address = "group@example.com"
        [imap]
        host = "imap.example.com"
        port = 993
        [smtp]
        host = "smtp.example.com"
        port = 587
    "#;
    let config: Config = toml::from_str(toml_str).unwrap();
    assert_eq!(config.imap.username, "");
    assert_eq!(config.imap.password, "");
    assert_eq!(config.smtp.username, "");
    assert_eq!(config.smtp.password, "");
}

#[test]
fn resolve_secret_prefers_env_when_set_and_non_empty() {
    assert_eq!(
        resolve_secret("file-value", Some("env-value".to_string())),
        Some("env-value".to_string())
    );
}

#[test]
fn resolve_secret_falls_back_to_file_when_env_is_unset() {
    assert_eq!(
        resolve_secret("file-value", None),
        Some("file-value".to_string())
    );
}

#[test]
fn resolve_secret_falls_back_to_file_when_env_is_empty() {
    assert_eq!(
        resolve_secret("file-value", Some(String::new())),
        Some("file-value".to_string())
    );
}

#[test]
fn resolve_secret_is_none_when_neither_is_set() {
    assert_eq!(resolve_secret("", None), None);
    assert_eq!(resolve_secret("", Some(String::new())), None);
}
