use std::sync::Mutex;
use std::time::{Duration, Instant};

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

use crate::config::{ImapConfig, MailingListConfig, SmtpConfig, StorageConfig, Theme, WebConfig};
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

#[derive(Default)]
struct FixtureSink {
    sent: Mutex<Vec<crate::mail::Message>>,
}

impl MailSink for FixtureSink {
    fn submit(&self, _envelope: &Envelope, raw: &[u8]) -> Result<(), crate::source::BoxError> {
        self.sent
            .lock()
            .unwrap()
            .push(crate::mail::Message::parse(raw, None).unwrap());
        Ok(())
    }
}

// Always fails, to exercise the outgoing comment queue's retry/expiry paths deterministically.
#[derive(Default)]
struct FailingSink;

impl MailSink for FailingSink {
    fn submit(&self, _envelope: &Envelope, _raw: &[u8]) -> Result<(), crate::source::BoxError> {
        Err("the mail provider is unreachable".into())
    }
}

// Always fails, to exercise the /healthz failure path deterministically.
struct FailingStore;

impl OutgoingCommentStore for FailingStore {
    fn enqueue(
        &self,
        _slug: &str,
        _from_address: &str,
        _to_address: &str,
        _raw: &[u8],
    ) -> Result<OutgoingComment, crate::source::BoxError> {
        Err("the comment store is unreachable".into())
    }

    fn pending(&self) -> Result<Vec<OutgoingComment>, crate::source::BoxError> {
        Err("the comment store is unreachable".into())
    }

    fn remove(&self, _id: i64) -> Result<(), crate::source::BoxError> {
        Err("the comment store is unreachable".into())
    }
}

// Long enough that nothing in these tests ever hits it by accident; tests
// that care about expiry pass an explicit short TTL to `AppState::new`.
const NO_REFRESH_NEEDED: u64 = 3600;
const NO_OUTBOX_EXPIRY: u64 = 3600;
const TEST_SWEEP_INTERVAL_SECS: u64 = 1;

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
        },
        smtp: SmtpConfig {
            host: "smtp.example.com".to_string(),
            port: 587,
            username: String::new(),
            password: String::new(),
            max_concurrent_submits: 4,
        },
        storage: StorageConfig {
            path: ":memory:".to_string(),
            incoming_message_ttl_secs: NO_REFRESH_NEEDED,
            outgoing_message_ttl_secs: NO_OUTBOX_EXPIRY,
            outgoing_comment_sweep_interval_secs: TEST_SWEEP_INTERVAL_SECS,
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
    AppState::new(Arc::new(source), sink, store.clone(), store, config)
}

// Pre-warms the cache as if an earlier request had already resolved this
// slug, so a GET renders it deterministically without a background refresh.
fn seed(state: &AppState, slug: &str, raw_messages: &[Vec<u8>]) {
    state.cache.store(slug, raw_messages).unwrap();
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

    let (status, html) = get_html(router(app_state), "/thread/my-post").await;

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

    let (status, html) = get_html(router(app_state), "/thread/no-such-post").await;

    assert_eq!(status, StatusCode::OK);
    assert!(html.contains("No comments yet"));
    assert!(!html.contains("class=\"stale-notice\""));
    assert!(html.contains("mailto:group@googlegroups.com?subject=no-such-post"));
}

#[tokio::test]
async fn empty_state_still_offers_a_top_level_comment_form_when_relay_is_enabled() {
    let app = router(state(
        FixtureSource {
            raw_messages: Vec::new(),
        },
        Arc::new(FixtureSink::default()),
    ));

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

    let (_, html) = get_html(router(app_state), "/thread/my-post").await;

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

    let (_, html) = get_html(router(app_state), "/thread/my-post").await;

    assert!(html.contains("<form"));
    assert!(!html.contains("mailto:"));
}

#[tokio::test]
async fn submitting_a_comment_redirects_immediately_and_relays_it_in_the_background() {
    let sink = Arc::new(FixtureSink::default());
    let app = router(state(
        FixtureSource {
            raw_messages: vec![ROOT.to_vec()],
        },
        sink.clone(),
    ));

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
    let app = router(state_with_subject(
        FixtureSource {
            raw_messages: vec![ROOT.to_vec()],
        },
        sink.clone(),
        true,
        true,
        NO_REFRESH_NEEDED,
        "Blog Comments: ",
        " (blog)",
    ));

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
    let app = router(state(
        FixtureSource {
            raw_messages: vec![ROOT.to_vec()],
        },
        sink.clone(),
    ));

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
    let app = router(state(
        FixtureSource {
            raw_messages: vec![ROOT.to_vec()],
        },
        sink.clone(),
    ));

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
    let app = router(app_state);

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
    let app = router(app_state);

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
    let app = router(app_state);

    let (_, html) = get_html(app, "/thread/my-post").await;

    assert!(!html.contains("class=\"latency-notice\""));
}

#[tokio::test]
async fn rejects_a_comment_submission_when_relay_comments_is_disabled() {
    let sink = Arc::new(FixtureSink::default());
    let app = router(state_with(
        FixtureSource {
            raw_messages: vec![ROOT.to_vec()],
        },
        sink.clone(),
        false,
        true,
    ));

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
    let app = router(state(
        FixtureSource {
            raw_messages: vec![ROOT.to_vec()],
        },
        Arc::new(FixtureSink::default()),
    ));

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

    let (_, fresh) = get_html(router(app_state), "/thread/my-post").await;
    let (_, cold) = get_html(
        router(state(
            FixtureSource {
                raw_messages: Vec::new(),
            },
            Arc::new(FixtureSink::default()),
        )),
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
    let (_, fresh) = get_html(router(fresh_state), "/thread/my-post").await;

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
    let (_, stale) = get_html(router(stale_state), "/thread/my-post").await;

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
        store.clone(),
        store,
        config,
    );
    seed(&app_state, "my-post", &[ROOT.to_vec()]);

    let (_, html) = get_html(router(app_state), "/thread/my-post").await;

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
    let cache = app_state.cache.clone();
    let app = router(app_state);

    // Cold cache: kicks off a background refresh but renders a checking
    // notice for now, since it hasn't confirmed there's nothing there.
    let (_, first) = get_html(app.clone(), "/thread/my-post").await;
    assert!(first.contains("class=\"stale-notice\""));

    wait_for(|| !cache.get("my-post").unwrap().raw_messages.is_empty()).await;

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
        store.clone(),
        store,
        config,
    );
    let app = router(app_state);

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
    let cache = app_state.cache.clone();
    let app = router(app_state);

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

    // The submission invalidates the cache and, once the queued comment is
    // actually delivered, re-triggers a background search; wait for that
    // search to land and mark the slug refreshed again.
    wait_for(|| cache.get("my-post").unwrap().refreshed_at.is_some()).await;

    assert!(cache.get("my-post").unwrap().refreshed_at.is_some());
}

#[tokio::test]
async fn a_queued_comment_is_removed_from_outgoing_comments_once_delivered() {
    let sink = Arc::new(FixtureSink::default());
    let app_state = state(
        FixtureSource {
            raw_messages: vec![ROOT.to_vec()],
        },
        sink,
    );
    let outgoing_comments = app_state.outgoing_comments.clone();
    let app = router(app_state);

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

    wait_for(|| outgoing_comments.pending().unwrap().is_empty()).await;

    assert!(outgoing_comments.pending().unwrap().is_empty());
}

#[tokio::test]
async fn a_failed_delivery_stays_queued_for_retry() {
    let app_state = state(
        FixtureSource {
            raw_messages: vec![ROOT.to_vec()],
        },
        Arc::new(FailingSink),
    );
    let outgoing_comments = app_state.outgoing_comments.clone();
    let app = router(app_state);

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

    // Give the first (failing) attempt a moment to run and settle.
    tokio::time::sleep(Duration::from_millis(100)).await;

    assert_eq!(outgoing_comments.pending().unwrap().len(), 1);
}

#[tokio::test]
async fn an_expired_pending_comment_is_dropped_without_being_sent() {
    let sink = Arc::new(FixtureSink::default());
    let store = Arc::new(SqliteStore::open(":memory:").unwrap());
    let mut config = test_config();
    config.storage.outgoing_message_ttl_secs = 0; // already expired the instant it's queued
    let app_state = AppState::new(
        Arc::new(FixtureSource {
            raw_messages: Vec::new(),
        }),
        sink.clone(),
        store.clone(),
        store,
        config,
    );
    let outgoing_comments = app_state.outgoing_comments.clone();
    let app = router(app_state);

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

    wait_for(|| outgoing_comments.pending().unwrap().is_empty()).await;

    assert!(outgoing_comments.pending().unwrap().is_empty());
    assert!(sink.sent.lock().unwrap().is_empty());
}

#[tokio::test]
async fn the_outgoing_comment_worker_delivers_messages_left_over_from_a_previous_run() {
    let sink = Arc::new(FixtureSink::default());
    let app_state = state(
        FixtureSource {
            raw_messages: Vec::new(),
        },
        sink.clone(),
    );
    // Simulate a comment that was queued before a restart, with no HTTP
    // request involved this time.
    app_state
        .outgoing_comments
        .enqueue(
            "my-post",
            "bot@ourdomain.example",
            "group@googlegroups.com",
            ROOT,
        )
        .unwrap();

    spawn_outgoing_comment_worker(app_state);

    wait_for(|| !sink.sent.lock().unwrap().is_empty()).await;

    assert_eq!(sink.sent.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn healthz_returns_ok_when_the_store_is_reachable() {
    let app_state = state(
        FixtureSource {
            raw_messages: Vec::new(),
        },
        Arc::new(FixtureSink::default()),
    );
    *app_state.last_outgoing_sweep.lock().unwrap() = Some(SweepHeartbeat::now(
        Duration::from_secs(TEST_SWEEP_INTERVAL_SECS),
    ));

    let (status, body) = get_html(router(app_state), "/healthz").await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, "ok");
}

#[tokio::test]
async fn healthz_returns_service_unavailable_when_the_store_is_unreachable() {
    let cache = Arc::new(SqliteStore::open(":memory:").unwrap());
    let app_state = AppState::new(
        Arc::new(FixtureSource {
            raw_messages: Vec::new(),
        }),
        Arc::new(FixtureSink::default()),
        cache,
        Arc::new(FailingStore),
        test_config(),
    );

    let (status, _) = get_html(router(app_state), "/healthz").await;

    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn healthz_returns_service_unavailable_when_the_outgoing_worker_has_stalled() {
    let app_state = state(
        FixtureSource {
            raw_messages: Vec::new(),
        },
        Arc::new(FixtureSink::default()),
    );
    *app_state.last_outgoing_sweep.lock().unwrap() = Some(SweepHeartbeat {
        interval: Duration::from_secs(TEST_SWEEP_INTERVAL_SECS),
        last_swept: Instant::now() - Duration::from_secs(3600),
    });

    let (status, _) = get_html(router(app_state), "/healthz").await;

    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn healthz_returns_service_unavailable_when_the_outgoing_worker_has_never_ticked() {
    let app_state = state(
        FixtureSource {
            raw_messages: Vec::new(),
        },
        Arc::new(FixtureSink::default()),
    );

    let (status, _) = get_html(router(app_state), "/healthz").await;

    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn the_outgoing_worker_sets_its_heartbeat_on_each_tick() {
    let app_state = state(
        FixtureSource {
            raw_messages: Vec::new(),
        },
        Arc::new(FixtureSink::default()),
    );
    assert!(app_state.last_outgoing_sweep.lock().unwrap().is_none());

    spawn_outgoing_comment_worker(app_state.clone());

    wait_for(|| app_state.last_outgoing_sweep.lock().unwrap().is_some()).await;

    assert!(app_state.last_outgoing_sweep.lock().unwrap().is_some());
}
