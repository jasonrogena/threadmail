use askama::Template;

use crate::config::{MailingListConfig, WebConfig};
use crate::mail::{Author, Subject};
use crate::thread::{Node, Thread};

#[cfg(test)]
mod tests;

pub struct Options<'a> {
    pub comment_action: &'a str,
    pub mailing_list: &'a MailingListConfig,
    pub web: &'a WebConfig,
    pub stale: bool,
}

#[derive(Template)]
#[template(path = "style.html")]
struct StyleTemplate<'a> {
    theme: &'a str,
}

#[derive(Template)]
#[template(path = "thread.html")]
struct ThreadTemplate<'a> {
    slug: &'a str,
    style_html: String,
    top_level_html: String,
    form_html: String,
    mailto_address: &'a str,
    mailto_subject: String,
    show_email_link: bool,
    allow_relay: bool,
    refresh_interval_secs: u64,
    stale: bool,
}

#[derive(Template)]
#[template(path = "empty.html")]
struct EmptyTemplate<'a> {
    slug: &'a str,
    style_html: String,
    form_html: String,
    mailto_address: &'a str,
    mailto_subject: String,
    show_email_link: bool,
    allow_relay: bool,
    refresh_interval_secs: u64,
    stale: bool,
}

#[derive(Template)]
#[template(path = "comment.html")]
struct CommentTemplate<'a> {
    id: &'a str,
    author: &'a Author,
    body_html: String,
    form_html: String,
    replies_html: Vec<String>,
}

#[derive(Template)]
#[template(path = "comment_form.html")]
struct CommentFormTemplate<'a> {
    action: &'a str,
    in_reply_to: &'a str,
    label: &'a str,
}

fn style(theme: &str) -> String {
    StyleTemplate { theme }
        .render()
        .expect("style template is valid")
}

impl Thread {
    pub fn render(&self, options: &Options) -> String {
        let top_level_html = self
            .top_level_messages
            .iter()
            .map(|node| node.render(options))
            .collect::<Vec<_>>()
            .join("\n");
        let form_html = if options.web.relay_comments {
            comment_form(options.comment_action, "", "Leave a comment")
        } else {
            String::new()
        };
        ThreadTemplate {
            slug: &self.slug,
            style_html: style(options.web.theme.as_str()),
            top_level_html,
            form_html,
            mailto_address: &options.mailing_list.posting_address,
            mailto_subject: Subject {
                slug: &self.slug,
                prefix: &options.mailing_list.subject_prefix,
                suffix: &options.mailing_list.subject_suffix,
            }
            .to_string(),
            show_email_link: options.web.show_email_link,
            allow_relay: options.web.relay_comments,
            refresh_interval_secs: options.web.refresh_interval_secs,
            stale: options.stale,
        }
        .render()
        .expect("thread template is valid")
    }
}

pub fn empty(slug: &str, options: &Options) -> String {
    let form_html = if options.web.relay_comments {
        comment_form(options.comment_action, "", "Leave a comment")
    } else {
        String::new()
    };
    EmptyTemplate {
        slug,
        style_html: style(options.web.theme.as_str()),
        form_html,
        mailto_address: &options.mailing_list.posting_address,
        mailto_subject: Subject {
            slug,
            prefix: &options.mailing_list.subject_prefix,
            suffix: &options.mailing_list.subject_suffix,
        }
        .to_string(),
        show_email_link: options.web.show_email_link,
        allow_relay: options.web.relay_comments,
        refresh_interval_secs: options.web.refresh_interval_secs,
        stale: options.stale,
    }
    .render()
    .expect("empty template is valid")
}

impl Node {
    fn render(&self, options: &Options) -> String {
        let form_html = if options.web.relay_comments {
            comment_form(options.comment_action, &self.message.message_id, "Reply")
        } else {
            String::new()
        };
        let replies_html = self.replies.iter().map(|r| r.render(options)).collect();

        CommentTemplate {
            id: &self.message.message_id,
            author: &self.message.author,
            body_html: escape_paragraphs(&self.message.body),
            form_html,
            replies_html,
        }
        .render()
        .expect("comment template is valid")
    }
}

fn comment_form(action: &str, in_reply_to: &str, label: &str) -> String {
    CommentFormTemplate {
        action,
        in_reply_to,
        label,
    }
    .render()
    .expect("comment form template is valid")
}

fn escape(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn escape_paragraphs(input: &str) -> String {
    input
        .split("\n\n")
        .map(|p| format!("<p>{}</p>", escape(p).replace('\n', "<br>")))
        .collect::<Vec<_>>()
        .join("\n")
}
