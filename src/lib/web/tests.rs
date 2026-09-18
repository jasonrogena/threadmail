use std::sync::Mutex;
use std::time::Duration;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

use crate::sqlite_cache::SqliteCache;

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
    fn submit(&self, message: &lettre::Message) -> Result<(), crate::source::BoxError> {
        self.sent
            .lock()
            .unwrap()
            .push(crate::mail::Message::parse(&message.formatted(), None).unwrap());
        Ok(())
    }
}

// A long TTL so tests that pre-seed the cache get a deterministic render
// with no background refresh racing the assertion; tests that care about
// the refresh itself use `state_with_ttl` instead.
const NO_REFRESH_NEEDED: u64 = 3600;

fn state(source: FixtureSource, sink: Arc<FixtureSink>) -> AppState {
    state_with(source, sink, true, true)
}

fn state_with(
    source: FixtureSource,
    sink: Arc<FixtureSink>,
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
    sink: Arc<FixtureSink>,
    relay_comments: bool,
    show_email_link: bool,
    cache_ttl_secs: u64,
) -> AppState {
    state_with_subject(
        source,
        sink,
        relay_comments,
        show_email_link,
        cache_ttl_secs,
        "",
        "",
    )
}

#[allow(clippy::too_many_arguments)]
fn state_with_subject(
    source: FixtureSource,
    sink: Arc<FixtureSink>,
    relay_comments: bool,
    show_email_link: bool,
    cache_ttl_secs: u64,
    subject_prefix: &str,
    subject_suffix: &str,
) -> AppState {
    AppState::new(
        Arc::new(source),
        sink,
        Arc::new(SqliteCache::open_in_memory().unwrap()),
        cache_ttl_secs,
        60,
        "bot@ourdomain.example".to_string(),
        "group@googlegroups.com".to_string(),
        relay_comments,
        show_email_link,
        "auto".to_string(),
        None,
        subject_prefix.to_string(),
        subject_suffix.to_string(),
        8,
        4,
    )
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
async fn submitting_a_comment_relays_it_and_redirects_back_to_the_thread() {
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
        "/thread/my-post?posted=1"
    );

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
        "/thread/posts/2026-07-12-example?posted=1"
    );
    assert_eq!(sink.sent.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn shows_a_posted_notice_only_when_the_query_param_is_present() {
    let app_state = state(
        FixtureSource {
            raw_messages: vec![ROOT.to_vec()],
        },
        Arc::new(FixtureSink::default()),
    );
    seed(&app_state, "my-post", &[ROOT.to_vec()]);
    let app = router(app_state);

    let (_, plain) = get_html(app.clone(), "/thread/my-post").await;
    let (_, posted) = get_html(app, "/thread/my-post?posted=1").await;

    assert!(!plain.contains("class=\"posted-notice\""));
    assert!(posted.contains("class=\"posted-notice\""));
}

#[tokio::test]
async fn the_posted_page_reloads_itself_to_the_clean_url_so_the_notice_clears() {
    let app_state = state(
        FixtureSource {
            raw_messages: vec![ROOT.to_vec()],
        },
        Arc::new(FixtureSink::default()),
    );
    seed(&app_state, "my-post", &[ROOT.to_vec()]);
    let app = router(app_state);

    let (_, plain) = get_html(app.clone(), "/thread/my-post").await;
    let (_, posted) = get_html(app, "/thread/my-post?posted=1").await;

    assert!(!plain.contains(";url="));
    assert!(posted.contains(";url=/thread/my-post\""));
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
    let app_state = AppState::new(
        Arc::new(FixtureSource {
            raw_messages: vec![ROOT.to_vec()],
        }),
        Arc::new(FixtureSink::default()),
        Arc::new(SqliteCache::open_in_memory().unwrap()),
        NO_REFRESH_NEEDED,
        45,
        "bot@ourdomain.example".to_string(),
        "group@googlegroups.com".to_string(),
        true,
        true,
        "auto".to_string(),
        None,
        "".to_string(),
        "".to_string(),
        8,
        4,
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

    // Give the spawned refresh task a chance to run and write the cache.
    let mut entry = cache.get("my-post").unwrap();
    for _ in 0..50 {
        if !entry.raw_messages.is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
        entry = cache.get("my-post").unwrap();
    }

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
    let app_state = AppState::new(
        Arc::new(source),
        Arc::new(FixtureSink::default()),
        Arc::new(SqliteCache::open_in_memory().unwrap()),
        NO_REFRESH_NEEDED,
        60,
        "bot@ourdomain.example".to_string(),
        "group@googlegroups.com".to_string(),
        true,
        true,
        "auto".to_string(),
        None,
        "Blog Comments: ".to_string(),
        " (blog)".to_string(),
        8,
        4,
    );
    let app = router(app_state);

    get_html(app, "/thread/my-post").await;

    for _ in 0..50 {
        if !searched.lock().unwrap().is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

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

    // The submission invalidates the cache and immediately re-triggers a
    // background search (rather than waiting for the next stale page view);
    // wait for that search to land and mark the slug refreshed again.
    let mut entry = cache.get("my-post").unwrap();
    for _ in 0..50 {
        if entry.refreshed_at.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
        entry = cache.get("my-post").unwrap();
    }

    assert!(entry.refreshed_at.is_some());
}
