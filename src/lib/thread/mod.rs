use crate::mail::{Message, Subject};

#[cfg(test)]
mod tests;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("no message in the given set is a valid top-level message for this slug")]
    NoTopLevelMessage,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Node {
    pub message: Message,
    pub replies: Vec<Node>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Thread {
    pub slug: String,
    pub top_level_messages: Vec<Node>,
}

impl Thread {
    pub fn resolve(subject: &Subject, mut messages: Vec<Message>) -> Result<Thread, Error> {
        messages.sort_by_key(|m| m.sent_at.unwrap_or(i64::MAX));

        // At least one message must carry the exact canonical subject, or this
        // isn't confidently a thread for this slug at all.
        let exact_subject = subject.to_string();
        let has_exact_match = messages
            .iter()
            .any(|m| m.is_top_level() && m.subject == exact_subject);
        if !has_exact_match {
            return Err(Error::NoTopLevelMessage);
        }

        // Every non-reply message matching the subject is a top-level message
        // with equal standing, in the order they were sent.
        let mut top_level_messages = Vec::new();
        let mut i = 0;
        while i < messages.len() {
            if messages[i].is_top_level() && messages[i].subject.contains(subject.slug) {
                let message = messages.remove(i);
                top_level_messages.push(attach_replies(message, &mut messages));
            } else {
                i += 1;
            }
        }

        Ok(Thread {
            slug: subject.slug.to_string(),
            top_level_messages,
        })
    }
}

fn attach_replies(message: Message, remaining: &mut Vec<Message>) -> Node {
    let mut replies = Vec::new();
    let mut i = 0;
    while i < remaining.len() {
        if remaining[i].replies_to(&message) {
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
