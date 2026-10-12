use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Redirect};
use axum::routing::get;
use axum::{Form, Router};
use serde::Deserialize;

use crate::AppState;
use crate::mail::{Author, Message, Subject};
use crate::{compose, render, thread};

#[cfg(test)]
mod tests;

const DEDUPE_WINDOW: Duration = Duration::from_secs(60);

#[derive(Clone)]
pub struct WebState {
    app: AppState,
    recent_submissions: Arc<Mutex<HashMap<u64, Instant>>>,
}

impl WebState {
    pub fn new(app: AppState) -> Self {
        Self {
            app,
            recent_submissions: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn router(self) -> Router {
        // Wildcard: a slug can be a full post path (e.g. "posts/my-post").
        Router::new()
            .route(
                "/thread/{*slug}",
                get(Self::handle_get_thread).post(Self::handle_post_thread),
            )
            .route("/healthz", get(Self::handle_get_healthz))
            .with_state(self)
    }

    fn render_options<'a>(&'a self, comment_action: &'a str, stale: bool) -> render::Options<'a> {
        render::Options {
            comment_action,
            mailing_list: &self.app.config().mailing_list,
            web: &self.app.config().web,
            stale,
        }
    }

    // Guards against a re-click before a slow request returns; there's no JS to disable the button.
    fn is_duplicate_submission(&self, key: u64) -> bool {
        let mut recent = self.recent_submissions.lock().unwrap();
        let now = Instant::now();
        recent.retain(|_, seen_at| now.duration_since(*seen_at) < DEDUPE_WINDOW);
        recent.insert(key, now).is_some()
    }

    // Skips IMAP/SMTP: those are already rate-limited to avoid hammering the mail provider.
    async fn handle_get_healthz(State(state): State<Self>) -> impl IntoResponse {
        match state.app.sink().health() {
            Ok(()) => (StatusCode::OK, "ok".to_string()),
            Err(err) => {
                tracing::error!(%err, "healthz: the mail sink is unhealthy");
                (StatusCode::SERVICE_UNAVAILABLE, err.to_string())
            }
        }
    }

    async fn handle_get_thread(
        State(state): State<Self>,
        Path(slug): Path<String>,
    ) -> impl IntoResponse {
        let action = format!("/thread/{slug}");
        let subject = Subject {
            slug: &slug,
            prefix: &state.app.config().mailing_list.subject_prefix,
            suffix: &state.app.config().mailing_list.subject_suffix,
        };

        let cached = state.app.cache().get(&slug).unwrap_or_else(|err| {
            tracing::error!(%err, %slug, "failed to read the comment cache");
            crate::store::IncomingComment::empty()
        });

        let stale = cached.is_stale(Duration::from_secs(
            state.app.config().storage.incoming_message_ttl_secs,
        ));
        if stale {
            state.app.refresh_in_background(&slug);
        }

        let options = state.render_options(&action, stale);

        let messages: Vec<_> = cached
            .raw_messages
            .iter()
            .filter_map(|bytes| {
                Message::parse(
                    bytes,
                    state.app.config().mailing_list.body_footer_regex.as_ref(),
                )
                .ok()
            })
            .collect();

        match thread::Thread::resolve(&subject, messages) {
            Ok(resolved) => Html(resolved.render(&options)).into_response(),
            Err(_) => Html(render::empty(&slug, &options)).into_response(),
        }
    }

    async fn handle_post_thread(
        State(state): State<Self>,
        Path(slug): Path<String>,
        Form(form): Form<CommentForm>,
    ) -> impl IntoResponse {
        if !state.app.config().web.relay_comments {
            return (
                StatusCode::FORBIDDEN,
                "commenting via the web form is disabled on this site; reply by email instead",
            )
                .into_response();
        }

        if state.is_duplicate_submission(form.submission_key(&slug)) {
            return Redirect::to(&format!("/thread/{slug}")).into_response();
        }

        // Invalidated now, not after relay: the cache is wrong the moment we commit to sending this.
        let _ = state.app.cache().invalidate(&slug);

        let author = Author {
            display_name: form.name.clone(),
        };
        let comment = compose::NewComment {
            author: &author,
            body: &form.body,
            in_reply_to: form.in_reply_to(),
        };
        let subject = Subject {
            slug: &slug,
            prefix: &state.app.config().mailing_list.subject_prefix,
            suffix: &state.app.config().mailing_list.subject_suffix,
        };

        let message = match comment.compose(
            &subject,
            &state.app.config().mailing_list.bot_address,
            &state.app.config().mailing_list.posting_address,
        ) {
            Ok(message) => message,
            Err(err) => {
                tracing::warn!(%err, %slug, "rejected a malformed comment submission");
                return (StatusCode::BAD_REQUEST, "could not compose that comment").into_response();
            }
        };

        let submitted = match state.app.config().mailing_list.envelope() {
            Ok(envelope) => state
                .app
                .sink()
                .submit(&slug, &envelope, &message.formatted()),
            Err(err) => Err(err.into()),
        };
        if let Err(err) = submitted {
            tracing::error!(%err, %slug, "failed to submit a comment to the mail sink");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                "could not queue that comment",
            )
                .into_response();
        }

        Redirect::to(&format!("/thread/{slug}")).into_response()
    }
}

#[derive(Debug, Deserialize)]
pub struct CommentForm {
    pub name: String,
    pub body: String,
    pub in_reply_to: Option<String>,
}

impl CommentForm {
    // The form always posts the field, empty for a top-level comment.
    fn in_reply_to(&self) -> Option<&str> {
        self.in_reply_to.as_deref().filter(|s| !s.is_empty())
    }

    fn submission_key(&self, slug: &str) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        (slug, self.in_reply_to(), &self.name, &self.body).hash(&mut hasher);
        hasher.finish()
    }
}
