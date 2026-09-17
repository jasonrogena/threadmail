use std::sync::Mutex;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

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
            .push(crate::mail::parse(&message.formatted(), None).unwrap());
        Ok(())
    }
}

fn state(source: FixtureSource, sink: Arc<FixtureSink>) -> AppState {
    state_with(source, sink, true, true)
}

fn state_with(
    source: FixtureSource,
    sink: Arc<FixtureSink>,
    relay_comments: bool,
    show_email_link: bool,
) -> AppState {
    AppState::new(
        Arc::new(source),
        sink,
        "bot@ourdomain.example".to_string(),
        "group@googlegroups.com".to_string(),
        relay_comments,
        show_email_link,
        "auto".to_string(),
        None,
        8,
        4,
    )
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
    let app = router(state(
        FixtureSource {
            raw_messages: vec![ROOT.to_vec()],
        },
        Arc::new(FixtureSink::default()),
    ));

    let (status, html) = get_html(app, "/thread/my-post").await;

    assert_eq!(status, StatusCode::OK);
    assert!(html.contains("Alice"));
    assert!(html.contains("Great post!"));
}

#[tokio::test]
async fn renders_an_empty_state_when_no_thread_exists_yet() {
    let app = router(state(
        FixtureSource {
            raw_messages: Vec::new(),
        },
        Arc::new(FixtureSink::default()),
    ));

    let (status, html) = get_html(app, "/thread/no-such-post").await;

    assert_eq!(status, StatusCode::OK);
    assert!(html.contains("No comments yet"));
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
    let app = router(state_with(
        FixtureSource {
            raw_messages: vec![ROOT.to_vec()],
        },
        Arc::new(FixtureSink::default()),
        false,
        true,
    ));

    let (_, html) = get_html(app, "/thread/my-post").await;

    assert!(!html.contains("<form"));
    assert!(html.contains("mailto:group@googlegroups.com?subject=my-post"));
}

#[tokio::test]
async fn omits_the_mailto_hint_when_show_email_link_is_disabled() {
    let app = router(state_with(
        FixtureSource {
            raw_messages: vec![ROOT.to_vec()],
        },
        Arc::new(FixtureSink::default()),
        true,
        false,
    ));

    let (_, html) = get_html(app, "/thread/my-post").await;

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
                .uri("/thread/my-post/comment")
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
    assert_eq!(sent[0].display_name, "Bob (via web)");
    assert_eq!(sent[0].in_reply_to.as_deref(), Some("root@example.com"));
}

#[tokio::test]
async fn shows_a_posted_notice_only_when_the_query_param_is_present() {
    let app = router(state(
        FixtureSource {
            raw_messages: vec![ROOT.to_vec()],
        },
        Arc::new(FixtureSink::default()),
    ));

    let (_, plain) = get_html(app.clone(), "/thread/my-post").await;
    let (_, posted) = get_html(app, "/thread/my-post?posted=1").await;

    assert!(!plain.contains("class=\"posted-notice\""));
    assert!(posted.contains("class=\"posted-notice\""));
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
