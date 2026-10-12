use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::Semaphore;

use crate::config::Config;
use crate::mail::Subject;
use crate::source::{MailSink, MailSource};
use crate::store::IncomingCommentStore;

pub mod compose;
pub mod config;
pub mod imap_source;
pub mod mail;
pub mod render;
pub mod smtp_sink;
pub mod source;
pub mod sqlite_store;
pub mod store;
pub mod thread;
pub mod web;

const PERMIT_TIMEOUT: Duration = Duration::from_secs(10);

// Cloning is one Arc bump; every clone shares the same state.
#[derive(Clone)]
pub struct AppState {
    inner: Arc<Inner>,
}

struct Inner {
    source: Arc<dyn MailSource>,
    sink: Arc<dyn MailSink>,
    cache: Arc<dyn IncomingCommentStore>,
    config: Config,
    search_limit: Semaphore,
    refreshing: Mutex<HashSet<String>>,
}

impl AppState {
    pub fn new(
        source: Arc<dyn MailSource>,
        sink: Arc<dyn MailSink>,
        cache: Arc<dyn IncomingCommentStore>,
        config: Config,
    ) -> Self {
        let search_limit = Semaphore::new(config.imap.max_concurrent_searches);
        Self {
            inner: Arc::new(Inner {
                source,
                sink,
                cache,
                config,
                search_limit,
                refreshing: Mutex::new(HashSet::new()),
            }),
        }
    }

    pub(crate) fn sink(&self) -> &dyn MailSink {
        self.inner.sink.as_ref()
    }

    pub(crate) fn cache(&self) -> &dyn IncomingCommentStore {
        self.inner.cache.as_ref()
    }

    pub(crate) fn config(&self) -> &Config {
        &self.inner.config
    }

    fn source(&self) -> &dyn MailSource {
        self.inner.source.as_ref()
    }

    fn search_limit(&self) -> &Semaphore {
        &self.inner.search_limit
    }

    fn refreshing(&self) -> &Mutex<HashSet<String>> {
        &self.inner.refreshing
    }

    // Must run inside the async runtime; a delivered comment re-searches its thread so the reply shows up soon.
    pub fn boot(&self) {
        // Weak, since the sink holds this hook and a strong AppState would keep both alive forever.
        let app = Arc::downgrade(&self.inner);
        self.sink().boot(Arc::new(move |slug| {
            if let Some(inner) = app.upgrade() {
                AppState { inner }.refresh_in_background(slug);
            }
        }));
    }

    // Returns immediately; the caller renders from cache while this runs.
    pub(crate) fn refresh_in_background(&self, slug: &str) {
        let newly_started = self.refreshing().lock().unwrap().insert(slug.to_string());
        if newly_started {
            tokio::spawn(self.clone().refresh(slug.to_string()));
        }
    }

    async fn refresh(self, slug: String) {
        let permit = match tokio::time::timeout(PERMIT_TIMEOUT, self.search_limit().acquire()).await
        {
            Ok(permit) => permit.expect("semaphore is never closed"),
            Err(_) => {
                self.refreshing().lock().unwrap().remove(&slug);
                return;
            }
        };
        let search_subject = Subject {
            slug: &slug,
            prefix: &self.config().mailing_list.subject_prefix,
            suffix: &self.config().mailing_list.subject_suffix,
        }
        .to_string();
        let app = self.clone();
        let result =
            tokio::task::spawn_blocking(move || app.source().search_subject(&search_subject)).await;
        drop(permit);

        match result {
            Ok(Ok(raw)) => {
                if let Err(err) = self.cache().store(&slug, &raw) {
                    tracing::error!(%err, %slug, "failed to write the comment cache");
                }
            }
            Ok(Err(err)) => {
                tracing::error!(%err, %slug, "failed to search the mailbox for a thread");
                let _ = self.cache().mark_refresh_attempted(&slug);
            }
            Err(err) => {
                tracing::error!(%err, %slug, "the blocking IMAP search task panicked");
                let _ = self.cache().mark_refresh_attempted(&slug);
            }
        }

        self.refreshing().lock().unwrap().remove(&slug);
    }
}
