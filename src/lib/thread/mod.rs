//! Builds a reply tree for one post's slug out of a flat set of messages.
//!
//! This is resolved fresh per request (see the crate-level docs): there is
//! no persisted index, so this module only ever operates on whatever the
//! caller already fetched for one slug.

use crate::mail::Message;

#[cfg(test)]
mod tests;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("no message in the given set is a valid root for this slug")]
    NoRoot,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Node {
    pub message: Message,
    pub replies: Vec<Node>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Thread {
    pub slug: String,
    pub root: Node,
}

/// Resolves a thread for `slug` out of `messages`, which the caller is
/// expected to have already fetched via `IMAP SEARCH SUBJECT <slug>` (or an
/// equivalent on another backend). The root is the earliest top-level
/// message whose subject is exactly `slug`; everything else is attached by
/// walking `In-Reply-To`/`References`, falling back to "subject contains the
/// slug" for stray messages a client failed to thread correctly.
pub fn resolve(slug: &str, mut messages: Vec<Message>) -> Result<Thread, Error> {
    messages.sort_by_key(|m| m.sent_at.unwrap_or(i64::MAX));

    let root_index = messages
        .iter()
        .position(|m| m.is_top_level() && m.subject == slug)
        .ok_or(Error::NoRoot)?;
    let root_message = messages.remove(root_index);

    let mut root_node = attach_replies(root_message, &mut messages);

    // Fallback for messages a mail client failed to thread correctly (no
    // References/In-Reply-To pointing anywhere in this set): anything left
    // whose subject still carries the slug is attached directly under the
    // root rather than silently dropped.
    let mut i = 0;
    while i < messages.len() {
        if messages[i].subject.contains(slug) {
            let stray = messages.remove(i);
            root_node.replies.push(attach_replies(stray, &mut messages));
        } else {
            i += 1;
        }
    }

    Ok(Thread {
        slug: slug.to_string(),
        root: root_node,
    })
}

fn attach_replies(message: Message, remaining: &mut Vec<Message>) -> Node {
    let mut replies = Vec::new();
    let mut i = 0;
    while i < remaining.len() {
        if replies_to(&remaining[i], &message) {
            let child = remaining.remove(i);
            replies.push(child);
        } else {
            i += 1;
        }
    }

    let replies = replies
        .into_iter()
        .map(|child| attach_replies(child, remaining))
        .collect();

    Node { message, replies }
}

fn replies_to(candidate: &Message, parent: &Message) -> bool {
    candidate.in_reply_to.as_deref() == Some(parent.message_id.as_str())
        || candidate.references.iter().any(|r| r == &parent.message_id)
}
