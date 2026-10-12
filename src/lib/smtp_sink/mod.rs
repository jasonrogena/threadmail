use std::collections::HashSet;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use lettre::address::Envelope;
use lettre::transport::smtp::authentication::Credentials;
use lettre::{SmtpTransport, Transport};
use tokio::sync::Semaphore;

use crate::config::{SmtpConfig, StorageConfig};
use crate::source::{BoxError, MailSink, OnDelivered};
use crate::store::{OutgoingComment, OutgoingCommentStore};

#[cfg(test)]
mod tests;

const PERMIT_TIMEOUT: Duration = Duration::from_secs(10);
const SWEEP_HEARTBEAT_GRACE: u32 = 3;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("could not build the SMTP transport")]
    Build(#[from] lettre::transport::smtp::Error),
    #[error(transparent)]
    Config(#[from] crate::config::Error),
    #[error("an outgoing comment needs exactly one sender and one recipient")]
    UnsupportedEnvelope,
    #[error("outgoing comment store unreachable")]
    StoreUnreachable(#[source] BoxError),
    #[error("outgoing comment worker stalled")]
    WorkerStalled,
}

// Mutated in place after the first sweep; the Mutex around it is what makes that safe.
struct SweepHeartbeat {
    interval: Duration,
    last_swept: Instant,
}

impl SweepHeartbeat {
    fn now(interval: Duration) -> Self {
        Self {
            interval,
            last_swept: Instant::now(),
        }
    }

    fn record_sweep(&mut self) {
        self.last_swept = Instant::now();
    }

    fn is_stalled(&self) -> bool {
        self.last_swept.elapsed() > self.interval * SWEEP_HEARTBEAT_GRACE
    }
}

// SMTP blocks on the provider, so submit queues durably and the outgoing comment worker delivers until it succeeds or the comment expires.
pub struct SmtpSink<T = SmtpTransport> {
    inner: Arc<Inner<T>>,
}

struct Inner<T> {
    transport: T,
    outgoing_comments: Arc<dyn OutgoingCommentStore>,
    outgoing_message_ttl: Duration,
    sweep_interval: Duration,
    delivery_limit: Semaphore,
    delivering: Mutex<HashSet<i64>>,
    // Some only while the worker runs; doubles as the lock that keeps it to a single instance.
    worker: Mutex<Option<SweepHeartbeat>>,
    on_delivered: OnceLock<OnDelivered>,
}

impl SmtpSink {
    pub fn new(
        smtp: &SmtpConfig,
        storage: &StorageConfig,
        outgoing_comments: Arc<dyn OutgoingCommentStore>,
    ) -> Result<Self, Error> {
        let transport = SmtpTransport::starttls_relay(&smtp.host)?
            .port(smtp.port)
            .credentials(Credentials::new(smtp.username()?, smtp.password()?))
            .build();
        Ok(Self::with_transport(
            transport,
            smtp,
            storage,
            outgoing_comments,
        ))
    }
}

impl<T> SmtpSink<T>
where
    T: Transport + Send + Sync + 'static,
    T::Error: std::error::Error + Send + Sync + 'static,
{
    pub fn with_transport(
        transport: T,
        smtp: &SmtpConfig,
        storage: &StorageConfig,
        outgoing_comments: Arc<dyn OutgoingCommentStore>,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                transport,
                outgoing_comments,
                outgoing_message_ttl: Duration::from_secs(storage.outgoing_message_ttl_secs),
                sweep_interval: Duration::from_secs(storage.outgoing_comment_sweep_interval_secs),
                delivery_limit: Semaphore::new(smtp.max_concurrent_deliveries),
                delivering: Mutex::new(HashSet::new()),
                worker: Mutex::new(None),
                on_delivered: OnceLock::new(),
            }),
        }
    }
}

impl<T> MailSink for SmtpSink<T>
where
    T: Transport + Send + Sync + 'static,
    T::Error: std::error::Error + Send + Sync + 'static,
{
    // The first sweep delivers whatever was still queued before a restart.
    fn boot(&self, on_delivered: OnDelivered) {
        let _ = self.inner.on_delivered.set(on_delivered);
        self.inner.clone().spawn_worker();
    }

    fn submit(&self, slug: &str, envelope: &Envelope, raw: &[u8]) -> Result<(), BoxError> {
        let (Some(from), [to]) = (envelope.from(), envelope.to()) else {
            return Err(Error::UnsupportedEnvelope.into());
        };
        self.inner
            .outgoing_comments
            .enqueue(slug, from.as_ref(), to.as_ref(), raw)?;
        self.inner.clone().spawn_worker();
        Ok(())
    }

    fn health(&self) -> Result<(), BoxError> {
        self.inner
            .outgoing_comments
            .pending()
            .map_err(Error::StoreUnreachable)?;
        match self.inner.worker.lock().unwrap().as_ref() {
            Some(heartbeat) if heartbeat.is_stalled() => {
                tracing::error!(
                    since_last_sweep = ?heartbeat.last_swept.elapsed(),
                    "the outgoing comment worker looks stalled"
                );
                Err(Error::WorkerStalled.into())
            }
            _ => Ok(()),
        }
    }
}

impl<T> Inner<T>
where
    T: Transport + Send + Sync + 'static,
    T::Error: std::error::Error + Send + Sync + 'static,
{
    // A no-op if the worker is already running; it exits once nothing deliverable is left queued.
    fn spawn_worker(self: Arc<Self>) {
        {
            let mut worker = self.worker.lock().unwrap();
            if worker.is_some() {
                return;
            }
            *worker = Some(SweepHeartbeat::now(self.sweep_interval));
        }
        tokio::spawn(async move {
            loop {
                {
                    let mut worker = self.worker.lock().unwrap();
                    // Checked under the lock so a submit racing this exit sees None and spawns a fresh worker.
                    match self.outgoing_comments.pending() {
                        Ok(pending) if pending.is_empty() => {
                            *worker = None;
                            return;
                        }
                        Ok(pending) => {
                            for message in pending {
                                self.clone().spawn_delivery(message);
                            }
                        }
                        Err(err) => {
                            tracing::error!(%err, "failed to read the outgoing comment queue")
                        }
                    }
                    if let Some(heartbeat) = worker.as_mut() {
                        heartbeat.record_sweep();
                    }
                }
                tokio::time::sleep(self.sweep_interval).await;
            }
        });
    }

    // Returns immediately; skips if a delivery attempt for this comment is already in flight.
    fn spawn_delivery(self: Arc<Self>, pending: OutgoingComment) {
        let newly_started = self.delivering.lock().unwrap().insert(pending.id);
        if newly_started {
            tokio::spawn(self.attempt_delivery(pending));
        }
    }

    async fn attempt_delivery(self: Arc<Self>, pending: OutgoingComment) {
        if pending.is_expired(self.outgoing_message_ttl) {
            tracing::warn!(
                id = pending.id,
                slug = %pending.slug,
                "dropping an outgoing comment that was never delivered within its TTL"
            );
            let _ = self.outgoing_comments.remove(pending.id);
            self.delivering.lock().unwrap().remove(&pending.id);
            return;
        }

        let permit = match tokio::time::timeout(PERMIT_TIMEOUT, self.delivery_limit.acquire()).await
        {
            Ok(permit) => permit.expect("semaphore is never closed"),
            Err(_) => {
                self.delivering.lock().unwrap().remove(&pending.id);
                return;
            }
        };

        let envelope = match pending.envelope() {
            Ok(envelope) => envelope,
            Err(err) => {
                tracing::error!(%err, id = pending.id, "dropping an outgoing comment with an unusable envelope");
                let _ = self.outgoing_comments.remove(pending.id);
                self.delivering.lock().unwrap().remove(&pending.id);
                return;
            }
        };

        let inner = self.clone();
        let raw = pending.raw.clone();
        let result = tokio::task::spawn_blocking(move || {
            inner.transport.send_raw(&envelope, &raw).map(|_| ())
        })
        .await;
        drop(permit);

        match result {
            Ok(Ok(())) => {
                let _ = self.outgoing_comments.remove(pending.id);
                if let Some(on_delivered) = self.on_delivered.get() {
                    on_delivered(&pending.slug);
                }
            }
            Ok(Err(err)) => {
                tracing::warn!(%err, id = pending.id, slug = %pending.slug, "failed to deliver an outgoing comment; will retry");
            }
            Err(err) => {
                tracing::error!(%err, id = pending.id, "the blocking SMTP delivery task panicked");
            }
        }

        self.delivering.lock().unwrap().remove(&pending.id);
    }
}
