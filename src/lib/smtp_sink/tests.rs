use std::sync::Mutex;

use lettre::transport::stub::StubTransport;

use crate::sqlite_store::SqliteStore;

use super::*;

const ROOT: &[u8] = b"From: \"Alice\" <alice@example.com>\r\n\
Subject: my-post\r\n\
Message-ID: <root@example.com>\r\n\
Date: Wed, 16 Sep 2026 09:00:00 +0000\r\n\
Content-Type: text/plain\r\n\
\r\n\
Great post!\r\n";

const NO_OUTGOING_COMMENT_EXPIRY: u64 = 3600;
const TEST_SWEEP_INTERVAL_SECS: u64 = 1;

// Always fails, to exercise the health check's unreachable-store path deterministically.
struct FailingStore;

impl OutgoingCommentStore for FailingStore {
    fn enqueue(
        &self,
        _slug: &str,
        _from_address: &str,
        _to_address: &str,
        _raw: &[u8],
    ) -> Result<OutgoingComment, BoxError> {
        Err("the comment store is unreachable".into())
    }

    fn pending(&self) -> Result<Vec<OutgoingComment>, BoxError> {
        Err("the comment store is unreachable".into())
    }

    fn remove(&self, _id: i64) -> Result<(), BoxError> {
        Err("the comment store is unreachable".into())
    }
}

fn smtp_config() -> SmtpConfig {
    SmtpConfig {
        host: "smtp.example.com".to_string(),
        port: 587,
        username: String::new(),
        password: String::new(),
        max_concurrent_deliveries: 4,
    }
}

fn storage_config(outgoing_message_ttl_secs: u64) -> StorageConfig {
    StorageConfig {
        path: ":memory:".to_string(),
        incoming_message_ttl_secs: 3600,
        outgoing_message_ttl_secs,
        outgoing_comment_sweep_interval_secs: TEST_SWEEP_INTERVAL_SECS,
    }
}

fn sink_with_ttl(
    transport: StubTransport,
    outgoing_comments: Arc<dyn OutgoingCommentStore>,
    outgoing_message_ttl_secs: u64,
) -> SmtpSink<StubTransport> {
    SmtpSink::with_transport(
        transport,
        &smtp_config(),
        &storage_config(outgoing_message_ttl_secs),
        outgoing_comments,
    )
}

fn sink(
    transport: StubTransport,
    outgoing_comments: Arc<dyn OutgoingCommentStore>,
) -> SmtpSink<StubTransport> {
    sink_with_ttl(transport, outgoing_comments, NO_OUTGOING_COMMENT_EXPIRY)
}

fn store() -> Arc<SqliteStore> {
    Arc::new(SqliteStore::open(":memory:").unwrap())
}

fn envelope() -> Envelope {
    Envelope::new(
        Some("bot@ourdomain.example".parse().unwrap()),
        vec!["group@googlegroups.com".parse().unwrap()],
    )
    .unwrap()
}

// Simulates a comment queued before a restart, bypassing submit so no worker is started.
fn enqueue_directly(outgoing_comments: &SqliteStore) {
    outgoing_comments
        .enqueue(
            "my-post",
            "bot@ourdomain.example",
            "group@googlegroups.com",
            ROOT,
        )
        .unwrap();
}

impl<T> SmtpSink<T> {
    fn is_running(&self) -> bool {
        self.inner.worker.lock().unwrap().is_some()
    }

    // The worker only notices an emptied outgoing comment queue on its next sweep, so this outlasts a full interval.
    async fn wait_for_worker_exit(&self) {
        let deadline = Instant::now() + Duration::from_secs(TEST_SWEEP_INTERVAL_SECS * 3);
        while self.is_running() && Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
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
async fn a_submitted_comment_is_delivered_in_the_background() {
    let transport = StubTransport::new_ok();
    let sink = sink(transport.clone(), store());

    sink.submit("my-post", &envelope(), ROOT).unwrap();
    wait_for(|| !transport.messages().is_empty()).await;

    let sent = transport.messages();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].1.as_bytes(), ROOT);
}

#[tokio::test]
async fn a_submitted_comment_is_removed_from_outgoing_comments_once_delivered() {
    let outgoing_comments = store();
    let sink = sink(StubTransport::new_ok(), outgoing_comments.clone());

    sink.submit("my-post", &envelope(), ROOT).unwrap();
    assert_eq!(outgoing_comments.pending().unwrap().len(), 1);
    wait_for(|| outgoing_comments.pending().unwrap().is_empty()).await;

    assert!(outgoing_comments.pending().unwrap().is_empty());
}

#[tokio::test]
async fn on_delivered_is_called_with_the_slug_once_a_comment_is_delivered() {
    let delivered = Arc::new(Mutex::new(Vec::new()));
    let sink = sink(StubTransport::new_ok(), store());
    let recorded = delivered.clone();
    sink.boot(Arc::new(move |slug| {
        recorded.lock().unwrap().push(slug.to_string())
    }));

    sink.submit("my-post", &envelope(), ROOT).unwrap();
    wait_for(|| !delivered.lock().unwrap().is_empty()).await;

    assert_eq!(delivered.lock().unwrap().as_slice(), ["my-post"]);
}

#[tokio::test]
async fn booting_delivers_comments_left_over_from_a_previous_run() {
    let transport = StubTransport::new_ok();
    let outgoing_comments = store();
    enqueue_directly(&outgoing_comments);
    let sink = sink(transport.clone(), outgoing_comments);

    sink.boot(Arc::new(|_| {}));
    wait_for(|| !transport.messages().is_empty()).await;

    assert_eq!(transport.messages().len(), 1);
}

#[tokio::test]
async fn a_failed_delivery_stays_queued_for_retry() {
    let outgoing_comments = store();
    let sink = sink(StubTransport::new_error(), outgoing_comments.clone());

    sink.submit("my-post", &envelope(), ROOT).unwrap();
    // Give the first (failing) attempt a moment to run and settle.
    tokio::time::sleep(Duration::from_millis(100)).await;

    assert_eq!(outgoing_comments.pending().unwrap().len(), 1);
}

#[tokio::test]
async fn an_expired_comment_is_dropped_without_being_sent() {
    let transport = StubTransport::new_ok();
    let outgoing_comments = store();
    // Already expired the instant it's queued.
    let sink = sink_with_ttl(transport.clone(), outgoing_comments.clone(), 0);

    sink.submit("my-post", &envelope(), ROOT).unwrap();
    wait_for(|| outgoing_comments.pending().unwrap().is_empty()).await;

    assert!(outgoing_comments.pending().unwrap().is_empty());
    assert!(transport.messages().is_empty());
}

#[tokio::test]
async fn submit_rejects_an_envelope_without_exactly_one_recipient() {
    let outgoing_comments = store();
    let sink = sink(StubTransport::new_ok(), outgoing_comments.clone());
    let envelope = Envelope::new(
        Some("bot@ourdomain.example".parse().unwrap()),
        vec![
            "group@googlegroups.com".parse().unwrap(),
            "other@googlegroups.com".parse().unwrap(),
        ],
    )
    .unwrap();

    assert!(sink.submit("my-post", &envelope, ROOT).is_err());
    assert!(outgoing_comments.pending().unwrap().is_empty());
}

#[tokio::test]
async fn the_worker_exits_once_the_outgoing_comment_queue_is_empty() {
    let transport = StubTransport::new_ok();
    let sink = sink(transport.clone(), store());

    sink.submit("my-post", &envelope(), ROOT).unwrap();
    assert!(sink.is_running());
    sink.wait_for_worker_exit().await;

    assert_eq!(transport.messages().len(), 1);
    assert!(!sink.is_running());
}

#[tokio::test]
async fn the_worker_exits_immediately_when_booted_with_nothing_queued() {
    let sink = sink(StubTransport::new_ok(), store());

    sink.boot(Arc::new(|_| {}));
    wait_for(|| !sink.is_running()).await;

    assert!(!sink.is_running());
}

#[tokio::test]
async fn a_submit_restarts_the_worker_after_it_has_exited() {
    let transport = StubTransport::new_ok();
    let sink = sink(transport.clone(), store());

    for _ in 0..2 {
        sink.submit("my-post", &envelope(), ROOT).unwrap();
        assert!(sink.is_running());
        sink.wait_for_worker_exit().await;
        assert!(!sink.is_running());
    }

    assert_eq!(transport.messages().len(), 2);
}

#[tokio::test]
async fn the_worker_keeps_running_while_a_delivery_is_still_failing() {
    let outgoing_comments = store();
    let sink = sink(StubTransport::new_error(), outgoing_comments.clone());

    sink.submit("my-post", &envelope(), ROOT).unwrap();
    tokio::time::sleep(Duration::from_secs(TEST_SWEEP_INTERVAL_SECS * 2)).await;

    assert!(sink.is_running());
    assert_eq!(outgoing_comments.pending().unwrap().len(), 1);
}

#[tokio::test]
async fn submitting_while_the_worker_runs_does_not_start_a_second_one() {
    let sink = sink(StubTransport::new_error(), store());

    sink.submit("my-post", &envelope(), ROOT).unwrap();
    let started_at = sink
        .inner
        .worker
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .last_swept;
    sink.submit("my-post", &envelope(), ROOT).unwrap();

    // A second spawn must not have replaced the running worker's heartbeat with a fresh one.
    let worker = sink.inner.worker.lock().unwrap();
    assert_eq!(worker.as_ref().unwrap().last_swept, started_at);
}

#[tokio::test]
async fn health_is_ok_while_the_worker_is_idle() {
    let sink = sink(StubTransport::new_ok(), store());

    assert!(sink.health().is_ok());
}

#[tokio::test]
async fn health_reports_a_stalled_worker() {
    let sink = sink(StubTransport::new_ok(), store());
    *sink.inner.worker.lock().unwrap() = Some(SweepHeartbeat {
        interval: Duration::from_secs(TEST_SWEEP_INTERVAL_SECS),
        last_swept: Instant::now() - Duration::from_secs(3600),
    });

    assert!(sink.health().is_err());
}

#[tokio::test]
async fn health_reports_an_unreachable_store() {
    let sink = sink(StubTransport::new_ok(), Arc::new(FailingStore));

    assert!(sink.health().is_err());
}
