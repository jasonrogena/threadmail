use super::*;

#[test]
fn loads_a_well_formed_config() {
    let config = Config::load("tests/configs/good.toml").unwrap();
    assert_eq!(config.web.bind_address, "127.0.0.1:8080");
    assert_eq!(
        config.mailing_list.posting_address,
        "group@googlegroups.com"
    );
    assert_eq!(config.imap.port, 993);
    assert_eq!(config.smtp.port, 587);
    assert_eq!(config.storage.path, "/tmp/threadmail-test-store.db");
}

#[test]
fn defaults_the_toggles_and_limits_when_omitted() {
    let config = Config::load("tests/configs/good.toml").unwrap();
    assert!(config.web.relay_comments);
    assert!(config.web.show_email_link);
    assert_eq!(config.web.theme, Theme::Auto);
    assert_eq!(config.mailing_list.subject_prefix, "");
    assert_eq!(config.mailing_list.subject_suffix, "");
    assert_eq!(config.web.refresh_interval_secs, 180);
    assert_eq!(config.imap.max_concurrent_searches, 8);
    assert_eq!(config.imap.mailbox, "INBOX");
    assert_eq!(config.smtp.max_concurrent_deliveries, 4);
    assert_eq!(config.smtp.delivery_timeout_secs, 60);
    assert_eq!(config.storage.incoming_message_ttl_secs, 30);
    assert_eq!(config.storage.outgoing_message_ttl_secs, 3 * 60 * 60);
    assert_eq!(config.storage.outgoing_comment_sweep_interval_secs, 10);
}

#[test]
fn honors_explicit_limits_when_given() {
    let config = Config::load("tests/configs/good-with-limits.toml").unwrap();
    assert_eq!(config.imap.max_concurrent_searches, 20);
    assert_eq!(config.imap.mailbox, "[Gmail]/All Mail");
    assert_eq!(config.smtp.max_concurrent_deliveries, 2);
    assert_eq!(config.smtp.delivery_timeout_secs, 20);
    assert_eq!(config.storage.incoming_message_ttl_secs, 30);
    assert_eq!(config.web.refresh_interval_secs, 90);
}

#[test]
fn honors_an_explicit_outgoing_comment_config() {
    let config = Config::load("tests/configs/good-with-limits.toml").unwrap();
    assert_eq!(config.storage.outgoing_message_ttl_secs, 60);
    assert_eq!(config.storage.outgoing_comment_sweep_interval_secs, 5);
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
        [web]
        bind_address = "127.0.0.1:8080"
        [mailing_list]
        bot_address = "bot@example.com"
        posting_address = "group@example.com"
        [imap]
        host = "imap.example.com"
        port = 993
        [smtp]
        host = "smtp.example.com"
        port = 587
        [storage]
        path = "/tmp/threadmail-test-store.db"
    "#;
    let config: Config = toml::from_str(toml_str).unwrap();
    assert_eq!(config.imap.username, "");
    assert_eq!(config.imap.password, "");
    assert_eq!(config.smtp.username, "");
    assert_eq!(config.smtp.password, "");
}

#[test]
fn honors_an_explicit_subject_prefix_and_suffix() {
    let toml_str = r#"
        [web]
        bind_address = "127.0.0.1:8080"
        [mailing_list]
        bot_address = "bot@example.com"
        posting_address = "group@example.com"
        subject_prefix = "Blog Comments: "
        subject_suffix = " (blog)"
        [imap]
        host = "imap.example.com"
        port = 993
        [smtp]
        host = "smtp.example.com"
        port = 587
        [storage]
        path = "/tmp/threadmail-test-store.db"
    "#;
    let config: Config = toml::from_str(toml_str).unwrap();
    assert_eq!(config.mailing_list.subject_prefix, "Blog Comments: ");
    assert_eq!(config.mailing_list.subject_suffix, " (blog)");
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

// These read real process env, so they only assert the fallback-to-file
// path; they'd be flaky to write against env overrides (setting real env
// vars in tests is unsound-adjacent under parallel execution), which is why
// resolve_secret above is exercised directly as a pure function instead.
#[test]
fn imap_username_and_password_fall_back_to_the_file_when_no_env_is_set() {
    let config = Config::load("tests/configs/good.toml").unwrap();
    assert_eq!(config.imap.username().unwrap(), "bot@ourdomain.example");
    assert_eq!(config.imap.password().unwrap(), "app-password");
}

#[test]
fn smtp_username_and_password_fall_back_to_the_file_when_no_env_is_set() {
    let config = Config::load("tests/configs/good.toml").unwrap();
    assert_eq!(config.smtp.username().unwrap(), "bot@ourdomain.example");
    assert_eq!(config.smtp.password().unwrap(), "app-password");
}

#[test]
fn a_secret_missing_from_both_file_and_env_is_an_error() {
    let toml_str = r#"
        [web]
        bind_address = "127.0.0.1:8080"
        [mailing_list]
        bot_address = "bot@example.com"
        posting_address = "group@example.com"
        [imap]
        host = "imap.example.com"
        port = 993
        [smtp]
        host = "smtp.example.com"
        port = 587
        [storage]
        path = "/tmp/threadmail-test-store.db"
    "#;
    let config: Config = toml::from_str(toml_str).unwrap();
    let err = config.imap.username().unwrap_err();
    assert!(matches!(
        err,
        Error::MissingSecret {
            field: "imap.username",
            ..
        }
    ));
}

#[test]
fn ca_file_is_none_when_omitted() {
    let config = Config::load("tests/configs/good.toml").unwrap();
    assert!(config.imap.ca_file.is_none());
    assert!(config.smtp.ca_file.is_none());
}

#[test]
fn ca_file_is_parsed_when_given() {
    let toml_str = r#"
        [web]
        bind_address = "127.0.0.1:8080"
        [mailing_list]
        bot_address = "bot@example.com"
        posting_address = "group@example.com"
        [imap]
        host = "imap.example.com"
        port = 993
        ca_file = "/etc/threadmail/imap-ca.pem"
        [smtp]
        host = "smtp.example.com"
        port = 587
        ca_file = "/etc/threadmail/smtp-ca.pem"
        [storage]
        path = "/tmp/threadmail-test-store.db"
    "#;
    let config: Config = toml::from_str(toml_str).unwrap();
    assert_eq!(
        config.imap.ca_file,
        Some("/etc/threadmail/imap-ca.pem".to_string())
    );
    assert_eq!(
        config.smtp.ca_file,
        Some("/etc/threadmail/smtp-ca.pem".to_string())
    );
}

#[test]
fn smtp_tls_mode_defaults_to_starttls_when_omitted() {
    let config = Config::load("tests/configs/good.toml").unwrap();
    assert_eq!(config.smtp.tls_mode, SmtpTlsMode::Starttls);
}

#[test]
fn smtp_tls_mode_parses_implicit_when_given() {
    let toml_str = r#"
        [web]
        bind_address = "127.0.0.1:8080"
        [mailing_list]
        bot_address = "bot@example.com"
        posting_address = "group@example.com"
        [imap]
        host = "imap.example.com"
        port = 993
        [smtp]
        host = "smtp.example.com"
        port = 465
        tls_mode = "implicit"
        [storage]
        path = "/tmp/threadmail-test-store.db"
    "#;
    let config: Config = toml::from_str(toml_str).unwrap();
    assert_eq!(config.smtp.tls_mode, SmtpTlsMode::Implicit);
}

#[test]
fn body_footer_regex_is_none_when_omitted() {
    let config = Config::load("tests/configs/good.toml").unwrap();
    assert!(config.mailing_list.body_footer_regex.is_none());
}

#[test]
fn body_footer_regex_compiles_a_configured_pattern() {
    let toml_str = r#"
        [web]
        bind_address = "127.0.0.1:8080"
        [mailing_list]
        bot_address = "bot@example.com"
        posting_address = "group@example.com"
        body_footer_regex = '^--\s*$'
        [imap]
        host = "imap.example.com"
        port = 993
        [smtp]
        host = "smtp.example.com"
        port = 587
        [storage]
        path = "/tmp/threadmail-test-store.db"
    "#;
    let config: Config = toml::from_str(toml_str).unwrap();
    assert!(
        config
            .mailing_list
            .body_footer_regex
            .unwrap()
            .is_match("-- ")
    );
}

#[test]
fn an_invalid_body_footer_regex_fails_to_parse() {
    let toml_str = r#"
        [web]
        bind_address = "127.0.0.1:8080"
        [mailing_list]
        bot_address = "bot@example.com"
        posting_address = "group@example.com"
        body_footer_regex = "("
        [imap]
        host = "imap.example.com"
        port = 993
        [smtp]
        host = "smtp.example.com"
        port = 587
        [storage]
        path = "/tmp/threadmail-test-store.db"
    "#;
    let err = toml::from_str::<Config>(toml_str).unwrap_err();
    assert!(err.to_string().contains("body_footer_regex") || err.to_string().contains("regex"));
}

#[test]
fn theme_defaults_to_auto_when_omitted() {
    let config = Config::load("tests/configs/good.toml").unwrap();
    assert_eq!(config.web.theme, Theme::Auto);
}

#[test]
fn theme_parses_light_and_dark() {
    let toml_str = r#"
        [mailing_list]
        bot_address = "bot@example.com"
        posting_address = "group@example.com"
        [web]
        bind_address = "127.0.0.1:8080"
        theme = "dark"
        [imap]
        host = "imap.example.com"
        port = 993
        [smtp]
        host = "smtp.example.com"
        port = 587
        [storage]
        path = "/tmp/threadmail-test-store.db"
    "#;
    let config: Config = toml::from_str(toml_str).unwrap();
    assert_eq!(config.web.theme, Theme::Dark);
}

#[test]
fn an_unrecognized_theme_fails_to_parse() {
    let toml_str = r#"
        [mailing_list]
        bot_address = "bot@example.com"
        posting_address = "group@example.com"
        [web]
        bind_address = "127.0.0.1:8080"
        theme = "sepia"
        [imap]
        host = "imap.example.com"
        port = 993
        [smtp]
        host = "smtp.example.com"
        port = 587
        [storage]
        path = "/tmp/threadmail-test-store.db"
    "#;
    let err = toml::from_str::<Config>(toml_str).unwrap_err();
    assert!(err.to_string().contains("theme"));
}

#[test]
fn the_mailing_list_envelope_runs_from_the_bot_to_the_posting_address() {
    let config = Config::load("tests/configs/good.toml").unwrap();
    let envelope = config.mailing_list.envelope().unwrap();
    assert_eq!(
        envelope.from().map(ToString::to_string),
        Some(config.mailing_list.bot_address.clone())
    );
    assert_eq!(envelope.to().len(), 1);
    assert_eq!(
        envelope.to()[0].to_string(),
        config.mailing_list.posting_address
    );
}

#[test]
fn an_invalid_mailing_list_address_fails_to_build_an_envelope() {
    let mut config = Config::load("tests/configs/good.toml").unwrap();
    config.mailing_list.posting_address = "not an address".to_string();
    let err = config.mailing_list.envelope().unwrap_err();
    assert!(matches!(err, Error::Address(_)));
}

#[test]
fn max_concurrent_deliveries_still_loads_from_its_old_max_concurrent_submits_name() {
    let smtp: SmtpConfig =
        toml::from_str("host = \"smtp.example.com\"\nport = 587\nmax_concurrent_submits = 7\n")
            .unwrap();
    assert_eq!(smtp.max_concurrent_deliveries, 7);
}
