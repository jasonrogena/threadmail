use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use lettre::address::Envelope;
use tower::ServiceExt;

use crate::AppState;
use crate::config::{
    Config, ImapConfig, MailingListConfig, SmtpConfig, SmtpTlsMode, StorageConfig, Theme, WebConfig,
};
use crate::source::{MailSink, MailSource};
use crate::sqlite_store::SqliteStore;

use super::*;

const ROOT: &[u8] = b"From: \"Alice\" <alice@example.com>\r\n\
Subject: my-post\r\n\
Message-ID: <root@example.com>\r\n\
Date: Wed, 16 Sep 2026 09:00:00 +0000\r\n\
Content-Type: text/plain\r\n\
\r\n\
Great post!\r\n";

struct FixtureSource {
    raw_messages: Vec<Vec<u8>>,
}

impl MailSource for FixtureSource {
    fn search_subject(&self, _subject: &str) -> Result<Vec<Vec<u8>>, crate::source::BoxError> {
        Ok(self.raw_messages.clone())
    }
}

// Delivers synchronously on submit; queueing and retry are smtp_sink's concern, tested there.
#[derive(Default)]
struct FixtureSink {
    sent: Mutex<Vec<crate::mail::Message>>,
    on_delivered: OnceLock<crate::source::OnDelivered>,
}

impl MailSink for FixtureSink {
    fn boot(&self, on_delivered: crate::source::OnDelivered) {
        let _ = self.on_delivered.set(on_delivered);
    }

    fn submit(
        &self,
        slug: &str,
        _envelope: &Envelope,
        raw: &[u8],
    ) -> Result<(), crate::source::BoxError> {
        self.sent
            .lock()
            .unwrap()
            .push(crate::mail::Message::parse(raw, None).unwrap());
        if let Some(on_delivered) = self.on_delivered.get() {
            on_delivered(slug);
        }
        Ok(())
    }

    fn health(&self) -> Result<(), crate::source::BoxError> {
        Ok(())
    }
}

// Always unhealthy, to exercise the /healthz failure path deterministically.
struct UnhealthySink;

impl MailSink for UnhealthySink {
    fn boot(&self, _on_delivered: crate::source::OnDelivered) {}

    fn submit(
        &self,
        _slug: &str,
        _envelope: &Envelope,
        _raw: &[u8],
    ) -> Result<(), crate::source::BoxError> {
        Err("the mail sink is unreachable".into())
    }

    fn health(&self) -> Result<(), crate::source::BoxError> {
        Err("the mail sink is unreachable".into())
    }
}

// Long enough that nothing in these tests ever hits it by accident; tests
// that care about expiry pass an explicit short TTL to `AppState::new`.
const NO_REFRESH_NEEDED: u64 = 3600;

fn state(source: FixtureSource, sink: Arc<dyn MailSink>) -> AppState {
    state_with(source, sink, true, true)
}

fn state_with(
    source: FixtureSource,
    sink: Arc<dyn MailSink>,
    relay_comments: bool,
    show_email_link: bool,
) -> AppState {
    state_with_ttl(
        source,
        sink,
        relay_comments,
        show_email_link,
        NO_REFRESH_NEEDED,
    )
}

#[allow(clippy::too_many_arguments)]
fn state_with_ttl(
    source: FixtureSource,
    sink: Arc<dyn MailSink>,
    relay_comments: bool,
    show_email_link: bool,
    incoming_message_ttl_secs: u64,
) -> AppState {
    state_with_subject(
        source,
        sink,
        relay_comments,
        show_email_link,
        incoming_message_ttl_secs,
        "",
        "",
    )
}

// A fully-populated Config for tests: AppState only reads a handful of its
// fields, but Config itself always needs every section, so this fills the
// rest (server/imap/smtp/storage) with unused placeholders. Callers mutate
// the fields they actually care about.
fn test_config() -> Config {
    Config {
        mailing_list: MailingListConfig {
            bot_address: "bot@ourdomain.example".to_string(),
            posting_address: "group@googlegroups.com".to_string(),
            body_footer_regex: None,
            subject_prefix: String::new(),
            subject_suffix: String::new(),
        },
        web: WebConfig {
            bind_address: "127.0.0.1:0".to_string(),
            relay_comments: true,
            show_email_link: true,
            theme: Theme::Auto,
            refresh_interval_secs: 60,
        },
        imap: ImapConfig {
            host: "imap.example.com".to_string(),
            port: 993,
            username: String::new(),
            password: String::new(),
            max_concurrent_searches: 8,
            mailbox: "INBOX".to_string(),
            ca_file: None,
        },
        smtp: SmtpConfig {
            host: "smtp.example.com".to_string(),
            port: 587,
            username: String::new(),
            password: String::new(),
            max_concurrent_deliveries: 4,
            delivery_timeout_secs: 60,
            ca_file: None,
            tls_mode: SmtpTlsMode::Starttls,
        },
        storage: StorageConfig {
            path: ":memory:".to_string(),
            incoming_message_ttl_secs: NO_REFRESH_NEEDED,
            outgoing_message_ttl_secs: 3600,
            outgoing_comment_sweep_interval_secs: 10,
        },
    }
}

#[allow(clippy::too_many_arguments)]
fn state_with_subject(
    source: FixtureSource,
    sink: Arc<dyn MailSink>,
    relay_comments: bool,
    show_email_link: bool,
    incoming_message_ttl_secs: u64,
    subject_prefix: &str,
    subject_suffix: &str,
) -> AppState {
    let store = Arc::new(SqliteStore::open(":memory:").unwrap());
    let mut config = test_config();
    config.web.relay_comments = relay_comments;
    config.web.show_email_link = show_email_link;
    config.mailing_list.subject_prefix = subject_prefix.to_string();
    config.mailing_list.subject_suffix = subject_suffix.to_string();
    config.storage.incoming_message_ttl_secs = incoming_message_ttl_secs;
    let state = AppState::new(Arc::new(source), sink, store, config);
    state.boot();
    state
}

// Pre-warms the cache as if an earlier request had already resolved this
// slug, so a GET renders it deterministically without a background refresh.
fn seed(state: &AppState, slug: &str, raw_messages: &[Vec<u8>]) {
    state.cache().store(slug, raw_messages).unwrap();
}

async fn get_html(app: Router, uri: &str) -> (StatusCode, String) {
    let response = app
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (status, String::from_utf8(body.to_vec()).unwrap())
}

async fn wait_for<F: Fn() -> bool>(condition: F) {
    for _ in 0..50 {
        if condition() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[tokio::test]
async fn renders_an_existing_thread() {
    let app_state = state(
        FixtureSource {
            raw_messages: vec![ROOT.to_vec()],
        },
        Arc::new(FixtureSink::default()),
    );
    seed(&app_state, "my-post", &[ROOT.to_vec()]);

    let (status, html) = get_html(WebState::new(app_state).router(), "/thread/my-post").await;

    assert_eq!(status, StatusCode::OK);
    assert!(html.contains("Alice"));
    assert!(html.contains("Great post!"));
}

#[tokio::test]
async fn renders_an_empty_state_when_a_confirmed_search_found_nothing() {
    let app_state = state(
        FixtureSource {
            raw_messages: Vec::new(),
        },
        Arc::new(FixtureSink::default()),
    );
    // A fresh (not stale) empty entry represents a completed search that
    // genuinely found nothing, as opposed to a cache that's simply cold.
    seed(&app_state, "no-such-post", &[]);

    let (status, html) = get_html(WebState::new(app_state).router(), "/thread/no-such-post").await;

    assert_eq!(status, StatusCode::OK);
    assert!(html.contains("No comments yet"));
    assert!(!html.contains("class=\"stale-notice\""));
    assert!(html.contains("mailto:group@googlegroups.com?subject=no-such-post"));
}

#[tokio::test]
async fn empty_state_still_offers_a_top_level_comment_form_when_relay_is_enabled() {
    let app = WebState::new(state(
        FixtureSource {
            raw_messages: Vec::new(),
        },
        Arc::new(FixtureSink::default()),
    ))
    .router();

    let (_, html) = get_html(app, "/thread/no-such-post").await;

    assert!(html.contains("<form"));
    assert!(html.contains("name=\"in_reply_to\" value=\"\""));
}

#[tokio::test]
async fn omits_the_comment_form_when_relay_comments_is_disabled() {
    let app_state = state_with(
        FixtureSource {
            raw_messages: vec![ROOT.to_vec()],
        },
        Arc::new(FixtureSink::default()),
        false,
        true,
    );
    seed(&app_state, "my-post", &[ROOT.to_vec()]);

    let (_, html) = get_html(WebState::new(app_state).router(), "/thread/my-post").await;

    assert!(!html.contains("<form"));
    assert!(html.contains("mailto:group@googlegroups.com?subject=my-post"));
}

#[tokio::test]
async fn omits_the_mailto_hint_when_show_email_link_is_disabled() {
    let app_state = state_with(
        FixtureSource {
            raw_messages: vec![ROOT.to_vec()],
        },
        Arc::new(FixtureSink::default()),
        true,
        false,
    );
    seed(&app_state, "my-post", &[ROOT.to_vec()]);

    let (_, html) = get_html(WebState::new(app_state).router(), "/thread/my-post").await;

    assert!(html.contains("<form"));
    assert!(!html.contains("mailto:"));
}

#[tokio::test]
async fn submitting_a_comment_redirects_immediately_and_relays_it_in_the_background() {
    let sink = Arc::new(FixtureSink::default());
    let app_state = state(
        FixtureSource {
            raw_messages: vec![ROOT.to_vec()],
        },
        sink.clone(),
    );
    let app = WebState::new(app_state.clone()).router();

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/thread/my-post")
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from(
                    "name=Bob&body=I+agree&in_reply_to=root%40example.com",
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        response.headers().get("location").unwrap(),
        "/thread/my-post"
    );

    wait_for(|| !sink.sent.lock().unwrap().is_empty()).await;

    let sent = sink.sent.lock().unwrap();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].author.display_name, "Bob (via web)");
    assert_eq!(sent[0].in_reply_to.as_deref(), Some("root@example.com"));
}

#[tokio::test]
async fn a_submitted_comments_subject_carries_the_configured_prefix_and_suffix() {
    let sink = Arc::new(FixtureSink::default());
    let app_state = state_with_subject(
        FixtureSource {
            raw_messages: vec![ROOT.to_vec()],
        },
        sink.clone(),
        true,
        true,
        NO_REFRESH_NEEDED,
        "Blog Comments: ",
        " (blog)",
    );
    let app = WebState::new(app_state.clone()).router();

    app.oneshot(
        Request::builder()
            .method("POST")
            .uri("/thread/my-post")
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from("name=Bob&body=I+agree&in_reply_to="))
            .unwrap(),
    )
    .await
    .unwrap();

    wait_for(|| !sink.sent.lock().unwrap().is_empty()).await;

    let sent = sink.sent.lock().unwrap();
    assert_eq!(sent[0].subject, "Blog Comments: my-post (blog)");
}

#[tokio::test]
async fn repeated_identical_submissions_only_relay_once() {
    let sink = Arc::new(FixtureSink::default());
    let app_state = state(
        FixtureSource {
            raw_messages: vec![ROOT.to_vec()],
        },
        sink.clone(),
    );
    let app = WebState::new(app_state.clone()).router();

    for _ in 0..3 {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/thread/my-post")
                    .header("content-type", "application/x-www-form-urlencoded")
                    .body(Body::from(
                        "name=Bob&body=I+agree&in_reply_to=root%40example.com",
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::SEE_OTHER);
    }

    wait_for(|| !sink.sent.lock().unwrap().is_empty()).await;

    assert_eq!(sink.sent.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn a_different_comment_after_a_duplicate_still_relays() {
    let sink = Arc::new(FixtureSink::default());
    let app_state = state(
        FixtureSource {
            raw_messages: vec![ROOT.to_vec()],
        },
        sink.clone(),
    );
    let app = WebState::new(app_state.clone()).router();

    for body in [
        "name=Bob&body=I+agree",
        "name=Bob&body=I+agree",
        "name=Bob&body=Actually+wait",
    ] {
        app.clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/thread/my-post")
                    .header("content-type", "application/x-www-form-urlencoded")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
    }

    wait_for(|| sink.sent.lock().unwrap().len() >= 2).await;

    assert_eq!(sink.sent.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn a_slug_with_slashes_routes_correctly_for_get_and_post() {
    let sink = Arc::new(FixtureSink::default());
    let app_state = state(
        FixtureSource {
            raw_messages: vec![ROOT.to_vec()],
        },
        sink.clone(),
    );
    seed(&app_state, "posts/2026-07-12-example", &[ROOT.to_vec()]);
    let app = WebState::new(app_state.clone()).router();

    let (status, html) = get_html(app.clone(), "/thread/posts/2026-07-12-example").await;
    assert_eq!(status, StatusCode::OK);
    assert!(html.contains("action=\"/thread/posts/2026-07-12-example\""));

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/thread/posts/2026-07-12-example")
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from("name=Bob&body=I+agree&in_reply_to="))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        response.headers().get("location").unwrap(),
        "/thread/posts/2026-07-12-example"
    );

    wait_for(|| !sink.sent.lock().unwrap().is_empty()).await;
    assert_eq!(sink.sent.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn shows_a_persistent_latency_notice_whenever_relay_is_enabled() {
    let app_state = state(
        FixtureSource {
            raw_messages: vec![ROOT.to_vec()],
        },
        Arc::new(FixtureSink::default()),
    );
    seed(&app_state, "my-post", &[ROOT.to_vec()]);
    let app = WebState::new(app_state).router();

    let (_, html) = get_html(app, "/thread/my-post").await;

    assert!(html.contains("class=\"latency-notice\""));
}

#[tokio::test]
async fn omits_the_latency_notice_when_relay_comments_is_disabled() {
    let app_state = state_with(
        FixtureSource {
            raw_messages: vec![ROOT.to_vec()],
        },
        Arc::new(FixtureSink::default()),
        false,
        true,
    );
    seed(&app_state, "my-post", &[ROOT.to_vec()]);
    let app = WebState::new(app_state).router();

    let (_, html) = get_html(app, "/thread/my-post").await;

    assert!(!html.contains("class=\"latency-notice\""));
}

#[tokio::test]
async fn rejects_a_comment_submission_when_relay_comments_is_disabled() {
    let sink = Arc::new(FixtureSink::default());
    let app = WebState::new(state_with(
        FixtureSource {
            raw_messages: vec![ROOT.to_vec()],
        },
        sink.clone(),
        false,
        true,
    ))
    .router();

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/thread/my-post/comment")
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from("name=Bob&body=I+agree&in_reply_to="))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert!(sink.sent.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_cold_cache_renders_a_checking_notice_rather_than_claiming_no_comments() {
    let app = WebState::new(state(
        FixtureSource {
            raw_messages: vec![ROOT.to_vec()],
        },
        Arc::new(FixtureSink::default()),
    ))
    .router();

    let (status, html) = get_html(app, "/thread/my-post").await;

    assert_eq!(status, StatusCode::OK);
    // A cold cache hasn't confirmed there's nothing there yet, so it must
    // not claim "No comments yet" — that would be misleading if the thread
    // actually has comments the first search just hasn't found yet.
    assert!(!html.contains("No comments yet"));
    assert!(html.contains("class=\"stale-notice\""));
}

#[tokio::test]
async fn the_page_always_carries_a_periodic_refresh_tag_regardless_of_cache_freshness() {
    let app_state = state(
        FixtureSource {
            raw_messages: vec![ROOT.to_vec()],
        },
        Arc::new(FixtureSink::default()),
    );
    seed(&app_state, "my-post", &[ROOT.to_vec()]);

    let (_, fresh) = get_html(WebState::new(app_state).router(), "/thread/my-post").await;
    let (_, cold) = get_html(
        WebState::new(state(
            FixtureSource {
                raw_messages: Vec::new(),
            },
            Arc::new(FixtureSink::default()),
        ))
        .router(),
        "/thread/no-such-post",
    )
    .await;

    assert!(fresh.contains("http-equiv=\"refresh\""));
    assert!(cold.contains("http-equiv=\"refresh\""));
}

#[tokio::test]
async fn a_stale_notice_shows_only_for_content_past_the_cache_ttl() {
    let fresh_state = state_with_ttl(
        FixtureSource {
            raw_messages: vec![ROOT.to_vec()],
        },
        Arc::new(FixtureSink::default()),
        true,
        true,
        NO_REFRESH_NEEDED,
    );
    seed(&fresh_state, "my-post", &[ROOT.to_vec()]);
    let (_, fresh) = get_html(WebState::new(fresh_state).router(), "/thread/my-post").await;

    let stale_state = state_with_ttl(
        FixtureSource {
            raw_messages: vec![ROOT.to_vec()],
        },
        Arc::new(FixtureSink::default()),
        true,
        true,
        0,
    );
    seed(&stale_state, "my-post", &[ROOT.to_vec()]);
    let (_, stale) = get_html(WebState::new(stale_state).router(), "/thread/my-post").await;

    assert!(!fresh.contains("class=\"stale-notice\""));
    assert!(stale.contains("class=\"stale-notice\""));
}

#[tokio::test]
async fn the_refresh_tag_reflects_the_configured_interval() {
    let store = Arc::new(SqliteStore::open(":memory:").unwrap());
    let mut config = test_config();
    config.web.refresh_interval_secs = 45;
    let app_state = AppState::new(
        Arc::new(FixtureSource {
            raw_messages: vec![ROOT.to_vec()],
        }),
        Arc::new(FixtureSink::default()),
        store,
        config,
    );
    seed(&app_state, "my-post", &[ROOT.to_vec()]);

    let (_, html) = get_html(WebState::new(app_state).router(), "/thread/my-post").await;

    assert!(html.contains("content=\"45\""));
}

#[tokio::test]
async fn a_background_refresh_populates_the_cache_for_a_later_request() {
    let app_state = state_with_ttl(
        FixtureSource {
            raw_messages: vec![ROOT.to_vec()],
        },
        Arc::new(FixtureSink::default()),
        true,
        true,
        NO_REFRESH_NEEDED,
    );
    let app = WebState::new(app_state.clone()).router();

    // Cold cache: kicks off a background refresh but renders a checking
    // notice for now, since it hasn't confirmed there's nothing there.
    let (_, first) = get_html(app.clone(), "/thread/my-post").await;
    assert!(first.contains("class=\"stale-notice\""));

    wait_for(|| {
        !app_state
            .cache()
            .get("my-post")
            .unwrap()
            .raw_messages
            .is_empty()
    })
    .await;

    let (_, second) = get_html(app, "/thread/my-post").await;
    assert!(second.contains("Great post!"));
}

struct RecordingSource {
    searched: Arc<Mutex<Vec<String>>>,
}

impl MailSource for RecordingSource {
    fn search_subject(&self, subject: &str) -> Result<Vec<Vec<u8>>, crate::source::BoxError> {
        self.searched.lock().unwrap().push(subject.to_string());
        Ok(Vec::new())
    }
}

#[tokio::test]
async fn the_imap_search_uses_the_slug_wrapped_in_the_configured_prefix_and_suffix() {
    let searched = Arc::new(Mutex::new(Vec::new()));
    let source = RecordingSource {
        searched: searched.clone(),
    };
    let store = Arc::new(SqliteStore::open(":memory:").unwrap());
    let mut config = test_config();
    config.mailing_list.subject_prefix = "Blog Comments: ".to_string();
    config.mailing_list.subject_suffix = " (blog)".to_string();
    let app_state = AppState::new(
        Arc::new(source),
        Arc::new(FixtureSink::default()),
        store,
        config,
    );
    let app = WebState::new(app_state).router();

    get_html(app, "/thread/my-post").await;

    wait_for(|| !searched.lock().unwrap().is_empty()).await;

    assert_eq!(
        searched.lock().unwrap().as_slice(),
        ["Blog Comments: my-post (blog)"]
    );
}

#[tokio::test]
async fn a_successful_submission_triggers_an_immediate_re_search_so_the_reply_shows_up_soon() {
    let app_state = state(
        FixtureSource {
            raw_messages: vec![ROOT.to_vec()],
        },
        Arc::new(FixtureSink::default()),
    );
    seed(&app_state, "my-post", &[ROOT.to_vec()]);
    let app = WebState::new(app_state.clone()).router();

    app.oneshot(
        Request::builder()
            .method("POST")
            .uri("/thread/my-post")
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from("name=Bob&body=I+agree&in_reply_to="))
            .unwrap(),
    )
    .await
    .unwrap();

    // Once the sink reports the comment delivered, it re-triggers a search.
    wait_for(|| {
        app_state
            .cache()
            .get("my-post")
            .unwrap()
            .refreshed_at
            .is_some()
    })
    .await;

    assert!(
        app_state
            .cache()
            .get("my-post")
            .unwrap()
            .refreshed_at
            .is_some()
    );
}

#[tokio::test]
async fn healthz_returns_ok_when_the_mail_sink_is_healthy() {
    let app_state = state(
        FixtureSource {
            raw_messages: Vec::new(),
        },
        Arc::new(FixtureSink::default()),
    );

    let (status, body) = get_html(WebState::new(app_state).router(), "/healthz").await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, "ok");
}

#[tokio::test]
async fn healthz_returns_service_unavailable_when_the_mail_sink_is_unhealthy() {
    let app_state = state(
        FixtureSource {
            raw_messages: Vec::new(),
        },
        Arc::new(UnhealthySink),
    );

    let (status, _) = get_html(WebState::new(app_state).router(), "/healthz").await;

    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn a_submitted_top_level_comment_appears_once_the_thread_reloads() {
    let mailbox = crate::testing::SharedMailbox::default();
    let store = Arc::new(SqliteStore::open(":memory:").unwrap());
    let app_state = AppState::new(
        Arc::new(mailbox.source()),
        Arc::new(mailbox.sink()),
        store,
        test_config(),
    );
    app_state.boot();
    seed(&app_state, "my-post", &[]);
    let app = WebState::new(app_state.clone()).router();

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/thread/my-post")
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from("name=Bob&body=First+comment&in_reply_to="))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);

    wait_for(|| {
        !app_state
            .cache()
            .get("my-post")
            .unwrap()
            .raw_messages
            .is_empty()
    })
    .await;

    let (status, html) = get_html(app, "/thread/my-post").await;

    assert_eq!(status, StatusCode::OK);
    assert!(html.contains("Bob (via web)"));
    assert!(html.contains("First comment"));
}

#[tokio::test]
async fn a_submitted_reply_nests_under_its_parent_once_the_thread_reloads() {
    let mailbox = crate::testing::SharedMailbox::default();
    mailbox.seed(&[ROOT.to_vec()]);
    let store = Arc::new(SqliteStore::open(":memory:").unwrap());
    let app_state = AppState::new(
        Arc::new(mailbox.source()),
        Arc::new(mailbox.sink()),
        store,
        test_config(),
    );
    app_state.boot();
    seed(&app_state, "my-post", &[ROOT.to_vec()]);
    let app = WebState::new(app_state.clone()).router();

    app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/thread/my-post")
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from(
                    "name=Bob&body=I+agree&in_reply_to=root%40example.com",
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    wait_for(|| app_state.cache().get("my-post").unwrap().raw_messages.len() == 2).await;

    let (_, html) = get_html(app, "/thread/my-post").await;

    // The reply must render inside the root's own `<ol class="replies">`,
    // not merely appear somewhere later in the page.
    let root_idx = html.find("Great post!").expect("root comment missing");
    let replies_idx = html
        .find("class=\"replies\"")
        .expect("no nested replies list");
    let reply_idx = html.find("I agree").expect("reply comment missing");

    assert!(root_idx < replies_idx);
    assert!(replies_idx < reply_idx);
    assert!(html.contains("Bob (via web)"));
}
