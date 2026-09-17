use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Redirect};
use axum::routing::get;
use axum::{Form, Router};
use regex::Regex;
use serde::Deserialize;
use tokio::sync::{Semaphore, SemaphorePermit};

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
    bot_address: String,
    list_posting_address: String,
    relay_comments: bool,
    show_email_link: bool,
    theme: String,
    body_footer_regex: Option<Regex>,
    search_limit: Arc<Semaphore>,
    submit_limit: Arc<Semaphore>,
    recent_submissions: Arc<Mutex<HashMap<u64, Instant>>>,
}

impl AppState {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        source: Arc<dyn MailSource>,
        sink: Arc<dyn MailSink>,
        bot_address: String,
        list_posting_address: String,
        relay_comments: bool,
        show_email_link: bool,
        theme: String,
        body_footer_regex: Option<Regex>,
        max_concurrent_searches: usize,
        max_concurrent_submits: usize,
    ) -> Self {
        Self {
            source,
            sink,
            bot_address,
            list_posting_address,
            relay_comments,
            show_email_link,
            theme,
            body_footer_regex,
            search_limit: Arc::new(Semaphore::new(max_concurrent_searches)),
            submit_limit: Arc::new(Semaphore::new(max_concurrent_submits)),
            recent_submissions: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    fn render_options<'a>(
        &'a self,
        comment_action: &'a str,
        just_posted: bool,
    ) -> render::Options<'a> {
        render::Options {
            comment_action,
            mailto_address: &self.list_posting_address,
            allow_relay: self.relay_comments,
            show_email_link: self.show_email_link,
            theme: &self.theme,
            just_posted,
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

#[derive(Debug, Deserialize)]
pub struct ShowThreadQuery {
    posted: Option<String>,
}

async fn show_thread(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    Query(query): Query<ShowThreadQuery>,
) -> impl IntoResponse {
    let action = format!("/thread/{slug}");
    let options = state.render_options(&action, query.posted.is_some());

    let raw = {
        let _permit = match acquire(&state.search_limit).await {
            Ok(permit) => permit,
            Err(status) => return (status, "too busy, please try again").into_response(),
        };
        let source = state.source.clone();
        let search_slug = slug.clone();
        tokio::task::spawn_blocking(move || source.search_subject(&search_slug)).await
    };

    let raw = match raw {
        Ok(Ok(raw)) => raw,
        Ok(Err(err)) => {
            tracing::error!(%err, %slug, "failed to search the mailbox for a thread");
            return Html(render::empty(&slug, &options)).into_response();
        }
        Err(err) => {
            tracing::error!(%err, %slug, "the blocking IMAP search task panicked");
            return Html(render::empty(&slug, &options)).into_response();
        }
    };

    let messages: Vec<_> = raw
        .into_iter()
        .filter_map(|bytes| crate::mail::parse(&bytes, state.body_footer_regex.as_ref()).ok())
        .collect();

    match thread::resolve(&slug, messages) {
        Ok(resolved) => Html(render::render(&resolved, &options)).into_response(),
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
        return Redirect::to(&format!("/thread/{slug}?posted=1")).into_response();
    }

    let comment = compose::NewComment {
        name: &form.name,
        body: &form.body,
        in_reply_to,
    };

    let message = match compose::compose(
        &slug,
        &state.bot_address,
        &state.list_posting_address,
        &comment,
    ) {
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
        Ok(Ok(())) => Redirect::to(&format!("/thread/{slug}?posted=1")).into_response(),
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
