use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use lettre::address::Envelope;
use lettre::transport::smtp::authentication::Credentials;
use lettre::transport::smtp::client::{
    Certificate, CertificateStore, Tls, TlsParameters, TlsParametersBuilder,
};
use lettre::{SmtpTransport, Transport};
use tokio::sync::{Notify, Semaphore};
use tokio::task::JoinSet;

use crate::config::{SmtpConfig, SmtpTlsMode, StorageConfig};
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
    #[error("could not read the configured CA certificate file")]
    CaFile(#[from] std::io::Error),
}

// Mutated in place after the first sweep; the Mutex around it is what makes that safe.
struct SweepHeartbeat {
    max_gap: Duration,
    last_swept: Instant,
}

impl SweepHeartbeat {
    // A sweep awaits its deliveries, so the allowed gap covers one slow delivery on top of the retry sleep.
    fn now(sweep_interval: Duration, delivery_timeout: Duration) -> Self {
        Self {
            max_gap: sweep_interval * SWEEP_HEARTBEAT_GRACE + PERMIT_TIMEOUT + delivery_timeout,
            last_swept: Instant::now(),
        }
    }

    fn record_sweep(&mut self) {
        self.last_swept = Instant::now();
    }

    fn is_stalled(&self) -> bool {
        self.last_swept.elapsed() > self.max_gap
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
    delivery_timeout: Duration,
    delivery_limit: Semaphore,
    // Some only while the worker runs; doubles as the lock that keeps it to a single instance.
    worker: Mutex<Option<SweepHeartbeat>>,
    wake: Notify,
    on_delivered: OnceLock<OnDelivered>,
}

impl SmtpSink {
    pub fn new(
        smtp: &SmtpConfig,
        storage: &StorageConfig,
        outgoing_comments: Arc<dyn OutgoingCommentStore>,
    ) -> Result<Self, Error> {
        let tls_parameters = match &smtp.ca_file {
            Some(path) => {
                let pem = std::fs::read(path)?;
                TlsParametersBuilder::new(smtp.host.clone())
                    .certificate_store(CertificateStore::None)
                    .add_root_certificate(Certificate::from_pem(&pem)?)
                    .build()?
            }
            None => TlsParameters::new(smtp.host.clone())?,
        };
        let tls = match smtp.tls_mode {
            SmtpTlsMode::Starttls => Tls::Required(tls_parameters),
            SmtpTlsMode::Implicit => Tls::Wrapper(tls_parameters),
        };
        let transport = SmtpTransport::builder_dangerous(&smtp.host)
            .port(smtp.port)
            .tls(tls)
            .credentials(Credentials::new(smtp.username()?, smtp.password()?))
            .timeout(Some(Duration::from_secs(smtp.delivery_timeout_secs)))
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
                delivery_timeout: Duration::from_secs(smtp.delivery_timeout_secs),
                delivery_limit: Semaphore::new(smtp.max_concurrent_deliveries),
                worker: Mutex::new(None),
                wake: Notify::new(),
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
    fn spawn_worker(self: Arc<Self>) {
        {
            let mut worker = self.worker.lock().unwrap();
            if worker.is_some() {
                // Stores a permit if the worker isn't sleeping yet, so its next sleep returns at once.
                self.wake.notify_one();
                return;
            }
            *worker = Some(SweepHeartbeat::now(
                self.sweep_interval,
                self.delivery_timeout,
            ));
        }
        tokio::spawn(async move {
            loop {
                let pending = {
                    let mut worker = self.worker.lock().unwrap();
                    // Checked under the lock so a submit racing this exit sees None and spawns a fresh worker.
                    let pending = match self.outgoing_comments.pending() {
                        Ok(pending) if pending.is_empty() => {
                            *worker = None;
                            return;
                        }
                        Ok(pending) => Some(pending),
                        Err(err) => {
                            tracing::error!(%err, "failed to read the outgoing comment queue");
                            None
                        }
                    };
                    if let Some(heartbeat) = worker.as_mut() {
                        heartbeat.record_sweep();
                    }
                    pending
                };
                let all_left_queue = match pending {
                    Some(pending) => self.clone().deliver_all(pending).await,
                    None => false,
                };
                if !all_left_queue {
                    tokio::select! {
                        _ = tokio::time::sleep(self.sweep_interval) => {}
                        _ = self.wake.notified() => {}
                    }
                }
            }
        });
    }

    async fn deliver_all(self: Arc<Self>, pending: Vec<OutgoingComment>) -> bool {
        let mut deliveries = JoinSet::new();
        for comment in pending {
            deliveries.spawn(self.clone().attempt_delivery(comment));
        }
        let mut all_left_queue = true;
        while let Some(left_queue) = deliveries.join_next().await {
            // A panicked delivery leaves its comment queued.
            all_left_queue &= left_queue.unwrap_or(false);
        }
        all_left_queue
    }

    // Returns whether the comment left the queue, delivered or dropped; false means retry on a later sweep.
    async fn attempt_delivery(self: Arc<Self>, pending: OutgoingComment) -> bool {
        if pending.is_expired(self.outgoing_message_ttl) {
            tracing::warn!(
                id = pending.id,
                slug = %pending.slug,
                "dropping an outgoing comment that was never delivered within its TTL"
            );
            return self.remove(pending.id);
        }

        let permit = match tokio::time::timeout(PERMIT_TIMEOUT, self.delivery_limit.acquire()).await
        {
            Ok(permit) => permit.expect("semaphore is never closed"),
            Err(_) => return false,
        };

        let envelope = match pending.envelope() {
            Ok(envelope) => envelope,
            Err(err) => {
                tracing::error!(%err, id = pending.id, "dropping an outgoing comment with an unusable envelope");
                return self.remove(pending.id);
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
                let removed = self.remove(pending.id);
                if let Some(on_delivered) = self.on_delivered.get() {
                    on_delivered(&pending.slug);
                }
                removed
            }
            Ok(Err(err)) => {
                tracing::warn!(%err, id = pending.id, slug = %pending.slug, "failed to deliver an outgoing comment; will retry");
                false
            }
            Err(err) => {
                tracing::error!(%err, id = pending.id, "the blocking SMTP delivery task panicked");
                false
            }
        }
    }

    // A comment that can't be removed stays queued, so a delivered one would be delivered again.
    fn remove(&self, id: i64) -> bool {
        match self.outgoing_comments.remove(id) {
            Ok(()) => true,
            Err(err) => {
                tracing::error!(%err, id, "failed to remove an outgoing comment from the queue");
                false
            }
        }
    }
}
