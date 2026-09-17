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

impl Thread {
    pub fn resolve(slug: &str, mut messages: Vec<Message>) -> Result<Thread, Error> {
        messages.sort_by_key(|m| m.sent_at.unwrap_or(i64::MAX));

        let root_index = messages
            .iter()
            .position(|m| m.is_top_level() && m.subject == slug)
            .ok_or(Error::NoRoot)?;
        let root_message = messages.remove(root_index);

        let mut root_node = attach_replies(root_message, &mut messages);

        // Attach stray messages missing threading headers by subject instead.
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
