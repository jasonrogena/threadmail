use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Redirect};
use axum::routing::get;
use axum::{Form, Router};
use regex::Regex;
use serde::Deserialize;
use tokio::sync::{Semaphore, SemaphorePermit};

use crate::cache::CommentCache;
use crate::mail::{Author, Message, Subject};
use crate::source::{MailSink, MailSource};
use crate::{compose, render, thread};

#[cfg(test)]
mod tests;

const PERMIT_TIMEOUT: Duration = Duration::from_secs(10);
const DEDUPE_WINDOW: Duration = Duration::from_secs(60);

#[derive(Clone)]
pub struct AppState {
    source: Arc<dyn MailSource>,
    sink: Arc<dyn MailSink>,
    cache: Arc<dyn CommentCache>,
    cache_ttl: Duration,
    refresh_interval_secs: u64,
    bot_address: String,
    list_posting_address: String,
    relay_comments: bool,
    show_email_link: bool,
    theme: String,
    body_footer_regex: Option<Regex>,
    subject_prefix: String,
    subject_suffix: String,
    search_limit: Arc<Semaphore>,
    submit_limit: Arc<Semaphore>,
    recent_submissions: Arc<Mutex<HashMap<u64, Instant>>>,
    refreshing: Arc<Mutex<HashSet<String>>>,
}

impl AppState {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        source: Arc<dyn MailSource>,
        sink: Arc<dyn MailSink>,
        cache: Arc<dyn CommentCache>,
        cache_ttl_secs: u64,
        refresh_interval_secs: u64,
        bot_address: String,
        list_posting_address: String,
        relay_comments: bool,
        show_email_link: bool,
        theme: String,
        body_footer_regex: Option<Regex>,
        subject_prefix: String,
        subject_suffix: String,
        max_concurrent_searches: usize,
        max_concurrent_submits: usize,
    ) -> Self {
        Self {
            source,
            sink,
            cache,
            cache_ttl: Duration::from_secs(cache_ttl_secs),
            refresh_interval_secs,
            bot_address,
            list_posting_address,
            relay_comments,
            show_email_link,
            theme,
            body_footer_regex,
            subject_prefix,
            subject_suffix,
            search_limit: Arc::new(Semaphore::new(max_concurrent_searches)),
            submit_limit: Arc::new(Semaphore::new(max_concurrent_submits)),
            recent_submissions: Arc::new(Mutex::new(HashMap::new())),
            refreshing: Arc::new(Mutex::new(HashSet::new())),
        }
    }

    fn render_options<'a>(&'a self, comment_action: &'a str, stale: bool) -> render::Options<'a> {
        render::Options {
            comment_action,
            mailto_address: &self.list_posting_address,
            allow_relay: self.relay_comments,
            show_email_link: self.show_email_link,
            theme: &self.theme,
            refresh_interval_secs: self.refresh_interval_secs,
            stale,
            subject_prefix: &self.subject_prefix,
            subject_suffix: &self.subject_suffix,
        }
    }
}

pub fn router(state: AppState) -> Router {
    // Wildcard: a slug can be a full post path (e.g. "posts/my-post").
    Router::new()
        .route("/thread/{*slug}", get(show_thread).post(submit_comment))
        .with_state(state)
}

fn submission_key(slug: &str, in_reply_to: Option<&str>, name: &str, body: &str) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    (slug, in_reply_to, name, body).hash(&mut hasher);
    hasher.finish()
}

// Guards against a slow request being re-clicked before it returns; there's
// no JS here to disable the button after the first click.
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

// Kicks off a background IMAP search for `slug` if one isn't already
// running, and returns immediately so the caller can render from cache.
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
        prefix: &state.subject_prefix,
        suffix: &state.subject_suffix,
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

async fn show_thread(State(state): State<AppState>, Path(slug): Path<String>) -> impl IntoResponse {
    let action = format!("/thread/{slug}");
    let subject = Subject {
        slug: &slug,
        prefix: &state.subject_prefix,
        suffix: &state.subject_suffix,
    };

    let cached = state.cache.get(&slug).unwrap_or_else(|err| {
        tracing::error!(%err, %slug, "failed to read the comment cache");
        crate::cache::CacheEntry::empty()
    });

    let stale = cached.is_stale(state.cache_ttl);
    if stale {
        refresh_in_background(&state, &slug);
    }

    let options = state.render_options(&action, stale);

    let messages: Vec<_> = cached
        .raw_messages
        .iter()
        .filter_map(|bytes| Message::parse(bytes, state.body_footer_regex.as_ref()).ok())
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
    if !state.relay_comments {
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

    // Invalidated as soon as the write is accepted, not after it's relayed:
    // the cache is wrong the moment we've committed to sending this comment.
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
        prefix: &state.subject_prefix,
        suffix: &state.subject_suffix,
    };

    let message = match comment.compose(&subject, &state.bot_address, &state.list_posting_address) {
        Ok(message) => message,
        Err(err) => {
            tracing::warn!(%err, %slug, "rejected a malformed comment submission");
            return (StatusCode::BAD_REQUEST, "could not compose that comment").into_response();
        }
    };

    let result = {
        let _permit = match acquire(&state.submit_limit).await {
            Ok(permit) => permit,
            Err(status) => return (status, "too busy, please try again").into_response(),
        };
        let sink = state.sink.clone();
        tokio::task::spawn_blocking(move || sink.submit(&message)).await
    };

    match result {
        Ok(Ok(())) => {
            refresh_in_background(&state, &slug);
            Redirect::to(&format!("/thread/{slug}")).into_response()
        }
        Ok(Err(err)) => {
            tracing::error!(%err, %slug, "failed to submit a comment to the mailing list");
            (
                StatusCode::BAD_GATEWAY,
                "could not submit that comment, please try again",
            )
                .into_response()
        }
        Err(err) => {
            tracing::error!(%err, %slug, "the blocking SMTP submit task panicked");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "could not submit that comment",
            )
                .into_response()
        }
    }
}
