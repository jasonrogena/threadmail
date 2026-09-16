//! Thin HTTP layer: two routes, no business logic of their own. Everything
//! here does is parse the request, call into the ports/core, and translate
//! the result into a response.

use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Redirect};
use axum::routing::{get, post};
use axum::{Form, Router};
use serde::Deserialize;
use tokio::sync::{Semaphore, SemaphorePermit};

use crate::source::{MailSink, MailSource};
use crate::{compose, render, thread};

#[cfg(test)]
mod tests;

/// How long a request will wait for a permit before giving up. Without this,
/// the cap on *outbound* connections is real, but a large enough pileup of
/// incoming requests could still exhaust the server's own resources waiting
/// on a permit that never frees up.
const PERMIT_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone)]
pub struct AppState {
    source: Arc<dyn MailSource>,
    sink: Arc<dyn MailSink>,
    bot_address: String,
    list_posting_address: String,
    relay_comments: bool,
    show_email_link: bool,
    search_limit: Arc<Semaphore>,
    submit_limit: Arc<Semaphore>,
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
            search_limit: Arc::new(Semaphore::new(max_concurrent_searches)),
            submit_limit: Arc::new(Semaphore::new(max_concurrent_submits)),
        }
    }

    fn render_options<'a>(&'a self, comment_action: &'a str) -> render::Options<'a> {
        render::Options {
            comment_action,
            mailto_address: &self.list_posting_address,
            allow_relay: self.relay_comments,
            show_email_link: self.show_email_link,
        }
    }
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/thread/{slug}", get(show_thread))
        .route("/thread/{slug}/comment", post(submit_comment))
        .with_state(state)
}

/// Waits for a permit, giving up after `PERMIT_TIMEOUT` rather than queuing
/// forever. The semaphore is never closed, so the only way `acquire` itself
/// fails is a bug in this module.
async fn acquire(semaphore: &Semaphore) -> Result<SemaphorePermit<'_>, StatusCode> {
    tokio::time::timeout(PERMIT_TIMEOUT, semaphore.acquire())
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)
        .map(|permit| permit.expect("semaphore is never closed"))
}

async fn show_thread(State(state): State<AppState>, Path(slug): Path<String>) -> impl IntoResponse {
    let action = format!("/thread/{slug}/comment");
    let options = state.render_options(&action);

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
        .filter_map(|bytes| crate::mail::parse(&bytes).ok())
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

    let comment = compose::NewComment {
        name: &form.name,
        body: &form.body,
        in_reply_to: form.in_reply_to.as_deref().filter(|s| !s.is_empty()),
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
        Ok(Ok(())) => Redirect::to(&format!("/thread/{slug}")).into_response(),
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
