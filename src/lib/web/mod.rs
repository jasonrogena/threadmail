use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Redirect};
use axum::routing::get;
use axum::{Form, Router};
use lettre::address::Envelope;
use serde::Deserialize;
use tokio::sync::{Semaphore, SemaphorePermit};

use crate::config::Config;
use crate::mail::{Author, Message, Subject};
use crate::source::{BoxError, MailSink, MailSource};
use crate::store::{IncomingCommentStore, OutgoingComment, OutgoingCommentStore};
use crate::{compose, render, thread};

#[cfg(test)]
mod tests;

const PERMIT_TIMEOUT: Duration = Duration::from_secs(10);
const DEDUPE_WINDOW: Duration = Duration::from_secs(60);
const OUTGOING_SWEEP_HEARTBEAT_GRACE: u32 = 3;

// Mutated in place after the first tick; the Mutex is what makes that safe.
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
        self.last_swept.elapsed() > self.interval * OUTGOING_SWEEP_HEARTBEAT_GRACE
    }
}

#[derive(Clone)]
pub struct AppState {
    source: Arc<dyn MailSource>,
    sink: Arc<dyn MailSink>,
    cache: Arc<dyn IncomingCommentStore>,
    outgoing_comments: Arc<dyn OutgoingCommentStore>,
    config: Arc<Config>,
    search_limit: Arc<Semaphore>,
    submit_limit: Arc<Semaphore>,
    recent_submissions: Arc<Mutex<HashMap<u64, Instant>>>,
    refreshing: Arc<Mutex<HashSet<String>>>,
    sending: Arc<Mutex<HashSet<i64>>>,
    // None until the worker's first tick; lets /healthz tell "never started" from "stalled".
    last_outgoing_sweep: Arc<Mutex<Option<SweepHeartbeat>>>,
}

impl AppState {
    pub fn new(
        source: Arc<dyn MailSource>,
        sink: Arc<dyn MailSink>,
        cache: Arc<dyn IncomingCommentStore>,
        outgoing_comments: Arc<dyn OutgoingCommentStore>,
        config: Config,
    ) -> Self {
        let search_limit = Arc::new(Semaphore::new(config.imap.max_concurrent_searches));
        let submit_limit = Arc::new(Semaphore::new(config.smtp.max_concurrent_submits));
        Self {
            source,
            sink,
            cache,
            outgoing_comments,
            config: Arc::new(config),
            search_limit,
            submit_limit,
            recent_submissions: Arc::new(Mutex::new(HashMap::new())),
            refreshing: Arc::new(Mutex::new(HashSet::new())),
            sending: Arc::new(Mutex::new(HashSet::new())),
            last_outgoing_sweep: Arc::new(Mutex::new(None)),
        }
    }

    fn render_options<'a>(&'a self, comment_action: &'a str, stale: bool) -> render::Options<'a> {
        render::Options {
            comment_action,
            mailing_list: &self.config.mailing_list,
            web: &self.config.web,
            stale,
        }
    }
}

pub fn router(state: AppState) -> Router {
    // Wildcard: a slug can be a full post path (e.g. "posts/my-post").
    Router::new()
        .route("/thread/{*slug}", get(show_thread).post(submit_comment))
        .route("/healthz", get(healthz))
        .with_state(state)
}

// Skips IMAP/SMTP: those are already rate-limited to avoid hammering the mail provider.
async fn healthz(State(state): State<AppState>) -> impl IntoResponse {
    if let Err(err) = state.outgoing_comments.pending() {
        tracing::error!(%err, "healthz: could not reach the comment store");
        return (StatusCode::SERVICE_UNAVAILABLE, "comment store unreachable");
    }

    match state.last_outgoing_sweep.lock().unwrap().as_ref() {
        None => (
            StatusCode::SERVICE_UNAVAILABLE,
            "outgoing comment worker not running",
        ),
        Some(heartbeat) if heartbeat.is_stalled() => {
            tracing::error!(
                since_last_sweep = ?heartbeat.last_swept.elapsed(),
                "healthz: outgoing comment worker looks stalled"
            );
            (
                StatusCode::SERVICE_UNAVAILABLE,
                "outgoing comment worker stalled",
            )
        }
        Some(_) => (StatusCode::OK, "ok"),
    }
}

// The first tick also catches up on anything still pending from before a restart.
pub fn spawn_outgoing_comment_worker(state: AppState) {
    let interval = Duration::from_secs(state.config.storage.outgoing_comment_sweep_interval_secs);
    tokio::spawn(async move {
        loop {
            match state.outgoing_comments.pending() {
                Ok(pending) => {
                    for message in pending {
                        spawn_send(&state, message);
                    }
                }
                Err(err) => tracing::error!(%err, "failed to read the outgoing comment queue"),
            }
            {
                let mut heartbeat = state.last_outgoing_sweep.lock().unwrap();
                match heartbeat.as_mut() {
                    Some(heartbeat) => heartbeat.record_sweep(),
                    None => *heartbeat = Some(SweepHeartbeat::now(interval)),
                }
            }
            tokio::time::sleep(interval).await;
        }
    });
}

fn submission_key(slug: &str, in_reply_to: Option<&str>, name: &str, body: &str) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    (slug, in_reply_to, name, body).hash(&mut hasher);
    hasher.finish()
}

// Guards against a re-click before a slow request returns; there's no JS to disable the button.
fn is_duplicate_submission(recent: &Mutex<HashMap<u64, Instant>>, key: u64) -> bool {
    let mut recent = recent.lock().unwrap();
    let now = Instant::now();
    recent.retain(|_, seen_at| now.duration_since(*seen_at) < DEDUPE_WINDOW);
    recent.insert(key, now).is_some()
}

async fn acquire(semaphore: &Semaphore) -> Result<SemaphorePermit<'_>, StatusCode> {
    tokio::time::timeout(PERMIT_TIMEOUT, semaphore.acquire())
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)
        .map(|permit| permit.expect("semaphore is never closed"))
}

// Returns immediately; the caller renders from cache while this runs.
fn refresh_in_background(state: &AppState, slug: &str) {
    let newly_started = state.refreshing.lock().unwrap().insert(slug.to_string());
    if newly_started {
        tokio::spawn(run_refresh(state.clone(), slug.to_string()));
    }
}

async fn run_refresh(state: AppState, slug: String) {
    let permit = match acquire(&state.search_limit).await {
        Ok(permit) => permit,
        Err(_) => {
            state.refreshing.lock().unwrap().remove(&slug);
            return;
        }
    };
    let source = state.source.clone();
    let search_subject = Subject {
        slug: &slug,
        prefix: &state.config.mailing_list.subject_prefix,
        suffix: &state.config.mailing_list.subject_suffix,
    }
    .to_string();
    let result = tokio::task::spawn_blocking(move || source.search_subject(&search_subject)).await;
    drop(permit);

    match result {
        Ok(Ok(raw)) => {
            if let Err(err) = state.cache.store(&slug, &raw) {
                tracing::error!(%err, %slug, "failed to write the comment cache");
            }
        }
        Ok(Err(err)) => {
            tracing::error!(%err, %slug, "failed to search the mailbox for a thread");
            let _ = state.cache.mark_refresh_attempted(&slug);
        }
        Err(err) => {
            tracing::error!(%err, %slug, "the blocking IMAP search task panicked");
            let _ = state.cache.mark_refresh_attempted(&slug);
        }
    }

    state.refreshing.lock().unwrap().remove(&slug);
}

// Returns immediately; skips if a delivery attempt for this comment is already in flight.
fn spawn_send(state: &AppState, pending: OutgoingComment) {
    let newly_started = state.sending.lock().unwrap().insert(pending.id);
    if newly_started {
        tokio::spawn(attempt_send(state.clone(), pending));
    }
}

fn build_envelope(from_address: &str, to_address: &str) -> Result<Envelope, BoxError> {
    let from: lettre::Address = from_address.parse()?;
    let to: lettre::Address = to_address.parse()?;
    Ok(Envelope::new(Some(from), vec![to])?)
}

async fn attempt_send(state: AppState, pending: OutgoingComment) {
    if pending.is_expired(Duration::from_secs(
        state.config.storage.outgoing_message_ttl_secs,
    )) {
        tracing::warn!(
            id = pending.id,
            slug = %pending.slug,
            "dropping a queued comment that was never delivered within its TTL"
        );
        let _ = state.outgoing_comments.remove(pending.id);
        state.sending.lock().unwrap().remove(&pending.id);
        return;
    }

    let permit = match acquire(&state.submit_limit).await {
        Ok(permit) => permit,
        Err(_) => {
            state.sending.lock().unwrap().remove(&pending.id);
            return;
        }
    };

    let envelope = match build_envelope(&pending.from_address, &pending.to_address) {
        Ok(envelope) => envelope,
        Err(err) => {
            tracing::error!(%err, id = pending.id, "dropping a queued comment with an unusable envelope");
            let _ = state.outgoing_comments.remove(pending.id);
            state.sending.lock().unwrap().remove(&pending.id);
            return;
        }
    };

    let sink = state.sink.clone();
    let raw = pending.raw.clone();
    let result = tokio::task::spawn_blocking(move || sink.submit(&envelope, &raw)).await;
    drop(permit);

    match result {
        Ok(Ok(())) => {
            let _ = state.outgoing_comments.remove(pending.id);
            refresh_in_background(&state, &pending.slug);
        }
        Ok(Err(err)) => {
            tracing::warn!(%err, id = pending.id, slug = %pending.slug, "failed to submit a queued comment; will retry");
        }
        Err(err) => {
            tracing::error!(%err, id = pending.id, "the blocking SMTP submit task panicked");
        }
    }

    state.sending.lock().unwrap().remove(&pending.id);
}

async fn show_thread(State(state): State<AppState>, Path(slug): Path<String>) -> impl IntoResponse {
    let action = format!("/thread/{slug}");
    let subject = Subject {
        slug: &slug,
        prefix: &state.config.mailing_list.subject_prefix,
        suffix: &state.config.mailing_list.subject_suffix,
    };

    let cached = state.cache.get(&slug).unwrap_or_else(|err| {
        tracing::error!(%err, %slug, "failed to read the comment cache");
        crate::store::IncomingComment::empty()
    });

    let stale = cached.is_stale(Duration::from_secs(
        state.config.storage.incoming_message_ttl_secs,
    ));
    if stale {
        refresh_in_background(&state, &slug);
    }

    let options = state.render_options(&action, stale);

    let messages: Vec<_> = cached
        .raw_messages
        .iter()
        .filter_map(|bytes| {
            Message::parse(bytes, state.config.mailing_list.body_footer_regex.as_ref()).ok()
        })
        .collect();

    match thread::Thread::resolve(&subject, messages) {
        Ok(resolved) => Html(resolved.render(&options)).into_response(),
        Err(_) => Html(render::empty(&slug, &options)).into_response(),
    }
}

#[derive(Debug, Deserialize)]
pub struct CommentForm {
    pub name: String,
    pub body: String,
    pub in_reply_to: Option<String>,
}

async fn submit_comment(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    Form(form): Form<CommentForm>,
) -> impl IntoResponse {
    if !state.config.web.relay_comments {
        return (
            StatusCode::FORBIDDEN,
            "commenting via the web form is disabled on this site; reply by email instead",
        )
            .into_response();
    }

    let in_reply_to = form.in_reply_to.as_deref().filter(|s| !s.is_empty());
    let key = submission_key(&slug, in_reply_to, &form.name, &form.body);
    if is_duplicate_submission(&state.recent_submissions, key) {
        return Redirect::to(&format!("/thread/{slug}")).into_response();
    }

    // Invalidated now, not after relay: the cache is wrong the moment we commit to sending this.
    let _ = state.cache.invalidate(&slug);

    let author = Author {
        display_name: form.name,
    };
    let comment = compose::NewComment {
        author: &author,
        body: &form.body,
        in_reply_to,
    };
    let subject = Subject {
        slug: &slug,
        prefix: &state.config.mailing_list.subject_prefix,
        suffix: &state.config.mailing_list.subject_suffix,
    };

    let message = match comment.compose(
        &subject,
        &state.config.mailing_list.bot_address,
        &state.config.mailing_list.posting_address,
    ) {
        Ok(message) => message,
        Err(err) => {
            tracing::warn!(%err, %slug, "rejected a malformed comment submission");
            return (StatusCode::BAD_REQUEST, "could not compose that comment").into_response();
        }
    };

    let pending = match state.outgoing_comments.enqueue(
        &slug,
        &state.config.mailing_list.bot_address,
        &state.config.mailing_list.posting_address,
        &message.formatted(),
    ) {
        Ok(pending) => pending,
        Err(err) => {
            tracing::error!(%err, %slug, "failed to queue a comment for delivery");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                "could not queue that comment",
            )
                .into_response();
        }
    };

    spawn_send(&state, pending);

    Redirect::to(&format!("/thread/{slug}")).into_response()
}
